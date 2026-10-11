# Nightfall Skill Definitions (Phase 3)

Canonical, file-per-skill definitions generated from the pinned Lineage II High Five datapack.

## 1. Pinned Source Provenance

All skill definitions are extracted by `packages/data/scripts/gen_skills.py` from the pinned L2J High Five source repositories:

| Repository | Revision | Role |
|---|---|---|
| `https://bitbucket.org/l2jserver/l2j-server-datapack` | `3ca488dd2bd0bfaca43e378886a3c2e37968153a` | Skill definitions (`data/stats/skills/*.xml`), skill trees (`data/skillTrees/classSkillTree.xml`) |
| `https://bitbucket.org/l2jserver/l2j-server-game` | `abfde0490ac52a2106e884b7242c0276c44cdf76` | Combat formulas (`Formulas.java`), skill execution logic (`Skill.java`, `L2Character.java`) |

## 2. Design Principles & Numeric Representation

- **One file per skill:** Located at `packages/data/skills/<id>.toml` where `<id>` is the integer skill ID.
- **Strictly No Floats:** All numbers in skill definitions are integers or exact decimal strings (e.g. `"1.08"`, `"0.91"`, `"0.772184315"`). No binary floating point types are serialized; source IEEE rounding sites are explicitly modeled in the independent oracle.
- **Deterministic Array Length:** Per-level arrays (`mp_consume_start`, `mp_consume_end`, `hp_consume`, `magic_level`, `effect_point`, abnormal `level`, abnormal `time_s`, and stat child `value`) always have length exactly equal to `max_level`.
- **Computed Runtime Status:** `runtime_status` is strictly computed by the generator, never hand-edited. A skill is `"executable"` if and only if its `blockers` array is empty. Otherwise, `runtime_status` is `"deferred"` and all blocking capabilities are recorded in `blockers`.

## 3. TOML Schema Reference

Each skill TOML file adheres to the following structure:

### 3.1 Provenance Header
```toml
provenance = { repository = "https://bitbucket.org/l2jserver/l2j-server-datapack", revision = "3ca488dd2bd0bfaca43e378886a3c2e37968153a", file = "src/main/resources/data/stats/skills/00000-00099.xml", skill_id = 3 }
```
- `repository` (string): Datapack git repository URI.
- `revision` (string): 40-character commit hash.
- `file` (string): Repository-relative path to the XML source file.
- `skill_id` (integer): Source skill ID.

### 3.2 Identification & Operational Type
```toml
id = 3
key = "l2.skill.3"
l2_ref = "Power Strike"
max_level = 9
operate_type = "A1"
is_magic = 0
```
- `id` (integer): Skill ID.
- `key` (string): Globally unique skill identifier (`l2.skill.<id>`).
- `l2_ref` (string): Source skill name verbatim.
- `max_level` (integer): Maximum non-enchanted level of the skill.
- `operate_type` (string): Source operate type (`"A1"`: active instant, `"A2"`: active instant + continuous, `"A3"`: active continuous buff/debuff, `"P"`: passive).
- `is_magic` (integer): Magic classification (`0`: physical, `1`: magical, `2`: static / special / dance / song).

### 3.3 Targeting & Affect Scope
```toml
target_type = "ENEMY"
affect_scope = "SINGLE"
affect_range = 0
affect_limit = { offset = 10, width = 10 }
affect_object = ""
```
- `target_type` (string or array of strings): Primary targeting requirement (`"SELF"`, `"TARGET"`, `"ENEMY"`, `"ENEMY_ONLY"`, `"PC_BODY"`). If target type varies across levels via source tables, an array of length `max_level` is used.
- `affect_scope` (string): Area of effect scope (`"SINGLE"`, `"POINT_BLANK"`, `"RANGE"`, or `"unset"`).
- `affect_range` (integer, optional): Radius of the affect scope in world units (scalar or per-level array).
- `affect_limit` (table, optional): Structured target limit `{ offset = N, width = M }` per `Skill.getAffectLimit` (`Skill.java:637-641`). The runtime calculates: width 0 → no draw and NO limit (unlimited); width > 0 → offset + Rnd.get(width). Specifically, `Skill.java:638-640` checks `if (affectLimit[1] == 0) return 0;` (where width is `affectLimit[1]`). When `getAffectLimit()` returns 0, `AffectScope.java` treats limit 0 as UNLIMITED: target count loops guard break with `if ((affectLimit > 0) && (targets.size() >= affectLimit))` (`AffectScope.java:73-75, 143-145, 182-184, 233-235, 287-289, 357-359`), and stream scopes apply `.limit(affectLimit > 0 ? affectLimit : Integer.MAX_VALUE)` (`AffectScope.java:386, 401, 417`). Because the check requires `affectLimit > 0`, returning 0 means no limit is enforced (unlimited targets).
- `affect_object` (string, optional): Target filter within scope (e.g. `"NOT_FRIEND"`).

