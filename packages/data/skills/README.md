# Nightfall Skill Definitions (Phase 3)

These are the canonical skill definitions, one file for each skill. The generator makes them from the pinned Lineage II High Five datapack.

## 1. Pinned Source Provenance

The script `packages/data/scripts/gen_skills.py` extracts all skill definitions from the pinned L2J High Five source repositories:

| Repository | Revision | Role |
|---|---|---|
| `https://bitbucket.org/l2jserver/l2j-server-datapack` | `3ca488dd2bd0bfaca43e378886a3c2e37968153a` | Skill definitions (`data/stats/skills/*.xml`), skill trees (`data/skillTrees/classSkillTree.xml`) |
| `https://bitbucket.org/l2jserver/l2j-server-game` | `abfde0490ac52a2106e884b7242c0276c44cdf76` | Combat formulas (`Formulas.java`), the logic that does a skill (`Skill.java`, `L2Character.java`) |

## 2. Design Principles & Numeric Representation

- **One file for each skill:** The file is at `packages/data/skills/<id>.toml`, where `<id>` is the integer skill ID.
- **No floats:** All numbers in skill definitions are integers or exact decimal strings (for example `"1.08"`, `"0.91"`, `"0.772184315"`). The generator does not serialize binary floating point types. The independent oracle models each place where the source uses IEEE rounding.
- **Deterministic array length:** Each per-level array has a length that is always equal to `max_level`. These arrays are `mp_consume_start`, `mp_consume_end`, `hp_consume`, `magic_level`, `effect_point`, abnormal `level`, abnormal `time_s`, and stat child `value`.
- **Calculated runtime status:** Only the generator calculates `runtime_status`. Do not change it manually. A skill is `"executable"` only if its `blockers` array is empty. If the array is not empty, `runtime_status` is `"deferred"`, and `blockers` records all the capabilities that block the skill.

## 3. TOML Schema Reference

Each skill TOML file has this structure:

### 3.1 Provenance Header
```toml
provenance = { repository = "https://bitbucket.org/l2jserver/l2j-server-datapack", revision = "3ca488dd2bd0bfaca43e378886a3c2e37968153a", file = "src/main/resources/data/stats/skills/00000-00099.xml", skill_id = 3 }
```
- `repository` (string): The URI of the datapack git repository.
- `revision` (string): The commit hash, with 40 characters.
- `file` (string): The path to the XML source file, relative to the repository.
- `skill_id` (integer): The source skill ID.

### 3.2 Identification & Operational Type
```toml
id = 3
key = "l2.skill.3"
l2_ref = "Power Strike"
max_level = 9
operate_type = "A1"
is_magic = 0
```
- `id` (integer): The skill ID.
- `key` (string): The skill identifier. It is unique in all of the game (`l2.skill.<id>`).
- `l2_ref` (string): The source skill name, with no change.
- `max_level` (integer): The maximum level of the skill without enchant.
- `operate_type` (string): The source operate type (`"A1"`: active instant, `"A2"`: active instant + continuous, `"A3"`: active continuous buff/debuff, `"P"`: passive).
- `is_magic` (integer): The magic class (`0`: physical, `1`: magical, `2`: static / special / dance / song).

### 3.3 Targeting & Affect Scope
```toml
target_type = "ENEMY"
affect_scope = "SINGLE"
affect_range = 0
affect_limit = { offset = 10, width = 10 }
affect_object = ""
```
- `target_type` (string or array of strings): The primary target condition (`"SELF"`, `"TARGET"`, `"ENEMY"`, `"ENEMY_ONLY"`, `"PC_BODY"`). If the target type changes between levels through source tables, the field is an array with the length `max_level`.
- `affect_scope` (string): The scope of the area of effect (`"SINGLE"`, `"POINT_BLANK"`, `"RANGE"`, or `"unset"`).
- `affect_range` (integer, optional): The radius of the affect scope in world units (a scalar or a per-level array).
- `affect_limit` (table, optional): The structured target limit `{ offset = N, width = M }`, as in `Skill.getAffectLimit` (`Skill.java:637-641`). The runtime calculates the limit as follows:
  - If width is 0, there is no draw and NO limit (unlimited).
  - If width is more than 0, the limit is offset + Rnd.get(width).

  Thus `Skill.java:638-640` examines `if (affectLimit[1] == 0) return 0;` (width is `affectLimit[1]`). When `getAffectLimit()` returns 0, `AffectScope.java` uses limit 0 as UNLIMITED. The target count loops stop only with `if ((affectLimit > 0) && (targets.size() >= affectLimit))` (`AffectScope.java:73-75, 143-145, 182-184, 233-235, 287-289, 357-359`). The stream scopes use `.limit(affectLimit > 0 ? affectLimit : Integer.MAX_VALUE)` (`AffectScope.java:386, 401, 417`). The check is true only if `affectLimit > 0`. Thus, a return value of 0 means that there is no limit (unlimited targets).
