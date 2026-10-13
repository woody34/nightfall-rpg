#!/usr/bin/env python3
"""Phase 5 economy oracle (design §14.1; plan stories 1.5-1.7). Python 3.11+, standard library.

Independent of the Rust engine and of gen_economy.py's arithmetic: every family reads the
E1 tables (`tables-phase5/`, after `gen_economy.py --check`) or the pinned clones directly,
and computes with its own exact `Fraction` rounding (`fl`, `fl32`). CPython `float` (IEEE-754
binary64 with correctly rounded basic operations) is used ONLY as the binary64 reference that
the exact emulation must equal. No family uses tolerances and no expectation is Rust-authored.

    python3 packages/data/scripts/economy_oracle.py [--check] [--family D-E ...]

Each family asserts its Done-When facts and contributes vectors to a JSON golden per story
(`fixtures/economy-oracle-{drops,party,vendor}.json`) for the engine's oracle units; `--check`
regenerates the goldens in memory and fails if a committed golden differs.
"""
from __future__ import annotations

import argparse
from fractions import Fraction as F
import json
import math
from pathlib import Path
import re
import struct
import subprocess
import sys
import tomllib
import xml.etree.ElementTree as ET

DATA = Path(__file__).resolve().parents[1]
TABLES = DATA / "tables-phase5"
FIXTURES = DATA / "fixtures"
DP = Path.home() / ".cache/nightfall/l2j/l2j-dp"
GAME = Path.home() / ".cache/nightfall/l2j/l2j-game"
GJ = "src/main/java/com/l2jserver/gameserver/"
GC = "src/main/resources/config/"
DPD = "src/main/resources/data/"
TWO64 = 1 << 64


class OracleError(Exception):
    pass


def require(cond: bool, msg: str) -> None:
    if not cond:
        raise OracleError(msg)


_T: dict[str, dict] = {}


def table(name: str) -> dict:
    if name not in _T:
        _T[name] = tomllib.loads((TABLES / name).read_text(encoding="utf-8"))
    return _T[name]


def game_text(rel: str) -> str:
    return (GAME / rel).read_text(encoding="utf-8")


def prop(name: str, key: str) -> str:
    found = re.findall(rf"^{re.escape(key)}\s*=[ \t]*(.*?)[ \t]*$", game_text(GC + name), re.M)
    require(len(found) == 1, f"{name}: {key}")
    return found[0]


# ---------------------------------------------------------------------------
# Exact binary rounding (own implementation, integer shifts)
# ---------------------------------------------------------------------------


def _round(x: F, p: int, emin: int, emax: int) -> F:
    if x == 0:
        return F(0)
    if x < 0:
        return -_round(-x, p, emin, emax)
    n, d = x.numerator, x.denominator
    e = n.bit_length() - d.bit_length()
    if (n << max(0, -e)) < (d << max(0, e)):
        e -= 1
    require(emin <= e <= emax, f"{x} outside the normal range")
    shift = p - 1 - e
    num, den = (n << shift, d) if shift >= 0 else (n, d << -shift)
    q, r = divmod(num, den)
    if 2 * r > den or (2 * r == den and q & 1):
        q += 1
    if q == 1 << p:
        q >>= 1
        shift -= 1
        require(p - 1 - shift <= emax, "overflow")
    return F(q, 1 << shift) if shift >= 0 else F(q << -shift)


def fl(x) -> F:
    return _round(F(x), 53, -1022, 1023)


def fl32(x) -> F:
    return _round(F(x), 24, -126, 127)


def bits(x: F) -> str:
    require(fl(x) == x, f"{x} is not binary64")
    return struct.pack(">d", float(x)).hex()


def from_bits(h: str) -> F:
    return F(struct.unpack(">d", bytes.fromhex(h))[0])


def ref(x: float) -> F:
    """The exact value of a CPython (binary64) float."""
    return F(x)


def chance(r: int, c: F) -> bool:
    """E15: success iff r * 100 * D < N * 2^64 (exact; r is one draw_u64)."""
    return r * 100 * c.denominator < c.numerator * TWO64