### 3.4 Range, Timing & Resources
```toml
cast_range = 40
effect_range = 400
hit_time_ms = 1080
cool_time_ms = 720
reuse_ms = 3000
hit_cancel_ms = 500

mp_consume_start = [0, 0, 0, 0, 0, 0, 0, 0, 0]
mp_consume_end = [9, 9, 10, 11, 12, 13, 16, 16, 17]
hp_consume = [0, 0, 0, 0, 0, 0, 0, 0, 0]
magic_level = [3, 4, 5, 8, 9, 10, 13, 14, 15]
effect_point = [-52, -54, -57, -64, -67, -70, -79, -83, -86]
```
- `cast_range` (integer or per-level array): Maximum range in world units to initiate cast.
- `effect_range` (integer or per-level array): Maximum range in world units at cast completion.
- `hit_time_ms` (integer or per-level array): Base cast animation time in milliseconds before attack/cast speed scaling.
- `cool_time_ms` (integer or per-level array): Imported `coolTime` in milliseconds; unused by the pinned execution path (Skill.java:633; L2Character.java:4666,4690).
- `reuse_ms` (integer or per-level array): Base reuse cooldown delay in milliseconds.
- `hit_cancel_ms` (integer): Damage interruption window attribute from source (imported, unused by engine per D-R2).
- `mp_consume_start` (array of integers, length `max_level`): MP deducted at cast initiation (`mpConsume1`).
- `mp_consume_end` (array of integers, length `max_level`): MP deducted at cast completion (`mpConsume2`).
- `hp_consume` (array of integers, length `max_level`): HP deducted at cast completion (`hpConsume`).
- `magic_level` (array of integers, length `max_level`): Effective magic level used for level-difference and success calculations.
- `effect_point` (array of integers, length `max_level`): Aggro / hate points generated by the skill (negative for offensive skills).

### 3.5 Combat Modifiers & Flags
```toml
activate_rate = -1
basic_property = "NONE"
level_bonus_rate = 0
trait = "NONE"
min_chance = 10
max_chance = 90

next_action_attack = true
has_enchant_routes = false
runtime_status = "executable"
blockers = []
```
- `activate_rate` (integer): Base debuff land chance percentage (-1 if none or unconditional).
- `basic_property` (string): Saving throw base attribute (`"STR"`, `"CON"`, `"DEX"`, `"INT"`, `"MEN"`, `"WIT"`, `"NONE"`).
- `level_bonus_rate` (integer): Multiplier for level-difference modifier in `calcEffectSuccess`.
- `trait` (string): Trait classification (`"SHOCK"`, `"SLEEP"`, `"HOLD"`, `"PARALYZE"`, `"POISON"`, `"BLEED"`, `"DERANGEMENT"`, `"NONE"`).
- `min_chance` / `max_chance` (integer): Lower and upper clamps for debuff land rate (pinned character.properties:277–278 defaults 10 and 90, read by Skill.java:276–277).
- `next_action_attack` (boolean): Whether player automatically transitions to auto-attack after landing the skill.
- `has_enchant_routes` (boolean): Indicates whether enchant skill routes exist in source data (enchant data omitted from Phase 3 runtime).
- `runtime_status` (string): `"executable"` if `blockers` is empty; otherwise `"deferred"`.
- `blockers` (array of strings): Lexicographically sorted list of blocking capability identifiers.

### 3.6 Elemental Attribute Table
```toml
[attribute]
type = "NONE"
power = 0
```
- `type` (string): Attack element type (`"FIRE"`, `"WATER"`, `"WIND"`, `"EARTH"`, `"HOLY"`, `"UNHOLY"`, `"NONE"`).
- `power` (integer): Elemental skill attack power added to attribute bonus calculation.

### 3.7 Abnormal Status Table (Optional)
Present only when the skill applies a continuous abnormal state (buff, debuff, toggle):
```toml
[abnormal]
type = "STUN"
level = [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]
time_s = [9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9]
is_debuff = true
visual = "STUN"
```
- `type` (string): Abnormal slot/stack type (e.g. `"STUN"`, `"SPEED_UP"`, `"PD_UP"`).
- `level` (array of integers, length `max_level`): Abnormal intensity level for stacking precedence.
- `time_s` (array of integers, length `max_level`): Base duration in seconds.
- `is_debuff` (boolean): True for harmful continuous effects (occupies debuff slots, uses debuff land roll).
- `visual` (string, optional): Client visual effect identifier if present in source data.