- `affect_object` (string, optional): The target filter in the scope (for example `"NOT_FRIEND"`).

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
- `cast_range` (integer or per-level array): The maximum range in world units to start the cast.
- `effect_range` (integer or per-level array): The maximum range in world units when the cast is complete.
- `hit_time_ms` (integer or per-level array): The base time of the cast animation in milliseconds, before the attack/cast speed scales it.
- `cool_time_ms` (integer or per-level array): The imported `coolTime` in milliseconds. The pinned execution path does not use it (Skill.java:633; L2Character.java:4666,4690).
- `reuse_ms` (integer or per-level array): The base reuse cooldown delay in milliseconds.
- `hit_cancel_ms` (integer): The source attribute for the window in which damage interrupts the cast. It is imported, but the engine does not use it (D-R2).
- `mp_consume_start` (array of integers, length `max_level`): The MP that the cast removes when it starts (`mpConsume1`).
- `mp_consume_end` (array of integers, length `max_level`): The MP that the cast removes when it is complete (`mpConsume2`).
- `hp_consume` (array of integers, length `max_level`): The HP that the cast removes when it is complete (`hpConsume`).
- `magic_level` (array of integers, length `max_level`): The effective magic level for the level-difference and success calculations.
- `effect_point` (array of integers, length `max_level`): The aggro / hate points that the skill makes (negative for offensive skills).

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
- `activate_rate` (integer): The base land chance percentage of a debuff (-1 if there is no chance or no condition).
- `basic_property` (string): The base attribute of the saving throw (`"STR"`, `"CON"`, `"DEX"`, `"INT"`, `"MEN"`, `"WIT"`, `"NONE"`).
- `level_bonus_rate` (integer): The multiplier for the level-difference modifier in `calcEffectSuccess`.
- `trait` (string): The trait class (`"SHOCK"`, `"SLEEP"`, `"HOLD"`, `"PARALYZE"`, `"POISON"`, `"BLEED"`, `"DERANGEMENT"`, `"NONE"`).
- `min_chance` / `max_chance` (integer): The lower and upper clamps for the land rate of a debuff. The pinned defaults are 10 and 90 (character.properties:277–278), and Skill.java:276–277 reads them.
- `next_action_attack` (boolean): This field shows if the player automatically starts auto-attack after the skill lands.
- `has_enchant_routes` (boolean): This field shows if the source data has enchant skill routes. The Phase 3 runtime does not include enchant data.
- `runtime_status` (string): `"executable"` if `blockers` is empty; if not, `"deferred"`.
- `blockers` (array of strings): A list of the identifiers of the capabilities that block the skill, in lexicographic sort order.

### 3.6 Elemental Attribute Table
```toml
[attribute]
type = "NONE"
power = 0
```
- `type` (string): The element type of the attack (`"FIRE"`, `"WATER"`, `"WIND"`, `"EARTH"`, `"HOLY"`, `"UNHOLY"`, `"NONE"`).
- `power` (integer): The elemental attack power of the skill. The attribute bonus calculation adds it.

### 3.7 Abnormal Status Table (Optional)
This table is there only when the skill applies a continuous abnormal state (buff, debuff, toggle):
```toml
[abnormal]
type = "STUN"
level = [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]
time_s = [9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9]
is_debuff = true
visual = "STUN"
```
- `type` (string): The abnormal slot/stack type (for example `"STUN"`, `"SPEED_UP"`, `"PD_UP"`).
- `level` (array of integers, length `max_level`): The abnormal intensity level. It sets the precedence when effects stack.
- `time_s` (array of integers, length `max_level`): The base duration in seconds.
- `is_debuff` (boolean): True for harmful continuous effects. These effects use debuff slots and use the debuff land roll.
- `visual` (string, optional): The identifier of the client visual effect, if the source data has one.

