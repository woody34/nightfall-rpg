# Phase 1 asset acquisition manifest — E5.1 research

Researched **2026-10-08** for UE **5.8.3**, following [Phase 1 §2 D7, E5.1 and R3](../phase-1-kill-a-monster.md) and the [client README](../../../apps/client-unreal/README.md). This document covers acquisition research and the subsequent implementation plan; it does **not** claim the E5.1 import, repeat-import, editor-load or cook gates have passed.

**Decision:** use the installed Epic Manny template assets for the player; select Animated Cockatrice as the monster baseline; optionally acquire Stylized Low Poly Nature Lite. The cockatrice sacrifices the wolf/keltir appearance preference because its publisher explicitly advertises all four required animation roles. ANIMAL VARIETY PACK is the preferred-looking alternative, but its wolf's exact clip coverage is **UNVERIFIED**. It must not replace the baseline merely because it is free.

**Handoff status: conditional, not a fully validated acquisition gate.** Public Fab pages verified the selected listings' free prices, but exposed blank licence/version/download-size fields. Their applicable licence labels remain **UNVERIFIED**. Monster clip roles are publisher-advertised, not inspected FBX takes; actual UE 5.8 import is **UNVERIFIED**. Resolve applicable terms before downloading, and inspect the files after the one acquisition session. Do not mark R3 or E5.1 complete on this research alone. No purchases, account changes, downloads of vendor content, imports or game-code changes were performed during research.

## Decision table

| Category | Selected option | Reason and compatibility evidence | Acquisition / remaining gate |
| --- | --- | --- | --- |
| Player | **Epic Manny**, installed UE 5.8.3 template mesh/idle/walk/attack/death; installed legacy mannequin run retargeted to Manny | Actual local files cover all five roles. Native UE5 mannequin skeleton except the UE4 run donor. No large sample-project download needed. | Already installed; zero additional download. Editor playback and UE4→UE5 retarget result still require validation. |
| Monster | **[Animated Cockatrice][cockatrice] — Marko J / Marko Jäntti** | Free; publisher explicitly advertises idle, walking, attack and death; FBX and Blender source provide a practical import route on Linux. | Download original FBX, textures and available Blender source in the one session. Licence label, exact take names and UE 5.8 import **UNVERIFIED**. Credit the author. |
| Environment, optional | **[Stylized Low Poly Nature Lite][nature] — JustCreate** | Free; six static models, advertised FBX and Unreal versions. Small scope suitable for decorating the existing test zone. | Prefer original FBX + textures from Fab. Actual bytes, licence label and UE 5.8 import **UNVERIFIED**. Omit if unavailable; it must not block combat. |

The required baseline is deliberately available without a Windows/macOS launcher: Manny is local and the monster advertises downloadable interchange files. The wolf's Unreal-native route is a separate, optional same-session acquisition, not a second mandatory owner task.

## Candidate evidence

### Player: installed Epic templates — selected

- **Publisher / listing:** Epic Games. These are engine-installed templates, not a separate Fab purchase. [Third Person template documentation][third-person]; [Epic Linux engine download][linux-engine]. The documentation describes Manny/Quinn and the Combat variant, but the exact inventory below is **local filesystem evidence**, not a claim that the webpage enumerates these files.
- **Version:** this clone's installed engine at `/home/matt-woodruff/Linux_Unreal_Engine_5.8.3`; `Engine/Build/Build.version` reports 5.8.3, changelist 58210709. Template files exist in that installation. UE editor loading/playback has not been tested in this research.
- **Licence:** [Unreal Engine EULA][ue-eula], not an assumed Fab Standard or Epic Content licence. Section 1 includes installed Samples/Templates content in **Examples**; §5(b) permits distribution of Examples in source or object form. Thus these identified template assets are an exception to the usual prohibition on public raw-vendor redistribution. Shipped-game use is permitted subject to the EULA. Keep them ignored anyway for consistent provenance and to avoid accidentally including unrelated restricted content; do not relicense them as Nightfall code.
- **Skeleton:** Manny/Quinn files reference `/Game/Characters/Mannequins/Meshes/SK_Mannequin`. The legacy run references `/Game/Mannequin/Character/Mesh/UE4_Mannequin_Skeleton`; it needs IK retargeting, not reassignment of its Skeleton property.

