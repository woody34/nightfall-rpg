# Phase 5 scenario, pack and sidecar contract (story 1.8 → 3.15 / 5.6 / 6.1 / 6.6)

This file records what story 1.8 authored. Story 6.1 owns the parser, the fixture dispatcher
and the seeders. Story 5.6 owns the bot. Story 6.6 runs the files live. Nothing in this file is a
pass. The plan §2.8 table is the source of each header, and the design §13 is the source of each
carrier.

## Files

| Path | Content | Owner |
|------|---------|-------|
| `Scenarios/5-*.nfs` (36) | One file for each plan §2.8 row. Line 1 is the exact `# covers:` header. Line 2 is `# fixture: <pack>`. | 1.8 authors; 3.15 adds server sections; 6.6 runs |
| `Scenarios/5-*.expect.json` (36) | The oracle expectation sidecar of one file. Every number comes from `economy_oracle.py` (stories 1.5–1.7) or from `tables-phase5/`. | `phase5-expectations.py` |
| `Scenarios/fixtures/phase5-packs.json` | The 27 plan §5.3 pack declarations (E46): roles, levels, positions, inventories, recipes, party, container rows, NPC placements, fixture schedule, `next_item_instance`. | `phase5-expectations.py`; 6.1 extends with harness fields |
| `Scripts/phase5-expectations.py` | Generates both. `--check` fails on a stale file. | C lane |
| `Scripts/test/phase5-scenarios.test.py` | Static checks: headers, packs, vocabulary, gate labels, budgets, role blocks, generator `--check`. | C lane |

Regenerate after a table or declaration change:

```bash
python3 apps/client-unreal/Scripts/phase5-expectations.py
python3 apps/client-unreal/Scripts/phase5-expectations.py --check
python3 -m pytest apps/client-unreal/Scripts/test/phase5-scenarios.test.py
```

## Role blocks

A file with more than one role has `# role: <name>` lines. The commands after a role line belong
to that pack role until the next role line. A file without a role line is role `a`. The runner
starts one client for each role (the Phase 3 `run-sim-multi.sh` pattern). Role names are `a`, `b`
and `c`, and each must exist in the pack. The budget `nf.Within` is the largest role's sum of
`nf.WaitFor` timeouts + 30 s.

## Gate labels

`# econ-gate: <name>` binds a file to a design §14.3 export fact that the story 6.2 consumer must
check for that run. The names are the `econ_formula` formulas, the `econ_chance` callsites, the
`econ_select` callsite and the new `item_rng` callsites. A label is never a pass by itself.

## Argument forms used by the 36 files

Story 6.1's parser is the authority for the final grammar. The files use these forms. Change the
files and this table together.

| Command | Form used | Carrier |
|---------|-----------|---------|
| `nf.PickUp` | `template:<id>` (the nearest received ground item of that template) or `nearest` | EntityDespawn + EconomyResult(pickup) |
| `nf.Buy` / `nf.Sell` / `nf.Refund` | `merchant <list_id> <template|template:<id>> <count>`; `Refund merchant <list_id> <index>` | EconomyResult(vendor) |
| `nf.Multisell` | `exchanger <list_id> <entry_index> <amount> [enchant:<n>]` | EconomyResult(multisell) |
| `nf.RegisterRecipe` / `nf.Craft` | `template:<recipe item>`; `Craft <recipe_id>` | RecipeBookSync / EconomyResult(recipe, craft) |
| `nf.ManufactureOrder` | `<owner role> <recipe_id>` | EconomyResult(craft) for both |
| `nf.TradeRequest` / `nf.TradeAnswer` / `nf.TradeAdd` / `nf.TradeConfirm` | `<role>`; `accept|decline`; `template:<id> <count>` | TradeRequestReceived / TradeSessionChanged |
| `nf.StoreSell` / `nf.StoreBuy` / `nf.StoreManufacture` | `<title> [package] <entries>` with `template:<id>:<count>:<price>` (buy: `template:<id>:<enchant>:<count>:<price>`; manufacture: `<recipe>:<fee>`), comma separated | PrivateStoreChanged |
| `nf.StoreBuyFrom` / `nf.StoreSellTo` | `<owner role> <entries>` | EconomyResult(store) |
| `nf.PartyInvite` / `nf.PartyAnswer` / `nf.PartyLootVote` / `nf.PartyLootAnswer` | `<role> [mode]`; `-1|0|1`; `<mode>`; `accept|decline` | PartyStateChanged / PartyInviteReceived / PartyLootVote |
| `nf.Deposit` / `nf.Withdraw` / `nf.FreightSend` / `nf.FreightWithdraw` | `template:<id>:<count>,...`; `Withdraw <template>:<count>`; `FreightSend <role> <entries>` | EconomyResult(container) |
| `nf.MailSend` / `nf.MailTake` / `nf.MailReject` / `nf.MailCancel` / `nf.MailDelete` / `nf.MailRead` | `<role> <subject> [cod <adena>] [entries]`; `latest`; `MailDelete latest sent` | MailNotification / EconomyResult(mail) |
| `nf.AuctionBid` / `nf.AuctionCancel` | `<instance_npc_id> <bid>`; `<instance_npc_id>` | ItemAuctionStatusChanged / EconomyResult(auction) |
| `nf.ManorBuySeed` / `nf.ManorSellCrop` | `manor_manager <manor_id> <seed> <n>`; `manor_manager template:<crop> <crop_id> <manor_id> <count|all>` | EconomyResult(manor) |
| `nf.Target` / `nf.Cast` | `template:<npc id>` or `entity:<role>`; `Cast <skill> target` (Spoil 254, Sweeper 42) | TargetChanged / SkillCastResult / CorpseStateChanged |

Predicates with a `_between <lo> <hi>` suffix compare a received whole integer to an inclusive
range from the sidecar. `adena_delta` compares the received adena count to the `nf.MarkItemState`
sample. `economy_result == <scope>` reads the last received `EconomyResult.operation` by its
`ItemOperation` name in lower case. All predicates are UNKNOWN until their carrier arrives
(C11); UNKNOWN is never allow-listed.

## Pinned inputs in the packs

| Pack input | Pinned source |
|------------|---------------|
| NPC 22320, 20003, 18478, 18352, 18544, 4301 | `tables-phase5/drops.toml` `npc` rows (SP-only 4301 from `stats/npcs/04300-04399.xml`) |
| Merchant 31413 list 314130 item 1294 (ref 75, tax 15) | `merchants.toml` `entries` (Merchant/Antonio/Antonio.java `_sellList0`) |
| Merchant 35363 list 353630 item 16127 (count 10, restock 60 min) | `merchants.toml` `entries` (Chamberlain/Alfred/Alfred.java `_sellList0`) |
| Multisell 616 at NPC 32347 (maintainEnchantment) | `multisell.toml` `list` |
| Recipes 1 (`mk_wooden_arrow`, 100 %), 322 (`q_mk_titans_powerstone`, 25 %), 686 (`mk_lesser_healing_potion`, common) | `recipes.toml` `recipe` |
| Auction instance 32320, auction 1, item 9901, init bid 100,000 | `auction.toml` `instance`, `fixture_schedule` |
| Gludio seed 5016 / crop 5073 | `manor.toml` `fixture_gludio` |
| Fees, slots, flood intervals | `storage_mail.toml` |

No pinned common recipe carries `productionRare`: all 582 rare recipes are Dwarven. The
`5-craft-common-masterwork` file therefore asserts `craft_masterwork == false` on every common
craft (the rarity-0 draw is taken and never passes).