### 3.8 Conditions Tree
The conditions have the structure of a normalized recursive tree (`conditions`). The source `<cond>` XML nodes are kept with no change next to the tree (`raw_conditions`):
```toml
raw_conditions = ["<cond><using kind=\"SWORD,BLUNT\" /></cond>"]

[[conditions]]
kind = "and"

[[conditions.conditions]]
kind = "using_kind"
weapon_kinds = ["SWORD", "BLUNT"]
```

The supported condition kinds are:
- `{ kind = "using_kind", weapon_kinds = ["SWORD", "BLUNT"] }`: The runtime validates this against the equipped weapon.
- `{ kind = "player_can_resurrect", value = true }`: A state check that the dead target is valid for resurrection.
- Logical operators: `{ kind = "and", conditions = [...] }`, `{ kind = "or", conditions = [...] }`, `{ kind = "not", conditions = [...] }`.
- Unsupported conditions, kept with no change: `{ kind = "unsupported", tag = "player", attrs = { Charges = "3" } }`.
- Recursive conditions serialize as TOML arrays of tables (`[[conditions]]`, `[[conditions.conditions]]`, and other levels). Unsupported `attrs` use the TOML `=` and keep the attribute strings with no change.
- Effects and stat children also have the same `conditions` / `raw_conditions` fields. A stat condition can be a bare `<using>`, `<player>`, or boolean node (DocumentBase.java:246), or a `<cond>` wrapper. Each effect uses `section_conditions` / `section_raw_conditions` for the conditions that attach to a section (DocumentBase.java:201–223).
- `raw_conditions` keeps the parsed XML nodes, with attributes such as `msgId` and `addName`. The XML indentation is normalized, and ElementTree can normalize the spelling of entities. Thus the XML semantics are kept, but not the bytes of the original file.

### 3.9 Item Consumption
```toml
[[item_consume]]
item_id = 5589
count = 3
```
- `item_id` (integer): The ID of the necessary item that the cast uses when it is complete.
- `count` (integer or per-level array of integers): The quantity that the cast uses.

### 3.10 Effects List
This is the ordered list of effect handlers:
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
- `section` (string): The root effect section (`"effects"`, `"selfEffects"`, `"endEffects"`).
- `name` (string): The name of the effect handler.
- `params` (table): Key-value parameters. Each value is an exact string (a scalar string or an array of exact strings). The generator keeps all source parameters with no change (for example `heal = "true"` on `MaxHp`, `type = "PER"`, and other parameters).
  - Periodic effects (`TickHp`, `TickMp`, and all handlers with a `ticks` attribute) record `ticks` and the calculation mode (`DIFF` or `PER`) with no change. If the XML does not give the mode, the default is `DIFF` (`TickHp.java:45` and `TickMp.java:41`).
  - Periodic interval: `interval_ms = ticks × EffectTickRatio`, where `EffectTickRatio = 666` ms (from `config/character.properties:145`). The source is `BuffInfo.java:272` (`ThreadPoolManager.getInstance().scheduleEffectAtFixedRate(effectTask, effect.getTicks() * character().getEffectTickRatio(), effect.getTicks() * character().getEffectTickRatio())`).
  - Amount multiplier: `getTicksMultiplier() = (ticks × EffectTickRatio) / 1000f`, from `AbstractEffect.java:148-149`.
  - Mode: `DIFF` multiplies the power by `getTicksMultiplier()`. `PER` reads the current HP/MP at each pulse and calculates `current × power × getTicksMultiplier()`.
  - Non-lethal clamp of `TickHp`: The clamp sets the maximum DoT damage to `currentHp - 1` (`TickHp.java:75-76`). Thus a minimum of 1 HP stays (the damage is zero if `currentHp <= 1`).
- `stat_children` (array of tables): The modifier functions that the effect installs:
  - `op` (string): `"mul"`, `"add"`, `"sub"` are executable. All other imported operations are deferred.
  - `stat` (string): The identifier of the target stat (for example `"pDef"`, `"runSpd"`).
  - `value` (array of exact decimal strings, length `max_level`): The modifier values for each level.
  - `order` (integer or per-level integer array, optional): An explicit override of the priority order (default: SET=0, base=1, MUL/DIV=20, ADD/SUB=30). Note: The source installs the `MaxHp`, `MaxMp`, and `MaxCp` handlers at order 1.

## 4. Capability Rules & Executability Whitelist

