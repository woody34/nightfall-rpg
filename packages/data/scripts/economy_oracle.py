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
