# Data sources

Provenance for the generated stat tables (Phase 1 plan §2 fidelity gate, Story E1.1).
`tables/*.toml` and `classes/*.toml` are generated; `zones/` and `npcs/` are hand-written.

## Pinned High Five source

| Repository | Revision | Role |
|---|---|---|
| <https://bitbucket.org/l2jserver/l2j-server-game> (`develop`; GitHub mirror `monkey1sai/l2j-server-game`) | `abfde0490ac52a2106e884b7242c0276c44cdf76` (2026-09-13) | Formulas, functions, config defaults |
| <https://bitbucket.org/l2jserver/l2j-server-datapack> (`develop`) | `3ca488dd2bd0bfaca43e378886a3c2e37968153a` (2026-09-13) | Tables and templates |

Both READMEs: "develop branch is the current High Five development" (client: High Five part 5).

Files read (game prefix `src/main/java/com/l2jserver/gameserver/`, datapack prefix
`src/main/resources/data/`):

- Game: `model/stats/Formulas.java` (`calcPhysDam`, `calcHitMiss`, `calcCrit`),
  `model/stats/BaseStats.java`, `model/stats/functions/formulas/Func{PAtkMod,PDefMod,MaxHpMul,MaxMpMul,AtkAccuracy,AtkEvasion,AtkCritical,PAtkSpeed}.java`,
  `model/stats/functions/StatFunction.java` (`SET` order 0), `model/actor/L2Character.java`
  (`getLevelMod`, `calculateTimeBetweenAttacks`, `doAttack`, `getRandomDamageMultiplier`,
  `doRevive`), `model/actor/stat/{CharStat,PcStat,PlayableStat}.java`,
  `model/actor/L2Attackable.java`, `model/AggroInfo.java`,
  `model/actor/instance/L2PcInstance.java` (`calculateDeathExpPenalty`),
  `data/xml/impl/PlayerTemplateData.java`, `src/main/resources/config/character.properties`.
- Datapack: `stats/statBonus.xml`, `stats/expData.json`, `stats/chars/playerXpPercentLost.xml`,
  `stats/chars/baseStats/{Human,Elven,Dark,Orc,Dwarven}{Fighter,Mystic}.xml` (no Dwarven mystic),
  `stats/initialEquipment.xml`, `stats/items/02300-02399.xml` (item 2369).

## Regenerating

```bash
git clone -b develop https://bitbucket.org/l2jserver/l2j-server-game.git /tmp/l2j-game
git -C /tmp/l2j-game checkout abfde0490ac52a2106e884b7242c0276c44cdf76
git clone -b develop https://bitbucket.org/l2jserver/l2j-server-datapack.git /tmp/l2j-dp
git -C /tmp/l2j-dp checkout 3ca488dd2bd0bfaca43e378886a3c2e37968153a
python3 packages/data/scripts/gen_tables.py --game /tmp/l2j-game --datapack /tmp/l2j-dp --check
```

The script refuses other revisions, uses `decimal`/`fractions` only, parses every constant
from a literal source line that must exist verbatim, and proves each class HP/MP quadratic
reproduces all 85 source rows. `--check` fails on any drift from the committed files.

## Fidelity ledger (resolved)