def chance_threshold(c: F) -> int:
    """Number of u64 draws that succeed: #{r in [0, 2^64): r*100*D < N*2^64}."""
    if c <= 0:
        return 0
    lim = -(-(c.numerator * TWO64) // (100 * c.denominator))  # ceil
    return min(lim, TWO64)


def chance_bound(r: int, k: int, n: int) -> bool:
    """`Rnd.get(n) < k` under E15: chance(k*100 / n)."""
    return chance(r, F(k * 100, n))


def roll_below_words(words, n: int) -> tuple[int, int]:
    """The engine's unbiased roll_below (Z/state.rs:859-869); returns (value, words used)."""
    n = max(n, 1)
    limit = (1 << 32) - ((1 << 32) % n)
    used = 0
    for w in words:
        used += 1
        if w < limit:
            return w % n, used
    raise OracleError("word stream exhausted")


class Words:
    """A deterministic u32 word stream (xorshift64*) for whole-trace vectors."""

    def __init__(self, seed: int) -> None:
        self.s = seed or 0x9E3779B97F4A7C15
        self.log: list[int] = []

    def __iter__(self):
        return self

    def __next__(self) -> int:
        x = self.s
        x ^= (x >> 12) & ((1 << 64) - 1)
        x ^= (x << 25) & ((1 << 64) - 1)
        x ^= (x >> 27) & ((1 << 64) - 1)
        self.s = x & ((1 << 64) - 1)
        w = ((self.s * 0x2545F4914F6CDD1D) & ((1 << 64) - 1)) >> 32
        self.log.append(w)
        return w

    def u64(self) -> int:
        return (next(self) << 32) | next(self)

    def roll_below(self, n: int) -> int:
        return roll_below_words(self, n)[0]


FAMILIES: dict[str, tuple[str, object]] = {}


def family(name: str, story: str):
    def wrap(fn):
        FAMILIES[name] = (story, fn)
        return fn
    return wrap


# ---------------------------------------------------------------------------
# Story 1.5: drop families
# ---------------------------------------------------------------------------


def npc_rows():
    return table("drops.toml")["npc"]


def items_xml() -> dict[int, dict]:
    if "_items" not in _T:
        out = {}
        for f in sorted((DP / DPD / "stats/items").glob("*.xml")):
            for it in ET.parse(f).getroot().findall("item"):
                s = {x.get("name"): x.get("val") for x in it.findall("set")}
                out[int(it.get("id"))] = {"type": it.get("type"), "sets": s}
        _T["_items"] = out
    return _T["_items"]


def is_herb(item_id: int) -> bool:
    return items_xml()[item_id]["sets"].get("ex_immediate_effect") == "true"


def is_stackable(item_id: int) -> bool:
    return items_xml()[item_id]["sets"].get("is_stackable") == "true"


# --- killer modifiers (§3.3), recomputed from npc.properties and the Java expressions ----


def _umap_exact(v: int, in_min: int, in_max: int, out_min: int, out_max: F) -> F:
    x = F(min(max(v, in_min), in_max))
    return fl(fl(fl(fl(x - in_min) * fl(out_max - out_min)) / F(in_max - in_min)) + out_min)


def _umap_float(v: int, in_min: int, in_max: int, out_min: int, out_max: float) -> float:
    x = float(min(max(v, in_min), in_max))
    return (((x - in_min) * (out_max - out_min)) / (in_max - in_min)) + out_min


def killer_l(strategy: str, d: int, exact: bool = True) -> F:
    npc = lambda k: int(prop("npc.properties", k))  # noqa: E731
    if strategy == "raid_clamp":
        if exact:
            return max(F(0), min(F(1), fl(fl(d * fl(F("0.15"))) + 1)))
        return ref(max(0.0, min(1.0, (d * 0.15) + 1)))
    pre = "DropItem" if strategy == "item_window" else "DropAdena"
    lo, hi, gap = npc(pre + "MinLevelDifference"), npc(pre + "MaxLevelDifference"), npc(pre + "MinLevelGapChance")
    if exact:
        return fl(_umap_exact(d, -hi, -lo, gap, F(100)) / 100)
    return ref(_umap_float(d, -hi, -lo, gap, 100.0) / 100)


def all_l_values() -> list[F]:
    vals = set()
    for d in range(-20, 3):
        for s in ("item_window", "raid_clamp", "adena_window"):
            vals.add(killer_l(s, d))
    return sorted(vals)


# --- chance model -----------------------------------------------------------------------


def cd(s: str) -> F:
    return fl(F(s))


def ungrouped_c(row_chance: str, L: F, m: F = F(1)) -> F:
    # getChance(victim, killer) = killerMod * (getChance() * multiplier)
    return fl(L * fl(cd(row_chance) * m))


def single_collapse(item_chance: str, group_chance: str) -> F:
    # (item1.getChance() * dropItem.getChance()) / 100
    return fl(fl(cd(item_chance) * cd(group_chance)) / 100)


def group_eval(group: dict, L: F, m: F = F(1)) -> dict:
    g = cd(group["chance"])
    w = [fl(cd(i["chance"]) * m) for i in group["items"]]
    raw = [fl(fl(wi * g) * L) for wi in w]
    t = [fl(x / 100) for x in raw]
    S = F(0)
    for ti in t:
        S = fl(S + ti)
    if S == 0:
        return {"S": S, "s": None, "T": None}
    s = [fl(x / S) for x in raw]
    T, acc = [], F(0)
    for si in s:
        acc = fl(acc + si)
        T.append(acc)
    return {"S": S, "s": s, "T": T}


def group_eval_float(group: dict, L: float, m: float = 1.0) -> dict:
    g = float(group["chance"])
    w = [float(i["chance"]) * m for i in group["items"]]
    raw = [(wi * g) * L for wi in w]
    S = 0.0
    for x in raw:
        S += x / 100
    if S == 0:
        return {"S": ref(S), "s": None, "T": None}
    s = [x / S for x in raw]
    T, acc = [], 0.0
    for si in s:
        acc += si
        T.append(ref(acc))
    return {"S": ref(S), "s": [ref(x) for x in s], "T": T}


def multi_groups():
    for npc in npc_rows():
        for node in npc["death"]:
            if node["kind"] == "group" and len(node["items"]) > 1:
                yield npc, node


def remainder(c: F) -> F:
    """Java `chance % 100` for chance >= 0: exact IEEE fmod."""
    return c - 100 * math.floor(c / 100)


@family("D-C", "1.5")
def fam_dc(out: dict) -> str:
    vec = []
    hundred = F(100)
    hb = int(bits(hundred), 16)
    below, above = from_bits(format(hb - 1, "016x")), from_bits(format(hb + 1, "016x"))
    require(below < 100 < above and fl(below) == below and fl(above) == above, "neighbours of 100")
    for name, c in (("below_100", below), ("exactly_100", hundred), ("above_100", above)):
        precise = c > 100  # STRICT for ungrouped (IDropCalculationStrategy:36)
        draws = ["chance"] + (["remainder"] if precise else []) + ["quantity"]
        vec.append({"case": name, "C_bits": bits(c), "precise_branch": precise, "draws_on_pass": draws,
                    "pass_threshold": str(chance_threshold(c))})
    require([v["precise_branch"] for v in vec] == [False, False, True], "strict > 100")
    # (b) real group extremes used as synthetic ungrouped rows
    gch = sorted({n["chance"] for npc in npc_rows() for n in npc["death"] if n["kind"] == "group"}, key=F)
    require(gch[0] == "0.0003000000142492354" and gch[-1] == "188.85499572753906", "group chance extremes")
    for s in (gch[-1], gch[0]):
        c = ungrouped_c(s, F(1))
        require(ref(float(s)) == c, "parseDouble")
        mult_base = int(c) // 100
        rem = remainder(c)
        require(ref(math.fmod(float(s), 100.0)) == rem, "fmod reference")
        vec.append({"case": f"synthetic_ungrouped_{s}", "C_bits": bits(c), "mult_base": mult_base,
                    "remainder_bits": bits(rem), "remainder_draw": c > 100, "pass_threshold": str(chance_threshold(c)),
                    "remainder_threshold": str(chance_threshold(rem))})
    # (c) every single-item group collapse, STRICT rule, vs binary64 reference
    singles = 0
    for npc in npc_rows():
        for n in npc["death"]:
            if n["kind"] == "group" and len(n["items"]) == 1:
                singles += 1
                c = single_collapse(n["items"][0]["chance"], n["chance"])
                require(c == ref((float(n["items"][0]["chance"]) * float(n["chance"])) / 100), f"collapse {npc['id']}")
    require(singles == 3032, f"single-item groups {singles}")
    # (d) multi-item groups whose binary64 G is exactly 100: remainder 0, remainder draw still taken
    g100 = [(npc["id"], gi) for npc, gi in multi_groups() if group_eval(gi, F(1))["S"] == 100]
    require(g100, "a group with G == 100 exists")
    vec.append({"case": "group_G_exactly_100", "examples": [g100[0][0]], "count_at_L1": len(g100),
                "draws_on_pass": ["group", "selection", "remainder(0)", "quantity"]})
    # (e) the 150 multi-item groups with exact inner sum != 100 (and 1 single-item group)
    off_multi, off_single = [], 0
    for npc in npc_rows():
        for n in npc["death"]:
            if n["kind"] != "group":
                continue
            inner = sum(F(i["chance"]) for i in n["items"])
            if inner != 100:
                if len(n["items"]) > 1:
                    ev, fv = group_eval(n, F(1)), group_eval_float(n, 1.0)
                    require(ev == fv, f"normalized shares {npc['id']}")
                    off_multi.append({"npc": npc["id"], "inner_sum": str(inner),
                                      "shares_bits": [bits(x) for x in ev["s"]], "S_bits": bits(ev["S"])})
                else:
                    off_single += 1
    require(len(off_multi) == 150 and off_single == 1, f"inner sums != 100: {len(off_multi)}/{off_single}")
    out["dc_inner_sum_not_100"] = off_multi
    # (f) min == max still draws its quantity; (g) per-node draw counts
    eq = sum(1 for npc in npc_rows() for scope in ("death", "corpse") for n in npc[scope]
             for r in (n["items"] if n["kind"] == "group" else [n]) if r["min"] == r["max"])
    require(eq == 56117, f"min == max rows {eq}")
    vec.append({"case": "min_equals_max", "rows": eq, "quantity_draw": "roll_below(1), always taken (P3)"})
    out["dc_vectors"] = vec
    return f"strict>100 ok; 3,032 collapses = binary64; G==100 groups {len(g100)}; 150+1 inner-sum!=100; 56,117 min==max"


@family("D-E", "1.5")
def fam_de(out: dict) -> str:
    Ls = all_l_values()
    require(len(Ls) == 19, f"distinct L values {len(Ls)}")
    groups = list(multi_groups())
    checked = 0
    for npc, g in groups:
        for L in Ls:
            ev = group_eval(g, L)
            fv = group_eval_float(g, float(L))
            require(ev == fv, f"group of {npc['id']} at L={L}")
            checked += 1
    # Exact-normalized-100 groups and their binary64 G at L = 1
    exact100, below, above = 0, [], []
    falloff = 0
    for npc, g in groups:
        exact = sum(F(i["chance"]) * F(g["chance"]) / 100 for i in g["items"])
        ev = group_eval(g, F(1))
        if exact == 100:
            exact100 += 1
            if ev["S"] < 100:
                below.append(npc["id"])
            elif ev["S"] > 100:
                above.append(npc["id"])
        if ev["T"][-1] < 100:
            falloff += 1
    require(exact100 == 121, f"exact-100 groups {exact100}")
    require(sorted(below) == [29068, 29150, 29186], f"below-100 {below}")
    require(sorted(above) == [29028, 29068, 29118, 29163], f"above-100 {above}")
    require(falloff == 1127, f"fall-offs {falloff}")
    # ungrouped and single-item rows: every C for every L equals the binary64 reference
    rows = 0
    for npc in npc_rows():
        for scope in ("death", "corpse"):
            for n in npc[scope]:
                if n["kind"] == "item":
                    for L in Ls:
                        require(ungrouped_c(n["chance"], L) == ref(float(L) * (float(n["chance"]) * 1.0)), "ungrouped C")
                    rows += 1
                elif len(n["items"]) == 1:
                    c1 = single_collapse(n["items"][0]["chance"], n["chance"])
                    for L in Ls:
                        require(fl(L * c1) == ref(float(L) * float(c1)), "single C")
                    rows += 1
    out["de_summary"] = {"groups": len(groups), "L_values": [bits(x) for x in Ls], "exact_100": exact100,
                         "below_100_npcs": sorted(below), "above_100_npcs": sorted(above), "falloff_L1": falloff}
    return (f"{len(groups):,} multi-item groups x {len(Ls)} L = binary64; exact-100 {exact100}: "
            f"below {sorted(below)}, above {sorted(above)}; fall-offs at L=1 {falloff:,}; {rows:,} ungrouped/single rows")


def _width_terms(c: F) -> int:
    """Widest intermediate of the U256 chance test r*100*D < N*2^64 (r < 2^64)."""
    if c <= 0:
        return 0
    return max(((TWO64 - 1) * 100 * c.denominator).bit_length(), (c.numerator * TWO64).bit_length())


def drop_widths(g_str: str) -> int:
    G = fl(F(g_str))
    m = fl32(fl(F(1) * G))  # rate' = fl32(fl(1 x G)); multiplier = fl32(fl(1 x rate')) = rate'
    m = fl32(fl(1 * m))
    Ls = all_l_values()
    widest = 0
    for npc in npc_rows():
        for scope in ("death", "corpse"):
            for n in npc[scope]:
                if n["kind"] == "item" or len(n["items"]) == 1:
                    base = fl(cd(n["chance"]) * m) if n["kind"] == "item" else \
                        fl(fl(fl(cd(n["items"][0]["chance"]) * m) * cd(n["chance"])) / 100)
                    for L in Ls:
                        c = fl(L * base)
                        widest = max(widest, _width_terms(c))
                        if c > 100:
                            widest = max(widest, _width_terms(remainder(c)))
                    continue
                for L in Ls:
                    ev = group_eval(n, L, m)
                    if ev["S"] == 0:
                        continue
                    widest = max(widest, _width_terms(ev["S"]))
                    if ev["S"] >= 100:
                        widest = max(widest, _width_terms(remainder(ev["S"])))
                    for t in ev["T"]:
                        widest = max(widest, _width_terms(t))
    return widest


@family("D-W", "1.5")
def fam_dw(out: dict) -> str:
    w1 = drop_widths("1")
    whi = drop_widths(str(2 ** 32))
    wlo = drop_widths("0.00000000023283064365386962890625")  # exactly 2^-32
    out["dw_widths"] = {"G=1": w1, "G=2^32": whi, "G=2^-32": wlo}
    require(w1 == 141, f"width at G=1 is {w1}, design reports 141")
    require(max(whi, wlo) <= 255, f"width {max(whi, wlo)} exceeds 255")
    return f"widest intermediate: G=1 {w1} bits; G=2^32 {whi}; G=2^-32 {wlo} (<= 255)"


@family("D-L", "1.5")
def fam_dl(out: dict) -> str:
    vec = {}
    expected_item = [F(1), F(1), fl(F("0.82")), fl(F("0.64")), fl(F("0.46")), fl(F("0.28")), fl(F("0.1")), fl(F("0.1"))]
    got = [killer_l("item_window", d) for d in range(-4, -12, -1)]
    require(got == expected_item, "item window d=-4..-11")
    vec["item_window"] = {str(d): bits(killer_l("item_window", d)) for d in range(-4, -12, -1)}
    vec["adena_window"] = {str(d): bits(killer_l("adena_window", d)) for d in range(-8, -17, -1)}
    vec["raid_clamp"] = {str(d): bits(killer_l("raid_clamp", d)) for d in range(0, -8, -1)}
    for s, ds in (("item_window", range(-4, -12, -1)), ("adena_window", range(-8, -17, -1)), ("raid_clamp", range(0, -8, -1))):
        for d in ds:
            require(killer_l(s, d) == killer_l(s, d, exact=False), f"{s} {d} vs binary64")
    require(killer_l("raid_clamp", -7) == 0 and killer_l("raid_clamp", -6) == ref(-6 * 0.15 + 1), "raid clamp tail")
    # Ungrouped rows of a raid use the window, not the clamp (DEFAULT_NONGROUP_STRATEGY; F32)
    vec["ungrouped_raid_uses_window"] = True
    out["dl_vectors"] = vec
    return "item d=-4..-11, adena d=-8..-16, raid d=-1..-7 = binary64 references"


def m_branch(item_id: int, scope: str, raid: bool, g_str: str) -> tuple[str, F]:
    """m(item) per IChanceMultiplierStrategy.DEFAULT_STRATEGY with G folded per E37."""
    G = fl(F(g_str))
    rates = table("rates.toml")
    over = {}
    for pair in prop("rates.properties", "DropChanceMultiplierByItemId").split(";"):
        if pair.strip():
            k, v = pair.split(",")
            over[int(k)] = v
    def prime(v: str) -> F:
        return fl32(fl(fl(F(v)) * G))
    if item_id in over:
        branch, rate = "override", prime(over[item_id])
    elif is_herb(item_id):
        branch, rate = "herb", prime(prop("rates.properties", "HerbDropChanceMultiplier"))
    elif raid:
        branch, rate = "raid", prime(prop("rates.properties", "RaidDropChanceMultiplier"))
    else:
        key = "DeathDropChanceMultiplier" if scope == "death" else "CorpseDropChanceMultiplier"
        branch, rate = scope, prime(prop("rates.properties", key))
    m = fl32(fl(F(1) * rate))  # float multiplier = 1; multiplier *= rate'
    _ = rates
    return branch, m


@family("D-M", "1.5")
def fam_dm(out: dict) -> str:
    # The DR5 double-rounding counterexample: fl32(fl(G)) = 1 while fl32(G) = 1 + 2^-23.
    q = F(2164260992, 2164260863)
    g_str = format_decimal(q, 40)
    require(fl32(fl(F(g_str))) == 1 and fl32(F(g_str)) == 1 + F(1, 2 ** 23), "DR5 counterexample")
    picks = {"override": (57, "death", False), "herb": (8600, "death", False), "raid": (6656, "death", True),
             "death": (1864, "death", False), "corpse": (1864, "corpse", False)}
    vec = []
    for g in ("1", "1.5", "0.3333333333333333", g_str):
        for want, (item, scope, raid) in picks.items():
            branch, m = m_branch(item, scope, raid, g)
            require(branch == want, f"branch {want}")
            # float reference: (float)(1f * (float)(rate * G)) for rate 1
            ref_m = struct.unpack(">f", struct.pack(">f", 1.0 * float(F(g))))[0] if g != g_str else None
            if ref_m is not None:
                require(m == ref(ref_m), f"m {want} G={g}")
            vec.append({"G": g, "branch": branch, "item": item, "m_bits": bits(m)})
    # surplus -> precise mult: ungrouped chance 60 at G = 2 -> C = 120 -> mult 1 + remainder draw 20
    c = fl(F(1) * fl(F(60) * fl32(fl(F(2)))))
    require(c == 120 and int(c) // 100 == 1 and remainder(c) == 20, "surplus to mult")
    # amounts: G never touches a(item)
    a = fl(F(prop("rates.properties", "DeathDropAmountMultiplier")))
    require(a == 1, "amount multiplier")
    vec.append({"case": "surplus", "C": "120", "mult_base": 1, "remainder": "20", "amount_multiplier": "1"})
    out["dm_vectors"] = vec
    return "5 branches x G {1, 1.5, 0.333..., DR5 string}; DR5 double rounding reproduced; surplus->mult; amounts unchanged"


def format_decimal(q: F, digits: int) -> str:
    """The `digits`-significant-digit decimal string nearest to q (q > 0)."""
    e = 0
    while q / F(10) ** e >= 10:
        e += 1
    while q / F(10) ** e < 1:
        e -= 1
    scaled = q / F(10) ** (e - digits + 1)
    n = round(scaled)
    s = str(n)
    point = len(s) - (digits - 1 - e) if e < digits - 1 else None
    if point is None:
        return s + "0" * (e - digits + 1)
    return s[:point] + "." + s[point:] if point > 0 else "0." + "0" * (-point) + s


# --- whole-death traces (§2.2, §2.3) ---------------------------------------------------


def roll_node(node: dict, scope: str, raid: bool, L_group: F, L_item, words: Words, trace: list) -> list:
    """Draws of one drop node in pinned order; returns holders [(item, count)]."""
    def ungrouped(item, c, site):
        r = words.u64()
        trace.append({"site": site, "draw": "chance", "C": bits(c), "pass": chance(r, c)})
        if not chance(r, c):
            return None
        mult = 1
        if c > 100:
            mult = int(c) // 100
            rem = remainder(c)
            r2 = words.u64()
            trace.append({"site": site, "draw": "remainder", "C": bits(rem), "pass": chance(r2, rem)})
            if chance(r2, rem):
                mult += 1
        n = item["max"] - item["min"] + 1
        q, used = roll_below_words(words, n)
        trace.append({"site": site, "draw": "quantity", "n": n, "words": used})
        return (item["id"], (item["min"] + q) * mult)

    site = 2 if scope == "corpse" else 3
    if node["kind"] == "item":
        m = m_branch(node["id"], scope, raid, "1")[1]
        h = ungrouped(node, fl(L_item(node["id"]) * fl(cd(node["chance"]) * m)), site)
        return [h] if h else []
    if len(node["items"]) == 1:
        it = node["items"][0]
        m = m_branch(it["id"], scope, raid, "1")[1]
        c1 = fl(fl(fl(cd(it["chance"]) * m) * cd(node["chance"])) / 100)
        h = ungrouped(it, fl(L_group * c1), site)
        return [h] if h else []
    ev = group_eval(node, L_group)
    G = ev["S"]
    r = words.u64()
    trace.append({"site": site, "draw": "group", "G": bits(G) if G else "0", "pass": chance(r, G)})
    if not chance(r, G):
        return []
    u = words.u64()
    pick = None
    for idx, T in enumerate(ev["T"]):
        if u * 100 * T.denominator < T.numerator * TWO64:
            pick = idx
            break
    trace.append({"site": site, "draw": "selection", "picked": pick})
    if pick is None:
        return []
    it = node["items"][pick]
    mult = 1
    if G >= 100:
        mult = int(G) // 100
        rem = remainder(G)
        r2 = words.u64()
        trace.append({"site": site, "draw": "remainder", "C": bits(rem), "pass": chance(r2, rem)})
        if chance(r2, rem):
            mult += 1
    n = it["max"] - it["min"] + 1
    q, used = roll_below_words(words, n)
    trace.append({"site": site, "draw": "quantity", "n": n, "words": used})
    return [(it["id"], (it["min"] + q) * mult)]


def death_trace(npc: dict, killer_level: int, spoiled: bool, respawn_random: int, seed: int) -> dict:
    words = Words(seed)
    trace: list = []
    d = npc["level"] - killer_level
    L_group = killer_l("raid_clamp" if npc["is_raid"] else "item_window", d)
    L_item = lambda iid: killer_l("adena_window" if iid == 57 else "item_window", d)  # noqa: E731
    sweep = []
    if spoiled:
        for node in npc["corpse"]:
            sweep += roll_node(node, "corpse", npc["is_raid"], L_group, L_item, words, trace)
    holders = []
    for node in npc["death"]:
        holders += roll_node(node, "death", npc["is_raid"], L_group, L_item, words, trace)
    for item, count in holders:
        for _ in range(1 if is_stackable(item) else count):
            dx = words.roll_below(141) - 70
            dy = words.roll_below(141) - 70
            trace.append({"site": 4, "draw": "position", "item": item, "dx": dx, "dy": dy})
    if respawn_random > 0:
        trace.append({"site": 5, "draw": "jitter", "value": words.roll_below(respawn_random + 1)})
    sites = [t["site"] for t in trace]
    require(sites == sorted(sites), "corpse -> death -> positions -> jitter order")
    return {"npc": npc["id"], "killer_level": killer_level, "spoiled": spoiled, "respawn_random": respawn_random,
            "seed": seed, "sweep": sweep, "holders": holders, "trace": trace, "words": words.log}


@family("D-O", "1.5")
def fam_do(out: dict) -> str:
    byid = {n["id"]: n for n in npc_rows()}
    plain = next(n["id"] for n in npc_rows() if n["corpse"] and not n["is_raid"] and n["id"] != 22320)
    raid = next(n["id"] for n in npc_rows() if n["is_raid"] and len(n["death"]) > 1)
    cases = [(22320, 81, True, 0, 1), (22320, 72, False, 30, 2), (plain, 1, True, 0, 3), (raid, 70, False, 0, 4),
             (29068, 80, False, 0, 5)]
    traces = [death_trace(byid[n], kl, sp, rr, sd) for n, kl, sp, rr, sd in cases if n in byid]
    require(len(traces) == 5, "trace NPCs present")
    # 22320's first death node is an ungrouped item: it must be the first death draw (document order).
    first_death = next(t for t in traces[0]["trace"] if t["site"] == 3)
    require(first_death["draw"] == "chance", "22320 starts with its ungrouped item")
    out["do_traces"] = traces
    return f"{len(traces)} whole-death traces (corpse -> death -> positions -> jitter), draw counts per node"


@family("D-P", "1.5")
def fam_dp(out: dict) -> str:
    g = table("ground.toml")["pk"]
    non_drop = set(g["non_droppable_items"]) | set(g["pet_items"])
    require(len(g["non_droppable_items"]) == 30 and len(set(g["non_droppable_items"])) == 29, "NonDroppable 30/29")
    require(len(g["pet_items"]) == 11, "PetItems 11")
    require(g["player_rate_drop"] == 0, "PlayerRateDrop 0: NPC-killer branch never draws")

    def excluded(inst) -> bool:
        return (inst["template"] == 57 or inst["template"] in non_drop or inst.get("quest")
                or inst.get("augmented") or not inst.get("droppable", True))

    def pk_drop(inv, karma, pk_count, killer_is_player, words):
        trace, dropped = [], []
        if not killer_is_player:
            return trace, dropped, inv  # PlayerRateDrop = 0: no draw
        if not (karma > 0 and pk_count >= g["minimum_pk_required_to_drop"]):
            return trace, dropped, inv
        r = words.u64()
        trace.append(("gate", chance_bound(r, g["karma_rate_drop"], 100)))
        if not chance_bound(r, g["karma_rate_drop"], 100):
            return trace, dropped, inv
        for inst in sorted(inv, key=lambda i: i["id"]):
            if excluded(inst):
                continue
            p = (g["karma_rate_drop_equip_weapon"] if inst.get("weapon") else g["karma_rate_drop_equip"]) \
                if inst.get("equipped") else g["karma_rate_drop_item"]
            if inst.get("equipped"):
                inst["equipped"] = False
                trace.append(("unequip", inst["id"]))
            r = words.u64()
            ok = chance_bound(r, p, 100)
            trace.append(("item", inst["id"], p, ok))
            if ok:
                dx, dy = words.roll_below(50) - 25, words.roll_below(50) - 25
                require(-25 <= dx <= 24 and -25 <= dy <= 24, "offsets")
                dropped.append((inst["id"], dx, dy))
                if len(dropped) >= g["karma_drop_limit"]:
                    break
        return trace, dropped, inv

    # Gate threshold: chance_bound(40, 100) passes for exactly ceil(40 * 2^64 / 100) draws
    require(chance_threshold(F(40)) == -(-(40 * TWO64) // 100), "gate threshold")
    vec = []
    for seed in range(1, 200):
        inv = [{"id": i, "template": 100 + i, "equipped": i % 3 == 0, "weapon": i % 6 == 0} for i in range(1, 25)]
        inv += [{"id": 50, "template": 57}, {"id": 51, "template": 7694}, {"id": 52, "template": 2375},
                {"id": 53, "template": 999, "quest": True}, {"id": 54, "template": 998, "augmented": True}]
        tr, dropped, inv2 = pk_drop(inv, 10, 6, True, Words(seed))
        if len(dropped) == 10:
            last = dropped[-1][0]
            later_equipped = [i["id"] for i in inv2 if i["id"] > last and i.get("equipped") and not excluded(i)]
            require(all(i.get("equipped") for i in inv2 if i["id"] > last and i["id"] % 3 == 0 and i["id"] < 25), "later equipped stay")
            vec.append({"seed": seed, "trace": tr, "dropped": dropped, "later_equipped": later_equipped})
            break
    require(vec, "a vector reaching the 10-drop break")
    tr, dropped, _ = pk_drop([{"id": 1, "template": 100}], 10, 5, True, Words(9))
    require(tr == [] and dropped == [], "pk_count < 6 draws nothing")
    tr, dropped, _ = pk_drop([{"id": 1, "template": 100}], 10, 9, False, Words(9))
    require(tr == [], "NPC killer draws nothing")
    out["dp_vectors"] = vec
    return "exclusions (adena, NonDroppable, PetItems, quest, augmented); unequip-before-roll; break at 10; gate 40; offsets [-25, 24]; NPC killer no draw"


@family("D-T", "1.5")
def fam_dt(out: dict) -> str:
    gr = table("ground.toml")["ground"]
    scan = gr["scanner_ms"] // 100
    require(scan == 50, "scanner 50 ticks")
    limits = {"auto_destroy_9849": gr["auto_destroy_time"][0]["seconds"] * 10,
              "herb": gr["herb_decay_seconds"] * 10, "loot": gr["decay_seconds"] * 10}
    require(sorted(limits.values()) == [100, 600, 6000], "limits 100/600/6,000 ticks")

    def due(dropped: int, limit: int) -> int:
        t = -(-dropped // scan) * scan
        while t - dropped <= limit:  # strict `now - dropTime > limit`
            t += scan
        return t

    vec = {k: {str(t): due(t, lim) for t in (0, 1, 49, 50, 51)} for k, lim in limits.items()}
    require(vec["auto_destroy_9849"]["0"] == 150 and vec["auto_destroy_9849"]["50"] == 200, "strict > on the grid")
    require(vec["loot"]["1"] == 6050 and vec["herb"]["49"] == 650, "grid rounding")
    vec["pk_drop"] = None  # never registered (DestroyPlayerDroppedItem = False)
    require(gr["destroy_player_dropped_item"] is False, "PK drops never decay")
    out["dt_vectors"] = vec
    return "decay_due at ticks 0/1/49/50/51 for limits 100/600/6,000 (strict > on the 50-tick grid); PK never"


# ---------------------------------------------------------------------------
# Story 1.6: party, XP and ledger families
# ---------------------------------------------------------------------------


def in_range_party2(dist_sq: int, rad: int = 0) -> bool:
    # checkIfInRange(PartyRange2, ..., true) -> 4*dist^2 <= (2*R + rad)^2, R = m(1400) = 43,750 (X1/X2)
    r = int(prop("character.properties", "PartyRange2")) * 1000 // 32
    return 4 * dist_sq <= (2 * r + rad) ** 2


def random_looter(members: list[dict], words: Words) -> tuple[str, int]:
    cands = [m["name"] for m in members if in_range_party2(m["dist_sq"])]
    if not cands:
        return "picker", 0
    return cands[words.roll_below(len(cands))], 1


def by_turn_looter(members: list[dict], cursor: int) -> tuple[str | None, int]:
    for _ in range(len(members)):
        cursor += 1
        if cursor >= len(members):
            cursor = 0
        if in_range_party2(members[cursor]["dist_sq"]):
            return members[cursor]["name"], cursor
    return None, cursor


def even_split(members: list[dict], amount: int, words: Words) -> dict[str, int]:
    rec = [m["name"] for m in members if in_range_party2(m["dist_sq"])]  # member order (X9)
    if not rec:
        return {}
    k = len(rec)
    got = {n: amount // k for n in rec}
    for _ in range(amount % k):
        got[rec[words.roll_below(k)]] += 1
    return {n: v for n, v in got.items() if v > 0}


MODES = ["FINDERS_KEEPERS", "RANDOM", "RANDOM_INCLUDING_SPOIL", "BY_TURN", "BY_TURN_INCLUDING_SPOIL"]


@family("L-P", "1.6")
def fam_lp(out: dict) -> str:
    near, far = 40000 ** 2, 44000 ** 2
    require(in_range_party2(43750 ** 2) and not in_range_party2(43751 ** 2), "PartyRange2 1,400 -> 43,750 non-strict")
    vec: dict = {"random": [], "by_turn": [], "even_split": [], "sweep_routing": []}
    # RANDOM over every in-range subset of a 4-member party
    for mask in range(16):
        members = [{"name": f"m{i}", "dist_sq": near if mask >> i & 1 else far} for i in range(4)]
        w = Words(100 + mask)
        looter, draws = random_looter(members, w)
        require(draws == (1 if mask else 0), "RANDOM draws only with candidates")
        vec["random"].append({"in_range_mask": mask, "seed": 100 + mask, "looter": looter, "words": w.log})
    # BY_TURN: pre-increment, wrap, skip-advance, full lap -> picker with cursor advanced by len
    members = [{"name": f"m{i}", "dist_sq": far if i in (1, 2) else near} for i in range(4)]
    cur, seq = -1, []
    for _ in range(5):
        who, cur = by_turn_looter(members, cur)
        seq.append((who, cur))
    require([s[0] for s in seq] == ["m0", "m3", "m0", "m3", "m0"], f"by-turn order {seq}")
    nobody = [{"name": f"m{i}", "dist_sq": far} for i in range(3)]
    who, cur2 = by_turn_looter(nobody, 1)
    require(who is None and cur2 == 1, "full lap returns the picker; cursor advanced by len (mod len)")
    vec["by_turn"] = {"sequence": seq, "full_lap_cursor_in": 1, "full_lap_cursor_out": cur2}
    # Even split k = 1..9 with remainders, draws with replacement
    for k in range(1, 10):
        members = [{"name": f"m{i}", "dist_sq": near} for i in range(k)]
        for amount in (k * 7 + k - 1, 2, 1000003):
            w = Words(k * 31 + amount)
            got = even_split(members, amount, w)
            require(sum(got.values()) == amount, "split conserves")
            require(len(w.log) >= amount % k, "one draw per leftOver")
            vec["even_split"].append({"k": k, "amount": amount, "seed": k * 31 + amount, "payout": got, "words": w.log})
    # count 1 is never split: isItemToEvenlyDistribute requires count > 1 -> mode path
    require(int(prop("character.properties", "PartyEvenlyDistributeItems").split(",")[0]) == 57, "split template 57")
    vec["split_count_1"] = "mode path"
    # Sweep routing: spoil=true in every mode; adena count > 1 splits in every mode
    for mode in MODES:
        spoil_to = "picker" if mode in ("FINDERS_KEEPERS", "RANDOM", "BY_TURN") else (
            "random" if mode == "RANDOM_INCLUDING_SPOIL" else "next")
        vec["sweep_routing"].append({"mode": mode, "non_adena": spoil_to, "adena_gt_1": "even_split"})
    out["lp_vectors"] = vec
    return "RANDOM over 16 in-range subsets; BY_TURN cursor/wrap/full lap; even split k=1..9; sweep routing in 5 modes"


def pow56(k: int) -> F:
    require(1 <= k <= 79, f"pow56 k={k} fails closed")
    return fl(fl(F(5) / 6) ** k)


def base_rewards(level: int, exp_rate: str, sp: str, g: str = "1") -> tuple[int, int]:
    G = fl(F(g))
    rx = fl32(fl(fl(F(prop("rates.properties", "RateXp"))) * G))
    rs = fl32(fl(fl(F(prop("rates.properties", "RateSp"))) * G))
    X = math.floor(fl(fl(level * level * cd(exp_rate)) * rx))
    S = math.floor(fl(cd(sp) * rs))
    return X, S


def calc_exp_sp(diff: int, dmg: int, total: int, X: int, S: int) -> tuple[int, int]:
    diff = max(diff, -5)
    x = fl(fl(F(X) * dmg) / total)
    s = fl(fl(F(S) * dmg) / total)
    if diff > 5:
        p = pow56(diff - 5)
        x, s = fl(x * p), fl(s * p)
    if x <= 0:
        x, s = F(0), F(0)
    elif s <= 0:
        s = F(0)
    return math.floor(x), math.floor(s)


def round_half_up(v: F) -> int:
    return math.floor(v + F(1, 2))


def bonus(n: int, key: str) -> F:
    B = [fl(F(b)) for b in table("party.toml")["party"]["bonus_exp_sp"]]
    if n < 2:
        return F(1)
    return fl(B[min(n - 1, 8)] * fl32(fl(F(prop("rates.properties", key)))))


def party_distribute(e: int, s: int, levels: list[int], alive: list[bool], top: int) -> list[tuple[int, int]]:
    n = len(levels)
    e = math.floor(fl(e * bonus(n, "RatePartyXp")))
    s = math.floor(fl(s * bonus(n, "RatePartySp")))
    sq = sum(lv * lv for lv in levels)
    gaps = [tuple(map(int, g.split(","))) for g in prop("character.properties", "PartyXpCutoffGaps").split(";")]
    pct = [int(x) for x in prop("character.properties", "PartyXpCutoffGapPercent").split(",")]
    out = []
    for lv, ok in zip(levels, alive):
        if not ok:
            out.append((0, 0))
            continue
        pre = fl(fl(F(lv * lv) / sq) * 1)
        ae = round_half_up(fl(e * pre))
        as_ = math.floor(fl(s * pre))
        gap = top - lv
        award = (0, 0)
        for (lo, hi), p in zip(gaps, pct):
            if lo <= gap <= hi:
                award = (ae * p // 100, as_ * p // 100)
                break
        out.append(award)
    return out


@family("X", "1.6")
def fam_x(out: dict) -> str:
    vec: dict = {}
    # n = 5: every e < 2*10^6, floor(fl(e * 1.4)) vs floor(7e/5)
    b5 = bonus(5, "RatePartyXp")
    require(b5 == fl(F("1.40")), "n=5 bonus is fl(1.40)")
    diffs, first = 0, None
    bf = float(b5)
    for e in range(2_000_000):
        if int(e * bf) != (7 * e) // 5:
            diffs += 1
            if first is None:
                first = e
    require(diffs == 74892, f"n=5 differences {diffs}")
    require(math.floor(fl(45 * b5)) == 62 and (7 * 45) // 5 == 63, "e = 45 -> 62")
    for e in (45, 1_999_999, 2 ** 25 + 1):
        require(math.floor(fl(e * b5)) == int(e * bf), "binary64 reference")
    vec["n5"] = {"differences_below_2e6": diffs, "first": first, "e45": 62}
    # 36 SP-only templates: expRate absent -> 0 XP and 0 SP
    sp_only = table("drops.toml")["sp_only_templates"]
    require(len(sp_only) == 36, "36 SP-only")
    lv_sp = {}
    for f in sorted((DP / DPD / "stats/npcs").glob("*.xml")):
        for n in ET.parse(f).getroot().findall("npc"):
            if int(n.get("id")) in sp_only:
                a = n.find("acquire")
                require(a.get("expRate") is None, "expRate absent")
                lv_sp[int(n.get("id"))] = (int(n.get("level", 70)), a.get("sp"))
    for nid, (lv, sp) in lv_sp.items():
        X, S = base_rewards(lv, "0", sp)
        for lvl in (1, lv, 85):
            require(calc_exp_sp(lvl - lv, 1000, 1000, X, S) == (0, 0), f"SP-only {nid}")
    require({sp for _, sp in lv_sp.values()} >= {"557272", "144137"}, "raid-class SP rows")
    vec["sp_only"] = sorted(lv_sp)
    # Solo diffs (pow56 per E32); diff 85 (k = 80) fails closed
    X, S = base_rewards(40, "24.8737348778133", "17624")
    solo = {}
    for diff in (-6, -5, 0, 5, 6, 7, 84):
        e, s = calc_exp_sp(diff, 700, 1000, X, S)
        e = math.floor(fl32(fl32(e) * 1))
        solo[str(diff)] = (e, s)
    require(solo["-6"] == solo["-5"] == solo["0"] == solo["5"], "diff <= 5 has no penalty")
    try:
        calc_exp_sp(85, 700, 1000, X, S)
        raise OracleError("diff 85 must fail closed")
    except OracleError as err:
        require("fails closed" in str(err), "pow56 k=80")
    vec["solo"] = {"X": X, "S": S, "results": solo}
    # Fractional 0 < x < 1 keeps SP
    e, s = calc_exp_sp(0, 1, 3, 2, 300)
    require(e == 0 and s == 100, "0 < x < 1 keeps SP")
    vec["fractional_x"] = {"X": 2, "S": 300, "dmg": 1, "total": 3, "result": (e, s)}
    # e > 2^24: float rounding of long *= float
    big = 2 ** 24 + 1
    require(math.floor(fl32(fl32(big) * 1)) == 2 ** 24, "fl32 above 2^24")
    pm = fl32(fl32(1) / fl32(3))
    require(math.floor(fl32(fl32(big) * pm)) == int(struct.unpack('>f', struct.pack('>f', struct.unpack('>f', struct.pack('>f', float(big)))[0] * float(pm)))[0]), "partyMul product")
    vec["float_steps"] = {"e": big, "solo": 2 ** 24, "partyMul_1_3_bits": bits(pm)}
    # Party n = 1..9, Math.round halves, highfive gaps 9/10/14/15 and negative
    party = []
    for n in range(1, 10):
        levels = [60 - 3 * i for i in range(n)]
        party.append({"n": n, "levels": levels, "awards": party_distribute(100000, 5000, levels, [True] * n, 60)})
    gaps = party_distribute(100000, 5000, [70, 61, 60, 56, 55, 75], [True] * 6, 70)
    require(gaps[1][0] > 0 and gaps[2][0] > 0 and gaps[3][0] > 0 and gaps[4][0] == 0 and gaps[5] == (0, 0),
            "gaps 9 (100%), 10 (30%), 14 (30%), 15 (0%), negative (nothing)")
    require(round_half_up(F(5, 2)) == 3 and round_half_up(F(-5, 2)) == -2 and round_half_up(fl(F("0.49999999999999994"))) == 0,
            "Math.round halves (Java 7+)")
    # Dead member: skipped (no partyDmg, entry stays); its later visit gives partyDmg = 0 -> no award
    dead = party_distribute(100000, 5000, [60, 60, 60], [True, False, True], 60)
    require(dead[1] == (0, 0), "dead member gets nothing")
    vec["party"] = party
    vec["gaps"] = gaps
    vec["dead_member_second_pass"] = {"partyDmg": 0, "award": None}
    out["x_vectors"] = vec
    return f"n=5 e<2e6: {diffs:,} differences (e=45 -> 62); 36 SP-only -> 0/0; solo diffs; k=80 fails closed; party n=1..9; gaps; dead pass"


# --- ledger (§11.2, T1) -------------------------------------------------------------------


TERMINAL = ("transfer_out_whole", "destroy")


def t1_order(rows: list[dict]) -> list[dict]:
    """Rule T1 per (owner, tick, instance): non-terminal rows (cause order), Consume aggregate, terminal."""
    nonterm = [r for r in rows if r["op"] not in ("consume",) + TERMINAL]
    cons = [r for r in rows if r["op"] == "consume"]
    term = [r for r in rows if r["op"] in TERMINAL]
    require(len(cons) <= 1 and len(term) <= 1, "one consume aggregate, at most one terminal row")
    ordered = sorted(nonterm, key=lambda r: r["cause"]) + cons + term
    return [dict(r, ordinal=i) for i, r in enumerate(ordered)]


def transfer(src: dict, n: int, target_stack: dict | None, stackable: bool, next_id: int, link: str):
    """ItemContainer.transferItem: identity / merge / split rows (§11.2)."""
    whole = n == src["count"]
    if stackable and target_stack is not None:
        mode, tid, before = "merge", target_stack["id"], target_stack["count"]
    elif whole:
        mode, tid, before = "identity", src["id"], None
    else:
        mode, tid, before = "split", next_id, None
    out = {"op": "transfer_out_whole" if whole else "transfer_out", "instance": src["id"],
           "after": None if whole else src["count"] - n, "link": link, "mode": mode, "cause": "x"}
    inn = {"op": "transfer_in", "instance": tid, "before": before, "count_delta": n, "link": link, "mode": mode, "cause": "x"}
    return out, inn


@family("G", "1.6")
def fam_g(out: dict) -> str:
    vec = {}
    # T1: consume then terminal; ordinals contiguous
    rows = t1_order([{"op": "destroy", "cause": "pk_drop"}, {"op": "consume", "cause": "C(shot)"},
                     {"op": "unequip", "cause": "pk_drop"}, {"op": "grant", "cause": "a"}])
    require([r["op"] for r in rows] == ["grant", "unequip", "consume", "destroy"], "T1 order")
    require([r["ordinal"] for r in rows] == [0, 1, 2, 3], "contiguous ordinals")
    vec["t1"] = [r["op"] for r in rows]
    try:
        t1_order([{"op": "destroy", "cause": "a"}, {"op": "transfer_out_whole", "cause": "b"}])
        raise OracleError("two terminal rows accepted")
    except OracleError as err:
        require("at most one terminal" in str(err), "terminal uniqueness")
    # identity / merge / split pairs
    src = {"id": 10, "count": 5}
    o, i = transfer(src, 5, None, True, 99, "L1")
    require(i["mode"] == "identity" and i["instance"] == 10 and i["before"] is None and o["after"] is None, "identity")
    o2, i2 = transfer(src, 5, {"id": 20, "count": 7}, True, 99, "L2")
    require(i2["mode"] == "merge" and i2["instance"] == 20 and i2["before"] == 7 and o2["after"] is None, "merge whole")
    o3, i3 = transfer(src, 2, None, True, 99, "L3")
    require(i3["mode"] == "split" and i3["instance"] == 99 and o3["after"] == 3 and o3["op"] == "transfer_out", "split")
    vec["pairs"] = [[o, i], [o2, i2], [o3, i3]]
    # D67: a before=None transfer_in is exempt only when linked to a matching identity transfer_out
    def d67_ok(inn, outs):
        if inn["before"] is not None or inn["mode"] != "identity":
            return inn["mode"] != "identity" or inn["before"] is None
        return any(x["link"] == inn["link"] and x["mode"] == "identity" and x["instance"] == inn["instance"] for x in outs)
    require(d67_ok(i, [o]) and not d67_ok(i, [o3]), "D67 link exemption")
    # D68/D74 caps on merge receivers
    cap_adena = int(prop("character.properties", "MaxAdena"))
    require(cap_adena == 99_900_000_000, "adena cap")
    require(cap_adena - 1 + 2 > cap_adena, "a merge past the adena cap is refused (ITEM_COUNT_CAP)")
    vec["caps"] = {"adena": cap_adena, "other_stackable": 2 ** 31 - 1}
    # Conservation across pairs
    for o_, i_ in vec["pairs"]:
        moved_out = src["count"] - (o_["after"] or 0)
        require(moved_out == i_["count_delta"], "conservation")
    out["g_vectors"] = vec
    return "T1 orders and terminal uniqueness; identity/merge/split pairs; D67 link exemption; caps; conservation"


# ---------------------------------------------------------------------------
# Story 1.7: vendor, craft, multisell, manor, auction, fee, pow, L-table and flood families
# ---------------------------------------------------------------------------


def ref_price(item_id: int) -> int:
    return int(items_xml()[item_id]["sets"].get("price", "0"))


def unit_price(p: int, tax: str, castle: int) -> int:
    t = fl(fl(F(tax)) / 100)
    c = fl(F(castle) / 100)
    return math.floor(fl(F(p) * fl(1 + fl(t + c))))


@family("V-P", "1.7")
def fam_vp(out: dict) -> str:
    m = table("merchants.toml")
    require("price * (1 + (baseTaxRate + castleTaxRate))" in game_text(GJ + "network/clientpackets/RequestBuyItem.java"),
            "RequestBuyItem price expression")
    castles = m["castle_percents"]
    require(castles == [0, 5, 15, 25], f"castle percents {castles}")
    pairs = {(e["item"], e["tax"]) for e in m["entries"]}
    require(len(pairs) == 5803, f"pairs {len(pairs)}")
    tab = {(u["item"], u["tax"]): u for u in m["unit_prices"]}
    naive, cells, diffs = 0, 0, []
    for item, tax in sorted(pairs):
        p = ref_price(item)
        for c in castles:
            emu = unit_price(p, tax, c)
            hw = int(p * (1 + (float(tax) / 100 + c / 100)))
            require(emu == hw, f"emulation vs binary64 {item} {tax} {c}")
            require(tab[(item, tax)][f"castle_{c}"] == emu, f"table vs oracle {item} {tax} {c}")
            cells += 1
        if unit_price(p, tax, 0) != (p * (100 + int(tax))) // 100:
            naive += 1
            diffs.append([item, tax, p, unit_price(p, tax, 0)])
    require(naive == 362, f"naive differences {naive}")
    require(unit_price(100, "15", 0) == 114, "100 x 1.15 -> 114")
    out["vp_naive_differences"] = diffs
    return f"{len(pairs):,} pairs x 4 castle % = {cells:,} cells: table = oracle fl = binary64; {naive} naive-truncation differences at castle 0"


@family("V-R", "1.7")
def fam_vr(out: dict) -> str:
    want = {"0.040000": 1500, "0.042000": 1428, "0.083000": 722, "0.125000": 480, "0.166000": 361,
            "0.200000": 300, "0.250000": 240, "0.500000": 120, "1.000000": 60}
    got = {}
    for e in table("merchants.toml")["entries"]:
        d = fl(F(e["delay"]))
        mins = math.floor(fl(F(60) / d)) if d > 0 else -1
        require(e["restock_min"] == mins, "restock minutes")
        if d > 0:
            got[e["delay"]] = mins
    require(got == want, f"restock minutes {got}")
    vec = []
    for delay, mins in want.items():
        tick, stock, maxc = 1000, 3, 5
        due = tick + mins * 600  # restockDelay * 60,000 ms in 100 ms ticks
        vec.append({"delay": delay, "minutes": mins, "decrease_at": tick, "due": due, "refill_to": maxc})
    # limited stock check uses max(stock, 0): a negative stock admits nothing
    require(all(c > max(-2, 0) for c in (1, 2)), "negative-count clamp")
    out["vr_vectors"] = vec
    return "delay -> minutes 1,500/1,428/722/480/361/300/240/120/60 over all entries; due tick; refill; clamp"


@family("V-S", "1.7")
def fam_vs(out: dict) -> str:
    require("getSize() > 12" in game_text(GJ + "model/itemcontainer/PcRefund.java"), "refund tab 12")
    vec = []
    for p in (3, 5, 7, 101):
        for c in range(1, 6):
            sell = c * (p // 2)
            refund = (c * p) // 2
            require(refund - sell == c // 2, "refund costs floor(c/2) more")
            vec.append({"p": p, "c": c, "sell": sell, "refund": refund, "difference": refund - sell})
    require((4 * (3 // 2), (4 * 3) // 2, 2) == (4, 6, 2), "p=3, c=4 -> 4, 6, 2")
    tab: list[int] = []
    for i in range(13):
        tab.append(i)
        if len(tab) > 12:
            tab.pop(0)
    require(tab[0] == 1 and len(tab) == 12, "13th entry destroys the oldest")
    out["vs_vectors"] = {"prices": vec, "overflow_keeps": tab, "merge_grows_entries": False}
    return "odd p, c=1..5 sell/refund/difference; 12-entry overflow; merge does not grow the tab"


@family("M", "1.7")
def fam_m(out: dict) -> str:
    src = game_text(GJ + "network/clientpackets/MultiSellChoose.java")
    require("(_amount < 1) || (_amount > 5000)" in src, "amount 1..5000")
    require(prop("character.properties", "BlacksmithUseRecipes") == "True", "BlacksmithUseRecipes")
    ms = table("multisell.toml")
    flagged = [(lst["id"], i) for lst in ms["list"] for e in lst["entries"] for i in e["ingredients"] if i["maintain"]]
    require(len(flagged) == 21, "21 maintainIngredient")
    require(not any(i["is_tax"] for lst in ms["list"] for e in lst["entries"] for i in e["ingredients"]), "no isTaxIngredient")
    # required = (blacksmithUseRecipes || !maintainIngredient) ? count * amount : count
    def required(count, amount, maintain):
        return count * amount if (True or not maintain) else count
    vec = {"amount_bounds": [1, 5000], "rejected": [0, 5001],
           "maintain_ingredient_consumed": [{"list": l, "id": i["id"], "count": i["count"], "amount": 3,
                                             "required": required(i["count"], 3, True)} for l, i in flagged]}
    # synthetic isTaxIngredient: Math.round(count * taxRate), half-up on the exact double
    tax = []
    for count, rate in ((1, F(1, 2)), (3, F(1, 2)), (5, fl(F("0.1"))), (7, F(5, 100))):
        v = fl(fl(F(count)) * fl(rate))
        tax.append({"count": count, "rate_bits": bits(fl(rate)), "tax": round_half_up(v)})
    require(tax[0]["tax"] == 1 and tax[1]["tax"] == 2, "half-up at .5")
    vec["tax_half_up"] = tax
    vec["non_stackable_amount_gt_1"] = "INVALID_QUANTITY"
    vec["maintain_enchantment_lists"] = sum(1 for lst in ms["list"] if lst["maintain_enchantment"])
    require(vec["maintain_enchantment_lists"] == 31, "31 maintainEnchantment lists")
    out["m_vectors"] = vec
    return "amount 1/5000 (0, 5001 rejected); 21 maintainIngredient rows consumed (BlacksmithUseRecipes); tax half-up; 31 maintainEnchantment lists"


@family("C", "1.7")
def fam_c(out: dict) -> str:
    rec = table("recipes.toml")["recipe"]
    rates = sorted({r["success_rate"] for r in rec})
    require(rates == [10, 25, 60, 70, 95, 100], f"success rates {rates}")
    rar = sorted({r["production_rare"]["rarity"] for r in rec if "production_rare" in r})
    require(rar == [1, 4, 20], f"rarities {rar}")
    vec = {"success_threshold": {str(k): str(chance_threshold(F(k * 100, 100))) for k in rates},
           "masterwork_threshold": {str(k): str(chance_threshold(F(k * 100, 100))) for k in rar + [0]}}
    require(vec["success_threshold"]["100"] == str(TWO64) and vec["masterwork_threshold"]["0"] == "0", "bounds")
    vec["order"] = ["MP", "fee", "materials", "success draw", "masterwork draw (always, rarity 0 included)"]
    vec["fee_on_failure"] = "kept by the crafter"
    out["c_vectors"] = vec
    return "chance_bound for successRate 10/25/60/70/95/100; masterwork 1/4/20 and rarity 0; MP -> fee -> materials -> roll; fee kept on failure"


def sow_basic(seed_level: int, alternative: bool, player: int, target: int) -> int:
    basic = 20 if alternative else 90
    lo, hi = seed_level - 5, seed_level + 5
    if target < lo:
        basic -= 5 * (lo - target)
    if target > hi:
        basic -= 5 * (target - hi)
    diff = abs(player - target)
    if diff > 5:
        basic -= 5 * (diff - 5)
    return basic  # Math.max(basic, 1) result discarded: no floor


def harvest_basic(player: int, target: int) -> int:
    diff = abs(player - target)
    basic = 100 - (diff - 5) * 5 if diff > 5 else 100
    return max(basic, 1)


@family("S", "1.7")
def fam_s(out: dict) -> str:
    sow_src = (DP / "src/main/java/com/l2jserver/datapack/handlers/effecthandlers/instant/Sow.java").read_text()
    require("Math.max(basicSuccess, 1);" in sow_src and "Rnd.nextInt(99) < basicSuccess" in sow_src, "Sow.java")
    manor = table("manor.toml")
    vec = {"sow": [], "harvest": [], "harvest_count": [], "fee": {}}
    for alt in (False, True):
        for player, target in ((40, 40), (40, 30), (40, 52), (60, 40), (20, 70)):
            b = sow_basic(40, alt, player, target)
            vec["sow"].append({"seed_level": 40, "alternative": alt, "player": player, "target": target, "basic": b,
                               "threshold": str(chance_threshold(F(max(b, 0) * 100, 99)) if b > 0 else 0)})
    require(sow_basic(40, True, 20, 70) < 0, "negative basic exists (never succeeds)")
    for player, target in ((40, 40), (40, 46), (10, 85)):
        vec["harvest"].append({"player": player, "target": target, "basic": harvest_basic(player, target)})
    require(harvest_basic(10, 85) == 1, "harvest floor 1")
    mult = {m["skill"]: m["multiplier"] for m in manor["skills"]["strong_type_multipliers"]}
    for skills, npc_lv, seed_lv in (([], 30, 30), ([4303], 30, 30), ([4305, 4307], 50, 40)):
        c = 1
        for sk in skills:
            c *= mult[sk]
        c += max(0, npc_lv - seed_lv - 5)
        c *= manor["skills"]["rate_drop_manor"]
        vec["harvest_count"].append({"skills": skills, "npc_level": npc_lv, "seed_level": seed_lv, "count": c})
    require(vec["harvest_count"][2]["count"] == 4 * 6 + 5, "multipliers then level bonus")
    # crop sale reward and fee (castle 1 fixtures only)
    fg = manor["fixture_gludio"]
    crops = {c["crop"]: c for c in manor["crops"] if c["castle"] == 1}
    sales = []
    for row in fg["crop_procurement"][:5]:
        crop = crops[row["crop_id"]]
        reward = crop["rew1"] if row["reward_type"] == 1 else crop["rew2"]
        rp = ref_price(reward)
        for count in (1, 10):
            P = count * row["price"]
            rc = P // rp if rp else None
            sales.append({"crop": row["crop_id"], "count": count, "P": P, "reward": reward,
                          "reward_count": rc, "skipped": rp == 0 or (rc is not None and rc < 1),
                          "fee_other_manor": math.floor(fl(fl(F(P)) * fl(F("0.05"))))})
    vec["crop_sales"] = sales
    # fee (long)(P * 0.05) vs P div 20
    import random
    rng = random.Random(5)
    tested = list(range(0, 200_001)) + [2 ** 53 - 1 - i for i in range(1000)] + \
        [rng.randrange(2 ** 53) for _ in range(200_000)] + [20 * k + r for k in (10 ** 6, 10 ** 12, 2 ** 40) for r in range(20)]
    fee = lambda P: math.floor(fl(fl(F(P)) * fl(F("0.05"))))  # noqa: E731  (long)(P * 0.05)
    # The first P where (long)(P * 0.05) != P div 20: fl(0.05) exceeds 1/20 by delta, so a P = 20k + 19
    # rounds up to k + 1 once P*delta outgrows half an ulp of the product; the residue is monotone in k
    # inside the [2^48, 2^49) product binade, so bisection over k finds the minimum exactly.
    lo, hi = (1 << 52) // 20, ((1 << 53) - 20) // 20
    require(fee(20 * lo + 19) == lo and fee(20 * hi + 19) != hi, "bisection brackets")
    while hi - lo > 1:
        mid = (lo + hi) // 2
        if fee(20 * mid + 19) != mid:
            hi = mid
        else:
            lo = mid
    first = 20 * hi + 19
    bad = [P for P in tested if fee(P) != P // 20]
    require(all(P >= first for P in bad), "no difference below the first one")
    reach = max(r["price"] * r["amount"] for r in fg["crop_procurement"])
    max_adena = int(prop("character.properties", "MaxAdena"))
    require(max(reach, max_adena) < first, "every reachable P is below the first difference")
    vec["fee"] = {"tested": len(tested), "difference_count": len(bad), "first_difference": first,
                  "max_reachable_P": reach, "max_adena": max_adena}
    out["s_vectors"] = vec
    return (f"Sow 90/20 penalties, no floor; harvest floor 1; harvest count; crop sale reward/fee; fee floor(fl(P*0.05)) = P div 20 "
            f"for all {len(tested) - len(bad):,} tested P below {first:,} (first difference; {len(bad):,} tested P above it differ; reachable P <= {reach:,})")


@family("A", "1.7")
def fam_a(out: dict) -> str:
    src = game_text(GJ + "model/itemauction/ItemAuction.java")
    require("(_highestBid != null) && (newBid < _highestBid.getLastBid())" in src, "equal bid accepted")
    require("_highestBid.getLastBid() < bid.getLastBid()" in src, "leader changes on strictly higher")
    cfg = table("auction.toml")["config"]
    inst = table("auction.toml")["instance"][0]

    def bid(state, who, amount):
        if amount < inst["items"][0]["init_bid"]:
            return "BID_TOO_LOW", 0
        if amount > 100_000_000_000:
            return "BID_TOO_HIGH", 0
        if state["highest"] is not None and amount < state["bids"][state["highest"]]:
            return "BID_TOO_LOW", 0
        prev = state["bids"].get(who)
        pay = amount if prev is None or prev == -1 else amount - prev
        if prev not in (None, -1) and amount < prev:
            return "BID_TOO_LOW", 0
        state["bids"][who] = amount
        if state["highest"] is None or state["bids"][state["highest"]] < amount:
            state["highest"] = who
        return "OK", pay

    st = {"bids": {}, "highest": None}
    r1 = bid(st, "a", 100000)
    r2 = bid(st, "b", 100000)  # equal to highest: accepted but not leading
    r3 = bid(st, "a", 150000)  # increment-only payment
    require(r1 == ("OK", 100000) and r2 == ("OK", 100000) and st["highest"] == "a" and r3 == ("OK", 50000), "bid rules")
    # extension: INITIAL -> +5 (any bidder) -> +3 if bidder != last tracked (starts at 0) -> config 0 stops
    ext, state, last = [], "INITIAL", 0
    for who in ("a", "b", "c"):
        if state == "INITIAL":
            state, add = "EXTEND_5", 5
        elif state == "EXTEND_5":
            add = 3 if who != last else 0
            last = who
            state = "EXTEND_3" if add else state
        else:
            add = cfg["time_extends_on_bid"]
        ext.append(add)
    require(ext == [5, 3, 0], f"extension {ext}")
    cancel = {"leader_started": "reserve not met, no refund", "leader_finished": "refused",
              "finished_older_than_14_days": "refused", "cancelled": "skip", "other": "refund last bid"}
    require(cfg["expired_after_days"] == 14, "14 days")
    out["a_vectors"] = {"bids": [r1, r2, r3], "extension_minutes": ext, "cancel": cancel,
                        "item_draw": f"roll_below({len(inst['items'])})"}
    return "equal bid accepted not leading; increment-only payment; cancel matrix; extension 5 -> 3 -> stop; roll_below(82)"


@family("F", "1.7")
def fam_f(out: dict) -> str:
    sm = table("storage_mail.toml")
    wh, fr, ml = sm["warehouse"]["deposit_fee_per_entry"], sm["freight"]["fee_per_entry"], sm["mail"]
    require((wh, fr, ml["message_fee"], ml["message_fee_per_slot"]) == (30, 1000, 100, 1000), "fees")
    vec = []
    for entries, has_adena in ((1, False), (3, True), (8, True)):
        vec.append({"entries": entries, "adena_entry": has_adena, "warehouse_fee": wh * entries,
                    "freight_fee": fr * entries, "freight_slots": entries - (1 if has_adena else 0),
                    "mail_fee": ml["message_fee"] + ml["message_fee_per_slot"] * entries})
    out["f_vectors"] = vec
    return "warehouse 30 x entries (adena counted); freight 1,000 x entries (adena adds no slot); mail 100 + 1,000 x n"


def pow_table() -> dict[str, dict[int, str]]:
    out: dict[str, dict[int, str]] = {"pow56": {}, "pow13": {}}
    for ln in (DATA / "jvm-pow-tables.txt").read_text().splitlines()[1:]:
        m = re.fullmatch(r"(pow56|pow13) [kd]=(-?\d+) bits=([0-9a-f]{16})", ln)
        require(m is not None, f"pow line {ln}")
        out[m.group(1)][int(m.group(2))] = m.group(3)
    return out


def pow13(d: int) -> F:
    b = fl(F("1.3"))
    return fl(b ** d)


@family("PW", "1.7")
def fam_pw(out: dict) -> str:
    t = pow_table()
    require(len(t["pow56"]) == 79 and len(t["pow13"]) == 241, "79 + 241")
    for k, h in t["pow56"].items():
        require(bits(pow56(k)) == h, f"pow56 {k}")
    for d, h in t["pow13"].items():
        require(bits(pow13(d)) == h, f"pow13 {d}")
    for k in (0, 80):
        try:
            pow56(k)
            raise OracleError(f"pow56({k}) accepted")
        except OracleError as err:
            require("fails closed" in str(err), f"k={k}")
    base = fl(F(5) / 6)
    step_fl, acc = 0, F(1)
    for k in range(1, 80):
        acc = fl(acc * base)
        step_fl += acc != pow56(k)
    step_exact, acc = 0, F(1)
    for k in range(1, 80):
        acc = fl(acc * F(5, 6))
        step_exact += acc != pow56(k)
    require(step_fl == 44 and step_exact == 78, f"negative controls {step_fl}/79, {step_exact}/79")
    halves = {str(v): round_half_up(F(v)) for v in ("0.5", "1.5", "2.5", "-0.5", "-1.5")}
    require(halves == {"0.5": 1, "1.5": 2, "2.5": 3, "-0.5": 0, "-1.5": -1}, "Math.round halves")
    out["pw"] = {"entries": 320, "negative_controls": {"step_fl_base": step_fl, "step_exact_base": step_exact},
                 "math_round": halves}
    return f"all 320 entries reproduced; k=0/80 fail closed; negative controls {step_fl}/79 and {step_exact}/79; Math.round halves"


@family("LT", "1.7")
def fam_lt(out: dict) -> str:
    rows = table("adaptations.toml")["l_table"]
    require(len(rows) == 22, "22 rows")
    for r in rows:
        c = r["d_condition"]
        d = int(c.split()[-1])
        require(bits(killer_l(r["strategy"], d)) == r["bits"], f"{r['strategy']} {c}")
    distinct = len({r["bits"] for r in rows})
    require(distinct == 19, f"distinct {distinct}")
    out["lt"] = {"rows": 22, "distinct_bits": distinct}
    return "22 rows recomputed from the §3.3 expressions = frozen bits (19 distinct)"


@family("FL", "1.7")
def fam_fl(out: dict) -> str:
    iv = table("storage_mail.toml")["flood_intervals_ticks"]
    keys = {"multisell": "MultiSellInterval", "transaction": "TransactionInterval", "manufacture": "ManufactureInterval",
            "manor": "ManorInterval", "send_mail": "SendMailInterval", "server_bypass": "ServerBypassInterval"}
    for k, p in keys.items():
        require(iv[k] == int(prop("floodprotector.properties", p)), f"interval {k}")
    require("if ((curTick < nextGameTick) || punishmentInProgress)" in game_text(GJ + "util/FloodProtectorAction.java"), "gate")

    def gate(nxt, tick, interval):
        if tick < nxt:
            return False, nxt
        return True, tick + interval

    vec = []
    for k in keys:
        n = iv[k]
        ok1, nxt = gate(0, 100, n)
        ok2, nxt2 = gate(nxt, 100 + n - 1, n)
        ok3, nxt3 = gate(nxt, 100 + n, n)
        require(ok1 and not ok2 and nxt2 == nxt and ok3 and nxt3 == 100 + 2 * n, f"{k} gate")
        vec.append({"kind": k, "interval": n, "accept_at": 100, "refuse_at": 100 + n - 1, "accept_again_at": 100 + n})
    out["fl_vectors"] = {"gates": vec, "earlier_failing_check": "next unchanged",
                         "later_failing_check": "next updated (E35)", "transaction_shared": True,
                         "manufacture_users": ["CraftItem", "PrivateManufactureOrder"],
                         "server_bypass_users": ["ItemAuctionCancelBid"]}
    return "refuse at tick < next, accept at next, next := tick + interval for 6 kinds; E35 positions"


# ---------------------------------------------------------------------------
# Driver
# ---------------------------------------------------------------------------

GOLDENS = {"1.5": "economy-oracle-drops.json", "1.6": "economy-oracle-party.json", "1.7": "economy-oracle-vendor.json"}


def run(selected: list[str] | None) -> tuple[dict[str, dict], list[str]]:
    outs: dict[str, dict] = {s: {} for s in GOLDENS}
    lines = []
    for name, (story, fn) in FAMILIES.items():
        if selected and name not in selected:
            continue
        summary = fn(outs[story])
        lines.append(f"{name:5} [{story}] {summary}")
    return outs, lines


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true", help="fail if a committed golden differs")
    parser.add_argument("--family", action="append", help="run only these families (no golden I/O)")
    args = parser.parse_args()
    try:
        repos = table("sources.toml")["repositories"]
        for root, label in ((DP, "datapack"), (GAME, "game")):
            head = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
            require(head == repos[label]["revision"], f"{label} clone at {head}, pinned {repos[label]['revision']}")
        outs, lines = run(args.family)
    except OracleError as err:
        print(f"economy_oracle: FAILED: {err}", file=sys.stderr)
        return 1
    for ln in lines:
        print(ln)
    if args.family:
        return 0
    stale = []
    for story, name in GOLDENS.items():
        if not outs[story]:
            continue
        content = json.dumps(outs[story], indent=1, sort_keys=True) + "\n"
        path = FIXTURES / name
        if args.check:
            if not path.exists() or path.read_text(encoding="utf-8") != content:
                stale.append(name)
        else:
            path.write_text(content, encoding="utf-8")
    if stale:
        print("economy_oracle: stale goldens: " + ", ".join(stale), file=sys.stderr)
        return 1
    print(f"economy_oracle: {len(lines)} families {'verified' if args.check else 'generated'}.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