Paths below are relative to `Templates/TemplateResources/High/` in the installed engine. File presence and embedded AnimSequence/skeleton references were inspected; motion quality, root motion and duration remain **UNVERIFIED** until editor inspection.

| Purpose | Exact local file |
| --- | --- |
| Player mesh | `Characters/Content/Mannequins/Meshes/SKM_Manny_Simple.uasset` |
| Alternative mesh, not required | `Characters/Content/Mannequins/Meshes/SKM_Quinn_Simple.uasset` |
| Idle | `Characters/Content/Mannequins/Anims/Unarmed/MM_Idle.uasset` |
| Walk | `Characters/Content/Mannequins/Anims/Unarmed/Walk/MF_Unarmed_Walk_Fwd.uasset` |
| Jog, same UE5 skeleton | `Characters/Content/Mannequins/Anims/Unarmed/Jog/MF_Unarmed_Jog_Fwd.uasset` |
| Distinct run donor | `Mannequin/Content/Animations/ThirdPersonRun.uasset` |
| Attack | `Characters/Content/Mannequins/Anims/Unarmed/Attack/MM_Attack_01.uasset` |
| Death | `Characters/Content/Mannequins/Anims/Death/MM_Death_Front_01.uasset` |

Other confirmed local sequences include `MM_Attack_02`, `MM_Attack_03`, `MM_ChargedAttack`, `MM_Death_Front_02`, `MM_Death_Front_03`, `MM_Death_Back_01`, `MM_Death_Left_01` and `MM_Death_Right_01`. These are alternatives, not additional requirements. Do not describe the jog as a separately verified run animation; use the explicit run donor if the walk/jog set is insufficient.

**Size:** measured whole `Characters/Content/Mannequins` tree: 131,381,051 bytes (~125.3 MiB). Legacy `Mannequin/Content` tree: 41,208,714 bytes (~39.3 MiB). Together ~164.6 MiB already on disk; imported dependency subset size **UNVERIFIED**, additional download **0 bytes**. Manny mesh alone is ~15.1 MiB. These are installed asset sizes, not compressed download estimates.

### Player alternative: Game Animation Sample — not selected

- **[Fab listing][gas-fab], Epic Games:** free at research time; Unreal project. [Official sample documentation][gas-doc] describes locomotion/traversal and retargeting from the sample's UEFN mannequin to other characters, including UE5 mannequins.
- **Clips:** walking, running, jumping/falling and vaulting/climbing roles are documented; exact sequence filenames are **UNVERIFIED**. **Attack and death are not documented by the inspected listing/manual. Their presence or absence in the current download is UNVERIFIED.** This is not evidence that the binary contains none; it is enough to reject GAS as the sole verified combat-animation source. Do not confuse Game Animation Sample with the disabled Gameplay Ability System.
- **Compatibility:** official documentation is available for UE 5.8; the precise Fab downloadable project versions and a successful 5.8 load are **UNVERIFIED**. Skeleton: sample UEFN mannequin, with documented retargeting rather than a promise of identical bone layout to every Manny pack.
- **Licence / size:** current listing's licence label and download bytes **UNVERIFIED**. Do not infer a licence from Epic authorship or the word “Sample”; separately distributed Fab content can have separate terms. Shipped/raw redistribution permissions must follow the actual acquisition licence, using the rules below.
- **Route if ever substituted:** launcher **Create Project**, then agent migrates selected assets; do not look for **Add to Project** on a complete-project sample. Not on the baseline owner download list.

### Player alternative: MC Sample Animation Pack — free combat supplement