### 3.8 Conditions Tree
Conditions are structured as a normalized recursive tree (`conditions`) alongside verbatim source `<cond>` XML nodes (`raw_conditions`):
```toml
raw_conditions = ["<cond><using kind=\"SWORD,BLUNT\" /></cond>"]

[[conditions]]
kind = "and"

[[conditions.conditions]]
kind = "using_kind"
weapon_kinds = ["SWORD", "BLUNT"]
```

Supported condition kinds:
- `{ kind = "using_kind", weapon_kinds = ["SWORD", "BLUNT"] }`: Validated against equipped weapon.
- `{ kind = "player_can_resurrect", value = true }`: State check for valid dead target resurrection.
- Logical operators: `{ kind = "and", conditions = [...] }`, `{ kind = "or", conditions = [...] }`, `{ kind = "not", conditions = [...] }`.
- Verbatim unsupported: `{ kind = "unsupported", tag = "player", attrs = { Charges = "3" } }`.
- Recursive conditions serialize as TOML arrays of tables (`[[conditions]]`, `[[conditions.conditions]]`, etc.). Unsupported `attrs` use TOML `=` and retain attribute strings verbatim.
- The same `conditions` / `raw_conditions` fields occur on effects and stat children. Stat conditions may be bare `<using>`, `<player>`, or boolean nodes (DocumentBase.java:246), as well as `<cond>` wrappers. Section attachment conditions use `section_conditions` / `section_raw_conditions` on each effect (DocumentBase.java:201–223).
- `raw_conditions` preserves parsed XML nodes including attributes like `msgId` and `addName`; XML indentation is normalized, and ElementTree may normalize entity spelling. This is XML semantic preservation, not original file bytes.

### 3.9 Item Consumption
```toml
[[item_consume]]
item_id = 5589
count = 3
```
- `item_id` (integer): Required item ID consumed at cast completion.
- `count` (integer or per-level array of integers): Quantity consumed.

### 3.10 Effects List
Ordered list of effect handlers:
```toml
[[effects]]
section = "effects"
name = "PhysicalAttack"
params = { power = ["30", "32", "35", "46", "51", "55", "71", "78", "84"] }
stat_children = []

[[effects]]
section = "effects"
name = "Buff"
params = {}
stat_children = [
  { op = "mul", stat = "pDef", value = ["1.08", "1.12", "1.15"] },
]

[[effects]]
section = "effects"
name = "TickHp"
params = { mode = "DIFF", power = ["-25", "-34", "-44", "-54", "-62", "-67"], ticks = "5" }
stat_children = []
```
- `section` (string): Root effect section (`"effects"`, `"selfEffects"`, `"endEffects"`).
- `name` (string): Effect handler name.
- `params` (table): Key-value parameters with values represented as exact strings (scalar string or array of exact strings). All source parameters (e.g. `heal = "true"` on `MaxHp`, `type = "PER"`, etc.) are preserved verbatim.
  - Periodic effects (`TickHp`, `TickMp`, and any handler with a `ticks` attribute) record `ticks` verbatim and the calculation mode (`DIFF` or `PER`) verbatim (defaulting to `DIFF` per `TickHp.java:45` and `TickMp.java:41` if omitted from XML).
  - Periodic interval: `interval_ms = ticks × EffectTickRatio` (where `EffectTickRatio = 666` ms from `config/character.properties:145`) per `BuffInfo.java:272` (`ThreadPoolManager.getInstance().scheduleEffectAtFixedRate(effectTask, effect.getTicks() * character().getEffectTickRatio(), effect.getTicks() * character().getEffectTickRatio())`).
  - Amount multiplier: `getTicksMultiplier() = (ticks × EffectTickRatio) / 1000f` per `AbstractEffect.java:148-149`.
  - Mode: `DIFF` scales power by `getTicksMultiplier()`; `PER` reads dynamic current HP/MP at each pulse and scales by `current × power × getTicksMultiplier()`.
  - `TickHp` non-lethal clamp: DoT damage is clamped to `currentHp - 1` (`TickHp.java:75-76`), leaving at least 1 HP (zero damage if `currentHp <= 1`).