A skill gets `runtime_status = "executable"` only if it agrees with **all** of these rules:
1. **Operate Type:** In `{"A1", "A2", "A3", "P"}`. Toggles (`T`) and channeled skills (`CA`) are deferred.
2. **Target Type:** In `{"SELF", "TARGET", "ENEMY", "ENEMY_ONLY", "PC_BODY"}`.
3. **Affect Scope:** In `{"SINGLE", "POINT_BLANK", "RANGE", "unset"}`. Party (`PARTY`) and fan (`FAN`) are deferred.
4. **Item Consumption:** None. If a skill uses items, it is deferred.
5. **Conditions:** Each condition leaf must be `using_kind` or `player_can_resurrect`. All other conditions defer the skill.
6. **Effect Sections:** Only the standard `"effects"` section is supported. `"selfEffects"` or `"endEffects"` defer the skill.
7. **Effect Handlers (Tier B without TargetMe and TickHpFatal):**
   `PhysicalAttack`, `MagicalAttack`, `MagicalAttackRange`, `Stun`, `Sleep`, `Root`, `Buff`, `Debuff`, `TickHp`, `TickMp`, `Heal`, `HpPerMax`, `MaxHp`, `MaxMp`, `Mp`, `Cp`, `Resurrection`, `GetAgro`, `AddHate`, `DispelBySlot`, `DispelByCategory`, `DefenceTrait`, `AttackTrait`, `Passive`.
   *(Notes:*
   *- `TargetMe` is deferred with `effect:TargetMe` (Phase 3 does not implement a forced target selection on players).*
   *- `TickHpFatal` is deferred with `effect:TickHpFatal` (lethal DoT is not included; the TickHp non-lethal clamp applies).*
   *- `GetAgro` is executable. It is a forced NPC intention, not hate, as in `GetAgro.java:49-51` (`info.getEffected().getAI().setIntention(CtrlIntention.AI_INTENTION_ATTACK, info.getEffector())`).)*
8. **Stat Modifiers Whitelist:**
   `pAtk`, `pDef`, `mAtk`, `mDef`, `pAtkSpd`, `mAtkSpd`, `runSpd`, `maxHp`, `maxMp`, `maxCp`, `accCombat`, `rEvas`, `critRate`, `critDmg`, `critDmgAdd`, `regHp`, `regMp`, `regCp`, `physicalMpConsumeRate`, `magicalMpConsumeRate`, `danceMpConsumeRate`, `pReuse`, `mReuse`, `pSkillEvas`, `darkRes`, `darkPower`, `earthRes`, `earthPower`, `fireRes`, `firePower`, `holyRes`, `holyPower`, `waterRes`, `waterPower`, `windRes`, `windPower`, `debuffVuln`, `cancelVuln`.
   A stat child that is not on the whitelist makes the blocker `stat:<name>`. An unsupported operation makes `op:<name>`.
9. **Transitive References:** Each skill that the skill refers to (for example through `skillId`) must also be executable.

## 5. Sample Skills

The generator makes these 10 sample skills as reference implementations of the schema:
- `3.toml` — Power Strike (Physical attack, weapon condition, instant damage)
- `100.toml` — Stun Attack (Physical attack + Stun debuff, abnormal visuals, save against CON)
- `1016.toml` — Resurrection (PC_BODY target, player_can_resurrect condition)
- `1040.toml` — Shield (P.Def multiplier buff)
- `1069.toml` — Sleep (Mental debuff, trait SLEEP defence)
- `1177.toml` — Wind Strike (Magical range attack, WIND attribute power)
- `1204.toml` — Wind Walk (Speed additive buff)
- `1015.toml` — Battle Heal (Instant HP restoration)
- `5076.toml` — Death Penalty (Passive multi-stat penalty for 15 levels)
- `286.toml` — Provoke (AoE pole trait debuff, with the mark `sample_only = true`)

## 6. Admission and source defaults

Each XML table reference in skill sets, effect parameters, and stat values expands to exactly `max_level` rows. A numeric field stays a scalar if it is a literal. It becomes a rank array if a table gives it. Resources and abnormal level/time always use rank arrays. If a table is missing or short, or an integer is malformed, the generator gives an error. It never puts zero in its place. `source_sets` keeps each source `<set>` as exact strings or expanded rank arrays. This includes the fields that are not in the executable slice.

