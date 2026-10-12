# Phase 4 client offline validation

Build `NightfallEditor` with the root Unreal lock, then run:

```bash
export UE_ROOT=/home/matt-woodruff/Linux_Unreal_Engine_5.8.3
export LINUX_MULTIARCH_ROOT=/home/matt-woodruff/UnrealToolchains/v26_clang-20.1.8-rockylinux8
bash apps/client-unreal/Scripts/run-tests.sh --offline Nightfall.Item
bash apps/client-unreal/Scripts/run-tests.sh offline
```

`offline` and `--offline` select all local automation tests, excluding live API
admission/end-to-end tests and the optional vendor-art import test. They do not
acquire or pass authentication credentials. An explicit filter selects a narrower
suite. `--require-live-api` and offline mode conflict.

| E5 story | Offline evidence |
|---|---|
| 5.1 Inventory/catalogue/paperdoll/weight | `Nightfall.Item.Reducer.HydrationRevisionPrivacy`, `Catalog.FailureHashAndLifetime`, `UI.InventoryCountsWeightInput`: owner fences, atomic validation, revision gaps and same-revision summaries, detached sentinel, catalogue failure/hash/lifetime, real input controls and received counts/weight |
| 5.2 Operation dialogs | `Nightfall.Item.UI.EnchantCrystallizeBlacksmithRetry`, `Audit.StickyCoherenceAndReset`, `Receipt.RevivalHistoryIsNotLiveState`: server results, pending/rejection states, exact keyed retry after ingredient consumption, received Blacksmith identity, historical receipt separation |
| 5.3 Hotbar/DSL | `Nightfall.Item.UI.HotbarKeyAndToggle`, `Bot.GrammarUnknownAndReset`, `Bot.AllReceivedCarriers`, `Bot.ReceivedMarksAndStatsFences`, `Bot.RefineAppliedRemovedRevisionJoin`: actual F1 input/config, received-only toggles, exact rational/signed comparisons, UNKNOWN/reset and owner revision joins |
| 5.4 Privacy/presentation | `Nightfall.Item.Codec.IntentsAndEnrichments`, `Codec.AllTenIntents`, `Codec.PublicSpawnAndOwnerStats`, `Presentation.PublicGearEmptyAndGlow`: all intents, public equipment and owner stats transport, mesh/glow selection, present-empty clearing and private-state isolation |
| 5.5 Scenario grammar | `Nightfall.Item.Bot.Phase4ScenarioRegistry`: native parser accepts all 23 canonical scenarios and 26 runtime variants; Python source-registration audit checks the same 49 files |

The item suite contains 16 tests, including the original 13 and three added tests
for revised E6 semantics and native scenario parsing. The complete offline suite
contains 111 tests at this revision. Headless execution verifies widget controls
and mesh state; pixels are a separate gate.

The client follows the current [E6 grammar](phase4-fixtures.md).
`nf.MarkItemState` captures synchronized received inventory revision and owner
sheet/maxima. The comparison mark survives same-character reconnect/incarnation
changes, while current facts stay UNKNOWN until fresh hydration. Leaving or
switching characters clears it. `refine_outcome` and `refine_result_consistent`
use RefineResult32; refine/cancel are excluded from `item_operation`33.

D51 accepts the deterministic silhouettes/materials as cosmetic placeholders.
Live scenarios, required-live automation with zero skips, content generation,
rendering/screenshots, zero-missing-reference cook, D26 live/replayed bytes and
independent review remain integration gates. E5 stories are not marked globally
complete by offline results.