- `stat_children` (array of tables): Modifier functions installed by the effect:
  - `op` (string): `"mul"`, `"add"`, `"sub"` are executable; all other imported operations are deferred.
  - `stat` (string): Target stat identifier (e.g. `"pDef"`, `"runSpd"`).
  - `value` (array of exact decimal strings, length `max_level`): Per-level modifier values.
  - `order` (integer or per-level integer array, optional): Explicit priority order override (default: SET=0, base=1, MUL/DIV=20, ADD/SUB=30). Note: `MaxHp`, `MaxMp`, and `MaxCp` handlers install at order 1 per source.

## 4. Capability Rules & Executability Whitelist

A skill is marked `runtime_status = "executable"` if and only if **all** of the following requirements are met:
1. **Operate Type:** In `{"A1", "A2", "A3", "P"}`. Toggles (`T`) and channeled (`CA`) are deferred.
2. **Target Type:** In `{"SELF", "TARGET", "ENEMY", "ENEMY_ONLY", "PC_BODY"}`.
3. **Affect Scope:** In `{"SINGLE", "POINT_BLANK", "RANGE", "unset"}`. Party (`PARTY`) and fan (`FAN`) are deferred.
4. **Item Consumption:** None. Skills requiring item consumption are deferred.
5. **Conditions:** Every condition leaf must be `using_kind` or `player_can_resurrect`. Any other condition defers the skill.
6. **Effect Sections:** Only standard `"effects"` section is supported. `"selfEffects"` or `"endEffects"` defer the skill.
7. **Effect Handlers (Tier B without TargetMe and TickHpFatal):**
   `PhysicalAttack`, `MagicalAttack`, `MagicalAttackRange`, `Stun`, `Sleep`, `Root`, `Buff`, `Debuff`, `TickHp`, `TickMp`, `Heal`, `HpPerMax`, `MaxHp`, `MaxMp`, `Mp`, `Cp`, `Resurrection`, `GetAgro`, `AddHate`, `DispelBySlot`, `DispelByCategory`, `DefenceTrait`, `AttackTrait`, `Passive`.
   *(Notes:*
   *- `TargetMe` is deferred with `effect:TargetMe` (forced target selection on players not implemented in Phase 3).*
   *- `TickHpFatal` is deferred with `effect:TickHpFatal` (lethal DoT excluded; TickHp non-lethal clamp applies).*
   *- `GetAgro` is executable: represents a forced NPC intention, not hate, per `GetAgro.java:49-51` (`info.getEffected().getAI().setIntention(CtrlIntention.AI_INTENTION_ATTACK, info.getEffector())`).)*
8. **Stat Modifiers Whitelist:**
   `pAtk`, `pDef`, `mAtk`, `mDef`, `pAtkSpd`, `mAtkSpd`, `runSpd`, `maxHp`, `maxMp`, `maxCp`, `accCombat`, `rEvas`, `critRate`, `critDmg`, `critDmgAdd`, `regHp`, `regMp`, `regCp`, `physicalMpConsumeRate`, `magicalMpConsumeRate`, `danceMpConsumeRate`, `pReuse`, `mReuse`, `pSkillEvas`, `darkRes`, `darkPower`, `earthRes`, `earthPower`, `fireRes`, `firePower`, `holyRes`, `holyPower`, `waterRes`, `waterPower`, `windRes`, `windPower`, `debuffVuln`, `cancelVuln`.
   Any unwhitelisted stat child produces blocker `stat:<name>`; unsupported operations produce `op:<name>`.
9. **Transitive References:** Any referenced skill (e.g. via `skillId`) must itself be executable.

## 5. Sample Skills

The following 10 sample skills are generated as reference implementations of the schema:
- `3.toml` — Power Strike (Physical attack, weapon requirement, instant damage)
- `100.toml` — Stun Attack (Physical attack + Stun debuff, abnormal visuals, save vs CON)
- `1016.toml` — Resurrection (PC_BODY target, player_can_resurrect condition)
- `1040.toml` — Shield (P.Def multiplier buff)
- `1069.toml` — Sleep (Mental debuff, trait SLEEP defence)
- `1177.toml` — Wind Strike (Magical range attack, WIND attribute power)
- `1204.toml` — Wind Walk (Speed additive buff)
- `1015.toml` — Battle Heal (Instant HP restoration)
- `5076.toml` — Death Penalty (Passive mult-stat penalty across 15 levels)
- `286.toml` — Provoke (AoE pole trait debuff, marked `sample_only = true`)

## 6. Admission and source defaults