| Item | Resolved value | Source |
|---|---|---|
| Normal-attack damage coefficient | **76**; crit ×2 | `Formulas.calcPhysDam`: `damage = (76 * damage * proximityBonus) / defence;` |
| Attack interval | **500000 / pAtkSpd** ms, impact at half | `L2Character.calculateTimeBetweenAttacks`, `doAttack` (`timeAtk / 2`) |
| Accuracy | **`sqrt(DEX) * 6 + L`**, `+(L−69)` above 69, `+(L−76)` above 77 | `FuncAtkAccuracy` |
| Evasion (player) | **`sqrt(DEX) * 6 + L`**, `+(L−69)` from 70, ×1.2 on that addition from 78; cap 250 | `FuncAtkEvasion`, `MaxEvasion` |
| Stat bonuses | **`statBonus.xml` 2-decimal tables**, indices 0–99; comments `1.036^(STR−34.845)`, `1.020^(INT−31.375)`, `1.030^(CON−27.632)`, `1.010^(MEN+0.060)`, `1.009^(DEX−19.360)`, `1.050^(WIT−20.000)` | `BaseStats` static loader |
| Class HP/MP | Per-level `lvlUpgainData` rows 1–85; exact quadratics (e.g. Human Fighter `80 + 11.765n + 0.065n²`, `30 + 5.43n + 0.03n²`) | `baseStats/*.xml` |
| XP table | `expData.json` levels 1–85, **sentinel L86 = 16,890,558,728**; cap `X[86] − 1` | `ExperienceData`, `PlayableStat.addExp` |
| Death loss | 10.0 % at L1, −0.125/level to 4.0 % at L49, 4.0 % to L75, 2.5/2.0/1.5 % at L76/77/78, 1.0 % L79–85 | `playerXpPercentLost.xml` |
| Starter weapon | Squire's Sword: P.Atk 6, crit 8, speed 379, random 10, range 40, accuracy 0 | item 2369 |
| Caps / respawn | crit 500 ‰, speed 1500, hit 200–980 ‰, HP 0.65 / MP 0, protection 600 s, max level 85 | `character.properties` |

## Errata against the planning docs

- **E-1 Stat curves.** `01-stat-formulas.md` §2.1 calls `1.009^(STR−49)` … `1.00369^(MEN+30.45)`
  and Human Fighter STR 88 "High Five". They come from `andridgitalbox/l2j-mobius`
  (`d2a8c5d5`, an Ertheia-era tree: LUC/CHA, Ertheia classes). HF uses the tables whose
  comments the doc labels "Interlude". Plan vector 1 is re-based on HF (STR 40 = 1.20).
- **E-2 XP table.** The doc's "High Five" column (L41 8,718,976; L85 5,008,025,097) is the
  Mobius/Ertheia table. HF `expData.json` matches the doc's "Interlude" column only to L10 and
  L76–78 (e.g. L11 71,202 vs 71,201; L80 3,075,966,164; L85 13,180,481,103).
- **E-3 Accuracy/evasion.** HF uses `* 6` and the level additions above, not stats §2.4's
  `* 5` / "+1 at 78+, +2 at 81+". Combat §2.3's `* 6` is right; its additions are imprecise.
- **E-4 CON 43** is 1.58 in the HF table (the doc prints 1.57).
- **E-5 Weapon stats replace, not add.** Item stats are `<set>` funcs (order 0), applied before
  the STR/DEX/level multipliers (order 1), so P.Atk = `weapon × STR × LM`, not
  `(fist + weapon) × …`; crit base and attack speed are likewise replaced (8, 379).
- **E-6 Spawn protection** is `PlayerSpawnProtection = 600` **seconds** (6000 ticks), not
  600 ticks.
- **E-7 Rounding.** L2J rounds where Nightfall's §3.1 contract does not: death loss
  `Math.round` (Nightfall floors), accuracy/evasion `Math.round` to int, crit `(int)` then
  `+0.5`, attack speed `Math.round`, per-level HP/MP stored as Java `float`. Nightfall keeps
  exact Q values per plan D3; differences are ≤ 1 unit and listed here for the E1.4 review.
- **E-8 The "77".** HF's normal attack uses 76; 77 appears only in the crit additive term
  (`CRITICAL_DAMAGE_ADD * 77 / defence`, zero without buffs) and in skill/blow damage.
- **E-9 Interval.** Stats §2.4 prints 470000 as HF; HF is 500000. The 470000 value was not
  verified against an Interlude source here.
- **E-10 Misc, unused in Phase 1.** `MaxRunSpeed = 250` (doc: 300); `baseMDef` sums the right
  ring twice (`PlayerTemplateData`); fists use random radius `5 + (int) sqrt(level)` (template
  `baseRndDam` unused for players); an attack dealing 0 hate to a player-attacked mob adds 1.
- **E-11 Table index 0.** CON[0] 0.45 and WIT[0] 0.39 differ from their comment expressions
  (0.44, 0.38); the table is authoritative and index 0 is unreachable (base stats ≥ 1).

## Missing rows

- Kamael starters (class 123/124) are omitted: Nightfall has no Kamael race.
- XP row 87 exists in the source and is not used (only the L86 sentinel is).
- Nothing in the E1.1 ledger was left unsourced.