Source defaults: `magic_level = [0,…]` (Skill.java:273), `cast_range = -1`, `effect_range = -1` (212–213), abnormal `level = [0,…]` (214). The generator imports `reuse_delay_lock` (boolean or rank array) from `reuseDelayLock` (204). A locked reuse does not use the reuse modifiers at the start of the cast (L2Character.java:1517–1530). The generator imports `hit_cancel_ms` and `cool_time_ms`, but the engine does not use them.

The generator admits only known parameter names and values. It expands the values from tables before validation. Unknown names or values make `param:<effect>.<name>`. The manifest agrees with the pinned datapack handler constructors:

| Handler | Parameters / admitted values | Source lines |
|---|---|---|
| PhysicalAttack | power (decimal), criticalChance (integer), ignoreShieldDefence (true/false) | instant/PhysicalAttack.java:44–46 |
| MagicalAttack / MagicalAttackRange | power (decimal); range also has shieldDefensePercent (decimal) | instant/MagicalAttack.java:43; MagicalAttackRange.java:50–52 |
| TickHp / TickMp | power (decimal), ticks (positive integer), mode (DIFF/PER) | ticks/TickHp.java:43–45; TickMp.java:40–42 |
| Heal / HpPerMax / Resurrection / AddHate | power (decimal for Heal/AddHate, integer for HpPerMax/Resurrection) | instant/Heal.java:46; HpPerMax.java:40; Resurrection.java:40; AddHate.java:37 |
| MaxHp / MaxMp | power (decimal), type (DIFF/PER), heal (true/false) | pump/MaxHp.java:50–60; MaxMp.java:51–61 |
| Mp / Cp | amount (decimal), mode (DIFF/PER) | instant/Mp.java:44–45; Cp.java:43–44 |
| DispelByCategory | slot (ALL/BUFF/DEBUFF), rate/max (integer) | instant/DispelByCategory.java:43–45; game enums/DispelCategory.java:25–27 |
| DispelBySlot | dispel (`slot[,integer]` entries with `;` between them); supported slots BLEEDING/POISON/PARALYZE/ROOT_PHYSICALLY/TURN_STONE | instant/DispelBySlot.java:47–60 |
| DefenceTrait / AttackTrait | TraitType names, finite decimal values | pump/DefenceTrait.java:52–55; AttackTrait.java:51–54; game model/stats/TraitType.java:26–74 |
| Buff / Debuff / Passive / Stun / Sleep / Root / GetAgro | no parameters | the applicable pinned handler constructors |

Scope filters admit ALL (also the source default if there is no filter) and NOT_FRIEND. FRIEND stays deferred until the engine implements its predicate. INVISIBLE (stealth condition), UNDEAD_REAL_ENEMY (race condition), and unknown filters make `scope_filter:<name>` and defer the skill. An unknown condition leaf at any position defers the skill with `cond:<tag>:<attribute names>`. The generator keeps `div/set/share`, but they are deferred. Blockers from referenced skills propagate until they reach a fixed point.

The generator parses each skill it makes with `tomllib` and examines it for structural equality before it writes the file or runs `--check`. The corpus tests load all 389 committed definitions. They also examine all source sets that tables give, and examine each executable effect/stat condition against the manifest again.

The lifecycle fields use the pinned Skill.java semantics, unless the XML overrides them: `stay_after_death=false` (231), `stay_on_subclass_change=true` (232), `irreplaceable_buff=false` (300), and `abnormal_instant=false` (228). The generator calculates `removed_on_damage` from the abnormal types SLEEP/FORCE_MEDITATION/HIDE (486–487). It calculates `removed_on_action_except_move` from INVINCIBILITY/HIDE (479–480). These fields are scalars, or rank arrays if a table gives them. The full source sets stay available, so you can audit the fields that have an effect on execution. A consumer must not think that a source condition is implemented only because the data keeps it.

The TickHp oracle evidence keeps `source_fractional_result` (pinned binary64 state arithmetic) separate from `rounded_result` and `pulse_trace` (D-R12 whole damage and Q restoration). The binary32 multiplier rounding uses an integer nearest/ties-even significand adapter. The tests examine this adapter independently against the IEEE encoding of `struct` from the Python standard library. For ticks=5, the exact multiplier is `1745879/524288`. For power100, the effect restores332 whole HP with999992 Q remainder. The whole conversion of DoT occurs before the CP/HP subtraction. The restoration/regen fractions stay until death or a full clamp.