All XML table references in skill sets, effect parameters, and stat values expand to exactly `max_level` rows. Numeric fields remain scalars when literal and become rank arrays when table-backed; resources and abnormal level/time always use rank arrays. Missing/short tables and malformed integers are generator errors, never zero substitutions. `source_sets` retains every source `<set>` as exact strings or expanded rank arrays, including fields outside the executable slice.

Source defaults: `magic_level = [0,…]` (Skill.java:273), `cast_range = -1`, `effect_range = -1` (212–213), abnormal `level = [0,…]` (214). `reuse_delay_lock` (boolean or rank array) is imported from `reuseDelayLock` (204); locked reuse bypasses reuse modifiers at cast start (L2Character.java:1517–1530). `hit_cancel_ms` and `cool_time_ms` are imported but unused.

The generator admits only known parameter names and values, expanding table-backed values before validation. Unknown names or values produce `param:<effect>.<name>`. The manifest follows the pinned datapack handler constructors:

| Handler | Parameters / admitted values | Source lines |
|---|---|---|
| PhysicalAttack | power (decimal), criticalChance (integer), ignoreShieldDefence (true/false) | instant/PhysicalAttack.java:44–46 |
| MagicalAttack / MagicalAttackRange | power (decimal); range also shieldDefensePercent (decimal) | instant/MagicalAttack.java:43; MagicalAttackRange.java:50–52 |
| TickHp / TickMp | power (decimal), ticks (positive integer), mode (DIFF/PER) | ticks/TickHp.java:43–45; TickMp.java:40–42 |
| Heal / HpPerMax / Resurrection / AddHate | power (decimal for Heal/AddHate, integer for HpPerMax/Resurrection) | instant/Heal.java:46; HpPerMax.java:40; Resurrection.java:40; AddHate.java:37 |
| MaxHp / MaxMp | power (decimal), type (DIFF/PER), heal (true/false) | pump/MaxHp.java:50–60; MaxMp.java:51–61 |
| Mp / Cp | amount (decimal), mode (DIFF/PER) | instant/Mp.java:44–45; Cp.java:43–44 |
| DispelByCategory | slot (ALL/BUFF/DEBUFF), rate/max (integer) | instant/DispelByCategory.java:43–45; game enums/DispelCategory.java:25–27 |
| DispelBySlot | dispel (`slot[,integer]` entries separated by `;`); supported slots BLEEDING/POISON/PARALYZE/ROOT_PHYSICALLY/TURN_STONE | instant/DispelBySlot.java:47–60 |
| DefenceTrait / AttackTrait | TraitType names, finite decimal values | pump/DefenceTrait.java:52–55; AttackTrait.java:51–54; game model/stats/TraitType.java:26–74 |
| Buff / Debuff / Passive / Stun / Sleep / Root / GetAgro | no parameters | respective pinned handler constructors |

Scope filters admit ALL (also the source default when absent) and NOT_FRIEND. FRIEND stays deferred until the engine implements its predicate. INVISIBLE (stealth requirement), UNDEAD_REAL_ENEMY (race requirement), and unknown filters produce `scope_filter:<name>` and defer the skill. Unknown condition leaves anywhere defer with `cond:<tag>:<attribute names>`. `div/set/share` are preserved but deferred. Referenced-skill blockers propagate to a fixed point.

Every generated skill is parsed with `tomllib` and checked for structural equality before write or `--check`. Corpus tests load all 389 committed definitions, verify all table-backed source sets, and recheck every executable effect/stat condition against the manifest.

Lifecycle fields use pinned Skill.java semantics: `stay_after_death=false` (231), `stay_on_subclass_change=true` (232), `irreplaceable_buff=false` (300), and `abnormal_instant=false` (228) unless overridden by XML. `removed_on_damage` is derived from abnormal types SLEEP/FORCE_MEDITATION/HIDE (486–487); `removed_on_action_except_move` from INVINCIBILITY/HIDE (479–480). These are scalars or rank arrays when table-backed. Complete source sets remain available for auditing execution-affecting fields; consumers must not treat an unimplemented source requirement as implemented merely because it was retained.

TickHp oracle evidence explicitly separates `source_fractional_result` (pinned binary64 state arithmetic) from `rounded_result` and `pulse_trace` (D-R12 whole damage and Q restoration). Binary32 multiplier rounding uses an integer nearest/ties-even significand adapter, independently checked against Python's standard-library `struct` IEEE encoding in tests. For ticks=5 the exact multiplier is `1745879/524288`; power100 restores332 whole HP with999992 Q remainder. DoT whole conversion occurs before CP/HP subtraction; restoration/regen fractions persist until death or full clamp.
