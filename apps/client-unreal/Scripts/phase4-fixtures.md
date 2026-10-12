# Phase 4 bot and fixture contract (E6 → E5/E3)

`phase4-grammar.py` is the parser-tested argument authority. Commands are case
insensitive; slot/predicate/enum tokens are case sensitive. `<item>` is a decimal
template ID, `template:<id>` or `instance:<positive-bigint>`. Template resolution
uses only the owner's received inventory, selecting the lowest instance ID;
missing baseline/item fails before sending. `entity:<uuid>` resolves a received
entity/incarnation; bare trailing UUID always means request key. Every mutation
accepts optional trailing UUID key; omitted key is fresh per invocation. A retry
with an explicit key reuses the originally resolved instance/target/body (do not
resolve a template anew after destruction/SA). Changed body with same key must
reach the server conflict path. Equip's optional slot maps to preferred_slot.
UseItem defaults to self. Refine/CancelRefine/ExchangeSA default NPC is the current
received target; explicitly target `template:blacksmith` first. GrantItem calls
the authenticated dev RPC for the selected character, never SQL or console GM.

Command forms (optional target is `self|target|entity:<uuid>` for UseItem,
`target|entity:<uuid>` for blacksmith commands):

```
nf.ItemCatalog
nf.Equip <item> [slot] [key]
nf.Unequip <slot> [key]
nf.UseItem <item> [target] [key]
nf.Destroy <item> <count> [key]
nf.Crystallize <item> [key]
nf.Enchant <scroll> <item> [support:<item>] [key]
nf.AutoShot <template> on|off [key]
nf.Refine <item> <stone> [npc-target] [key]
nf.CancelRefine <item> [npc-target] [key]
nf.ExchangeSA <item> <recipe-id> [npc-target] [key]
nf.GrantItem <template> <count> [key]
```

Slots are the lower-case EquipSlot suffixes in items.proto (rhand/lhand for
lrhand weapon; chest for onepiece). Numeric comparisons support == != >= <= > <.
`equipped` supports ==/!=. These are received projections, never predictions:

| Predicate argument form | Carrier / semantics |
|---|---|
| inventory_has template cmp count | InventorySync/Changed; sum all owner instances, including paperdoll |
| equipped slot == template (0 clears) | Owner paperdoll plus instances; public gear cannot establish owner instance state |
| enchant_level slot cmp level | Exact owner item enchant; cleared slot is UNKNOWN |
| weight_pct cmp decimal-or-ratio | Exact 100*weight/max_load cross multiplication, never rounded display/per_mille; max_load<=0 UNKNOWN |
| overload_level cmp rank | Owner PenaltyChanged/sync, not computed from weight |
| grade_penalty weapon\|armor cmp rank | Owner penalty fields |
| autoshot_on template | Received owner auto-shot set; charge/damage cannot imply toggle |
| arrows_remaining cmp count | Received owner lhand ARROW count, catalogue identifies arrow; empty slot is 0 after baseline |
| inventory_revision cmp revision; item_baseline | Owner revision and accepted live baseline |
| item_catalog_ready | Authenticated GetItemCatalog version2 and validated content |
| item_operation == operation | Last received keyed ItemOperationResult, operation names in grammar |
| enchant_outcome == outcome | Last received EnchantResult, normalized enum suffix |
| enchant_result_consistent | Join received result to received owner post-revision item state: success+1, no-increment unchanged, safe unchanged, blessed0, destroyed absent; refunds join received counts. No RNG/formula computation |
| refined item | Received RefineResult + owner item has augment_id |
| item_rpc_error == status | Last completed item RPC status; none only after a successful RPC, not reset default |
| item_receipt_matches_first | Exact received keyed result bytes equal first result in that operation/account scope |

UNKNOWN until appropriate carrier/catalogue/baseline arrives; UNKNOWN is never
allow-listed. Reset on account/character/session generation change. New incarnation
invalidates inventory, receipts, last results, weight and penalty until identity
then baseline. Tick/baseline/revision gaps fail closed and request resync; accept
same-revision weight/penalty/stats siblings without dropping them. Public-only
observer messages never hydrate private inventory. Reconnect invalidates transient
results but preserves a scoped first-receipt comparison history for explicit retries.

Fixtures: `phase4-packs.json` publishes exact role inputs and source citations;
counts/enchant/level/SP/position are declared test inputs, not production constants.
All kit extras/equipped flags come from starter_kits.toml; capacity/blacksmith
policy comes from capacities.toml. Item identity uses fixture low block [1,2^32),
per-role disjoint slices and next=max(seed ID)+1; production item_id_ranges starts
at 2^32 (D8). SQL never inserts that low block into the production range table.
Pending E2 admission interface must record the seed allocator and runtime rules
hash; data_sha256 is not the runtime hash. Missing admission support refuses live
execution. Blacksmith is at (124,128), design §7; tax0 is hashed fixture policy.

Live grammar compatibility is separately checked by sim-parser-check.py against
C++ registration. Publication/parser tests cannot attest that E5 has landed.