- **[Fab listing][mc-fab], MoCap Central:** free, 120+ animations, UE5 mannequin target. The [publisher product page][mc-product] identifies v1.2.1 and a UE5.0 project supporting UE5.0–5.8, with source FBX available through its own distribution. Exact correspondence of that version/FBX payload to the Fab download is **UNVERIFIED**.
- **Exact advertised identifiers:** the [publisher animation inventory][mc-clips] includes `am_Stand_Idle_03_LookAround`, `am_OrcHammer_Loco_Walk_Fwd`, `am_OrcHammer_Loco_Walk_Fwd_NoRM`, `am_Ready_Fight_01_Knockdown_A`, `af_Ready_Fight_01_Knockdown_B`, `am_Ready_Fight_01_Kickdown_A`, and `am_InjuredOnKnee_Die`. A distinct run clip is **UNVERIFIED**. Attack material is paired fight/knockdown choreography; the death starts injured/on a knee, so these require preview and possibly trimming for a standing auto-attack loop.
- **Compatibility / skeleton:** publisher explicitly supports 5.8 and the UE5 mannequin; validate the actual delivered skeleton, especially its v1.2.1 root-node change. IK Rig retargeting remains available if layouts differ.
- **Licence / size:** Fab licence label, Fab build version and archive size **UNVERIFIED**; do not import another storefront's terms. This is an identified free attack/death supplement if GAS were chosen, but the installed template clips are a simpler baseline. No owner download needed for the selected plan.

### Monster alternative: ANIMAL VARIETY PACK wolf — preferred appearance, unverified clips

- **[Fab listing][animals], PROTOFACTOR INC:** currently free. Wolf advertised with **26 animations**, 33 bones, approximately 10.11k triangles, two materials and 4K textures. A custom quadruped skeleton is indicated by the [publisher's standalone Wolf product][wolf-publisher]; it is not the Epic humanoid skeleton.
- **Required clip evidence:** idle, walk, attack and death **exact names and presence in this Fab payload are all UNVERIFIED**. The Fab page reports a count, not a named inventory. Its linked [Sketchfab preview][wolf-preview] exposes a single combined `Take 001` through the [preview animation endpoint][wolf-preview-anims]; this is not an inventory of the downloadable clips.
- **Version:** Fab-supported UE versions and native UE 5.8 support **UNVERIFIED**. The separate publisher Wolf product says UE4.15+ and lists 29 animations; that conflicts with the Fab pack's 26 and must not be silently substituted as evidence for this payload. A generic skeleton normally needs its own AnimBP, not Manny retargeting.
- **Licence / size:** actual Fab licence label, total pack bytes and isolated wolf dependency size **UNVERIFIED**. “Free” alone does not establish commercial or raw redistribution permission. The separate paid Wolf product is **not** the acquisition target.
- **Decision:** not the baseline until exact four-role coverage and terms are established. If the optional launcher route is available during the same session, acquire the free whole pack as an audition candidate alongside the baseline monster. No later owner session may be assumed if it fails validation.

### Monster: Animated Cockatrice — selected baseline

- **[Fab listing][cockatrice], Marko J (credit name Marko Jäntti):** free; rigged creature, approximately 2.2k triangles. Advertised labels: **Idle (flapping wings), Basic idle, Idle turn head (left), Walk cycle, Attack (peck), Receiving damage, Death**. These are the publisher's labels, not verified filenames/FBX take names.
- **Skeleton:** custom creature rig expected from this nonhumanoid model; hierarchy, bone count, root bone, frame rate and separate-action packaging **UNVERIFIED**. Keep its animations on its own imported skeleton.
- **Version / UE compatibility:** FBX, OBJ and Blender source are advertised; no explicit UE5.8 claim or versioned UE package was verified. FBX is an engine-supported import route, **not proof of a successful import**. Download original FBX plus textures and available `.blend` source; OBJ cannot carry the needed skeletal animation. Agent may need to export individual Blender actions.
- **Licence:** applicable Fab licence label **UNVERIFIED**. The publisher explicitly requests author credit. Preserve that credit regardless of whether the actual licence turns out to be Fab Standard or CC-BY; do not label it CC0 or assume the credit sentence establishes CC-BY.
- **Approximate size:** the [same publisher's ArtStation file inventory][cockatrice-size] lists FBX ~4.4 MB, textures ~11 MB, Blender source ~15 MB, and ~7.3 MB of DAE/OBJ/material files. Thus FBX+textures is approximately **15.4 MB**, all listed formats approximately **37.7 MB** on that storefront. **Fab download size is UNVERIFIED**; this is a planning reference, not a measured Fab archive. ArtStation's licence is separate and must not be copied into the Fab acquisition record.

### Optional environment: Stylized Low Poly Nature Lite

- **[Fab listing][nature], JustCreate:** free; six static low-poly models, FBX files and an Unreal/Unity demo advertised. Skeleton and animation clips: **N/A**.
- **Version / compatibility:** publisher release number and supported UE version list **UNVERIFIED**. Prefer FBX import into UE5.8; material reconstruction and editor/cook validation remain necessary.
- **Licence / size:** applicable Fab licence label and approximate download size **UNVERIFIED**. Six models describes scope, not disk size. No basis to promise this is a sub-100 MB download. Skip it if the free source payload cannot be acquired in the same session; the current test-zone geometry remains sufficient.

## Licence and public-repository policy

Fab supports multiple licence types, including legacy Marketplace terms on some migrated products; read the applicable listing/download terms rather than assigning one licence to every Fab item. [Fab licensing documentation][fab-licenses]

| Licence, only when actually confirmed | Shipped game | Raw or editable assets in a public repository |
| --- | --- | --- |
| [Fab Standard License][fab-eula] | Permits commercial projects, modification and distribution as part of a project. | Does not permit standalone source redistribution. Private sharing is limited to collaborators working on the project. Keep originals and derivatives out of public git. Personal/Professional have the same usage rights; choose the eligible tier, not an ineligible tier to obtain a price. |
| [Epic Content License Agreement][content-eula], if applicable | Permits distribution incorporated into projects in object/cooked form; observe any UE-only designation. | Raw source sharing is restricted to permitted employees, affiliates and contractors developing the project. Do not publish loose assets in a public repo. |
| [CC BY 4.0][cc-by], only if explicitly identified | Allows commercial use and adaptations with attribution, licence/source links and modification notices. | Allows sharing, including raw assets, subject to its conditions. Still keep vendor files ignored under this project's policy. |
| [Unreal Engine EULA Examples][ue-eula], for the identified installed template files | Permitted under engine terms. | §5(b) permits Examples source/object distribution; this exception does not extend automatically to unrelated Fab or engine content. |
| **UNVERIFIED** | Do not certify shipping rights until the actual terms are recorded. | Keep ignored; no public redistribution. |

`apps/client-unreal/.gitignore` contains the unanchored directory pattern **`Vendor/`**. It covers both staging `apps/client-unreal/Vendor/` and imported `apps/client-unreal/Content/Vendor/`, including retargeted/edited vendor animations and textures. Git LFS is storage, not redistribution permission. It does not retroactively untrack files; implementation must inspect any existing tracked vendor paths before adding content. Public source should contain importer code, manifests and attribution text, not copies of restricted assets or acquisition receipts containing account details.

## Exactly one owner download session

This is the complete planned checklist, **not a request to start downloading before the unresolved licence gate is closed**. No manual editor import, retargeting, Blueprint editing or material work is assigned to the owner. The selected baseline needs only the Fab website and the existing engine installation.

Before the session, the agent prepares these ignored directories and, only if the optional wolf route is usable, a launcher-visible scratch project on the owner's supported host:

```text
apps/client-unreal/Vendor/
  Downloads/Cockatrice/       # original FBX, textures, available Blender source
  Downloads/NatureLite/       # optional original FBX + texture files
  Projects/Phase1Intake/      # optional launcher Add to Project destination
  Evidence/                  # private licence/download receipts and version records
  Inventory/                 # checksums and actual file/clip inspection results
```

1. **Sign in once** to [Fab](https://www.fab.com/) with the Epic account that will hold the assets. For the optional launcher route, use the same account in Epic Games Launcher. Check that each requested product still shows **Free**; this manifest authorizes no paid substitute.
2. Open **[Animated Cockatrice][cockatrice]**. Use **Add to My Library**, review the actual licence shown in the acquisition/download dialog, and retain its label/link and receipt or screenshot in `Vendor/Evidence/`. If it imposes incompatible terms, stop this item rather than guessing. Use **Download** for the **original FBX and texture files**, plus available Blender/source archives. Save everything in `Vendor/Downloads/Cockatrice/`; leave original filenames intact. A library entitlement alone is not a file download.
3. In the **same session**, optionally add **[Stylized Low Poly Nature Lite][nature]** to the library and record its terms. Download the original **FBX + textures/source archive** to `Vendor/Downloads/NatureLite/`. Do not choose a Unity-only payload. If only an Unreal payload is exposed, use the scratch-project route below on a supported host or omit this optional pack.
4. **Optional wolf audition, same session only:** add **[ANIMAL VARIETY PACK][animals]** to the library, recording its actual terms. In Epic Games Launcher → Unreal Engine → Library → Fab Library, refresh/search for the pack and choose **Add to Project**, targeting the agent-prepared **Phase1Intake** project under `Vendor/Projects/`, not the live Nightfall project. If 5.8 is not offered, use an agent-prepared compatible scratch project for a version actually listed; the agent will upgrade a copy and migrate later. Do not fabricate a supported version or force an unknown selection. Keep the cockatrice download even when acquiring the wolf.
5. **No player download is required:** the identified Manny and legacy run assets already exist in the local 5.8.3 engine. Do not download Game Animation Sample or the MC pack for the baseline. For reference, complete sample projects use **Create Project**; content packs use **Add to Project** when offered.
6. Before ending the session, confirm actual files have arrived in staging: monster FBX, textures and all available animation/source archives; optional nature files; and, if requested, the populated scratch project's `Content/`. Preserve the complete scratch project and its content paths. If the launcher ran on a different machine, copy that project directory intact into this clone's `Vendor/Projects/Phase1Intake/` as part of this same bundled handoff. Record product/version/format selections and actual download sizes. Do not put assets directly in generated `Content/Blueprints`, `Input`, `UI` or `Maps`.
7. Hand the staged files back to the agent. The owner is finished; the agent handles unpacking, clip inspection, import, retargeting and wiring. A failed optional wolf or environment import must not create another owner download task.

**Platform constraint:** [Fab in Launcher currently supports Windows and macOS][fab-launcher]. This clone is Linux. [Fab acquisition documentation][fab-download] distinguishes browser-downloadable interchange formats from Unreal-native payloads obtained through Launcher/in-editor integration. Do not promise a browser ZIP of an Unreal-only pack, assume a Linux launcher exists, or ask the owner to perform in-editor import as a workaround. The optional wolf route requires a supported launcher host and scratch project prepared **before** the session; otherwise omit it. The baseline FBX route avoids that dependency.

## Agent implementation plan after handoff

1. **Inventory and rights gate.** Record source URL/ID, publisher, selected licence/tier, acquisition date, release/version, original filenames, byte counts and SHA-256 hashes in a machine-readable inventory. Verify the monster has separate usable idle/walk/attack/death motions; inspect FBX takes/Blender actions, frame ranges, loop boundaries, root motion, skeleton and materials. Replace each relevant **UNVERIFIED** entry with evidence. A price label, an animation count or a preview reel is not a clip test. Do not certify shipping with an unknown licence.
2. **Implement reproducible intake.** Add an importer (planned `Scripts/import_phase1_assets.py` plus a shell entry point) that accepts explicit source roots and an inventory, reports missing inputs and imports into stable asset paths. Keep downloaded originals immutable and ignored. Track Nightfall-authored code/config only. Record import options and exact role-to-sequence mappings so a clean clone plus legitimately acquired files can reproduce the content without a second manual editor session.
3. **Manny migration.** Build a transient scratch project from the installed templates so original `/Game/Characters/...` and `/Game/Mannequin/...` references resolve. Use Unreal's asset operations to relocate selected dependency closures into `/Game/Vendor/EpicMannequins/` and `/Game/Vendor/LegacyMannequin/`, fix redirectors, then [Migrate][migrate] into Nightfall's **`Content/` root**. Unreal migration preserves package-relative paths; do not target an arbitrary nested folder or rename `.uasset` files with filesystem operations. Include skeletal meshes, skeletons, physics assets, materials/textures and required sequences; omit donor game modes, input and gameplay Blueprints.
4. **Player retargeting.** Keep native Manny sequences on their matching UE5 skeleton. For `ThirdPersonRun`, create UE4-source and Manny-target **IK Rigs** with pelvis retarget roots and corresponding spine/arm/leg chains; match reference poses, use an **IK Retargeter**, inspect feet/hips and bake a target animation under `/Game/Vendor/Retargeted/`. Validate root translation and foot contact; prefer in-place playback for server-driven movement. [Epic IK Rig/retargeting documentation][ik-retarget] Do not assume an included Control Rig is already an IK Rig.
5. **Creature import.** Import the cockatrice's original FBX into `/Game/Vendor/Cockatrice/` with its own skeleton. Import each take/action against that skeleton; if the FBX contains only a combined take, use available Blender actions or documented frame boundaries to export separate FBX animations. The [UE FBX pipeline][fbx-pipeline] uses FBX 2020.2; validate conversion rather than assuming every exporter version works. Rebuild materials from supplied maps. No quadruped/creature-to-Manny retarget is planned. If auditioning the wolf, independently verify its four roles and licence before using the same scratch migration procedure; older-package migration is a candidate path, not proof of compatibility. [UE5 migration guide][ue-migration]
6. **Animation Blueprint wiring.** Generate separate player and creature AnimBPs outside the regenerated content directories, under an ignored vendor-derived subtree where needed. Player locomotion uses idle/walk/run speed blending; monster uses idle/walk. Attack and death use explicit states/montages; death is nonlooping and holds its final pose until server-authorized removal/respawn. Server combat cycle IDs, start/impact/ready ticks and life state drive playback; animation notifies never calculate/apply damage or decide death. Root motion must not move the authoritative entity.
7. **Attach to existing actors.** `ARemoteEntityActor::Body` and `ANightfallCharacter::Body` are currently **static** mesh components. Add a skeletal presentation component for `BP_RemoteEntity` and use the character's inherited skeletal mesh (`GetMesh()`) for the player, retiring/hiding the cylinder bodies as appropriate. Select mesh/AnimBP by entity kind/template; preserve collision/click targeting. Update the content-generation script so regenerated `BP_RemoteEntity` and player pawn wiring references these stable imported assets. Do not place irreplaceable imports in `Content/{Input,Blueprints,UI,Maps}`, which `create_content.py` recreates. Preserve 150 ms remote interpolation and local movement reconciliation; WorldProxySubsystem must still avoid duplicating the own-player entity. See the [client architecture and content contract][client-readme].
8. **Environment, if acquired.** Import only the needed static meshes/materials into `/Game/Vendor/NatureLite/`; decorate the existing test zone through its generator. Preserve the server tile/cm convention, ground click trace and navigation. Do not replace Nightfall's map/game mode with the vendor demo.
9. **Complete the E5.1 validation gates.** Load mesh, skeleton, all five player roles and all four monster roles in 5.8.3; preview attack/death and their transitions. Run the importer twice and verify stable asset paths, no duplicate assets and unchanged role mappings. Regenerate normal content and verify references survive. Run editor asset-load and cook checks, including missing-material/redirector checks. Exercise spawn, move, attack, death/corpse and respawn presentation against server events as E5.4 wiring lands. These checks are **planned, not run** by this research task.

## Attribution / licence file plan

Create and version **`apps/client-unreal/THIRD_PARTY_ASSETS.md`** during intake, once actual terms are known. It should contain one record per imported source (including template assets and any animation donor), with:

- Product title, publisher/credit name, canonical listing URL/ID, acquired version/date/format and checksum inventory reference.
- Exact licence name/version and canonical licence URL; allowed shipped-game use, raw-source restrictions, any UE-only condition and required attribution. Never replace an unknown licence with “free.”
- Original-to-imported asset paths and modifications: conversions, retargeting, trimming, material changes and generated derivatives. Include run-donor provenance separately.
- A short credits entry for **Marko Jäntti**, linking the cockatrice listing and the confirmed licence, with modification notices where required. Include that credit in the shipped game's credits or bundled notices as well as the repository document.
- A statement that vendor files are intentionally absent from the public repository and must be acquired under their applicable terms. Keep account details, order identifiers and private receipts only under ignored `Vendor/Evidence/`.

Do not create an apparently final licence ledger populated with guessed licence labels. Do not copy the licence from another storefront or bundle a public raw-asset archive as a convenience for contributors.

## Open questions and acceptance limits

1. **Fab licence metadata:** which licence/version is actually offered for the cockatrice, optional nature pack and optional wolf? The fetched public listing bodies show an empty licence field. This is the outstanding pre-download rights gate; the authenticated acquisition dialog must supply evidence. No item is certified for shipping until that is resolved.
2. **Monster import:** what are the exact delivered take/action names, root-motion settings and durations, and does the cockatrice import/cook correctly in UE5.8.3? Advertised role coverage is verified; binary behaviour is not. There is no fully tested monster asset in this research result.
3. **Wolf preference:** does the free Fab payload actually contain the four required clips? Public sources did not establish this, and standalone publisher counts differ. Keep it optional until inspected; do not advertise its inclusion as settled.
4. **Owner platform, only for optional Unreal-native acquisitions:** is a Windows/macOS launcher host available with an agent-prepared compatible intake project? If not, the selected website-download baseline still applies.
5. **Actual versions and storage:** Fab release identifiers and byte counts remain unknown for the web candidates. Capture them during acquisition; do not reserve disk based solely on the cockatrice's other-storefront estimate.
6. **Player motion quality:** local sequence presence is verified, but the legacy run retarget, attack readability and death transition still need editor validation. No additional manual player download is anticipated.

[third-person]: https://dev.epicgames.com/documentation/en-us/unreal-engine/third-person-template-in-unreal-engine
[linux-engine]: https://www.unrealengine.com/linux
[ue-eula]: https://www.unrealengine.com/eula/unreal
[gas-fab]: https://www.fab.com/listings/880e319a-a59e-4ed2-b268-b32dac7fa016
[gas-doc]: https://dev.epicgames.com/documentation/en-us/unreal-engine/game-animation-sample-project-in-unreal-engine
[mc-fab]: https://www.fab.com/listings/fba58a40-dc18-475a-b726-b04345f39697
[mc-product]: https://mocapcentral.com/products/mocap-studio-series-sample-pack-free
[mc-clips]: https://mocapcentral.com/pages/sample-animation-list
[animals]: https://www.fab.com/listings/2dd7964c-a601-4264-a53d-465dcae1644c
[wolf-publisher]: https://protofactor.biz/product/wolf/
[wolf-preview]: https://sketchfab.com/models/2cddc5b971ec4d5f8ad146e7002dbf1a
[wolf-preview-anims]: https://sketchfab.com/i/models/2cddc5b971ec4d5f8ad146e7002dbf1a/animations
[cockatrice]: https://www.fab.com/listings/c2c18fbc-48f8-41dc-8817-27bd6a6c9d74
[cockatrice-size]: https://www.artstation.com/marketplace/p/jND6O/animated-cockatrice
[nature]: https://www.fab.com/listings/fef6b0c6-19a4-47bc-913f-c8328910cec4
[fab-licenses]: https://dev.epicgames.com/documentation/fab/licenses-and-pricing-in-fab?lang=en-US
[fab-eula]: https://www.fab.com/eula
[content-eula]: https://www.unrealengine.com/eula/content
[cc-by]: https://creativecommons.org/licenses/by/4.0/
[fab-download]: https://dev.epicgames.com/documentation/en-us/fab/purchasing-and-downloading-assets-in-fab
[fab-launcher]: https://dev.epicgames.com/documentation/fab/exporting-assets-from-fab-in-launcher
[migrate]: https://dev.epicgames.com/documentation/en-us/unreal-engine/migrating-assets-in-unreal-engine
[ik-retarget]: https://dev.epicgames.com/documentation/en-us/unreal-engine/ik-rig-animation-retargeting-in-unreal-engine
[fbx-pipeline]: https://dev.epicgames.com/documentation/en-us/unreal-engine/fbx-skeletal-mesh-pipeline-in-unreal-engine
[ue-migration]: https://dev.epicgames.com/documentation/en-us/unreal-engine/unreal-engine-5-migration-guide
[client-readme]: ../../../apps/client-unreal/README.md
