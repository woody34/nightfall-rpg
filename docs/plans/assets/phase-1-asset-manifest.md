# Phase 1 asset acquisition manifest — E5.1 research

The research date was **2026-10-08**, for UE **5.8.3**. The research follows [Phase 1 §2 D7, E5.1 and R3](../phase-1-kill-a-monster.md) and the [client README](../../../apps/client-unreal/README.md). This document covers the acquisition research and the implementation plan that comes after it. It does **not** claim that the E5.1 import, repeat-import, editor-load or cook gates passed.

**Decision:** Use the installed Epic Manny template assets for the player. Select Animated Cockatrice as the monster baseline. Optionally, get Stylized Low Poly Nature Lite. With the cockatrice, the project loses the wolf/keltir appearance that it prefers. But its publisher explicitly advertises all four necessary animation roles. ANIMAL VARIETY PACK is the alternative with the preferred appearance, but the exact clip coverage of its wolf is **UNVERIFIED**. Do not use it to replace the baseline only because it is free.

**Handoff status: conditional, not a fully validated acquisition gate.** The public Fab pages showed that the selected listings are free. But the licence/version/download-size fields on these pages were blank. The applicable licence labels are **UNVERIFIED**. The publishers advertise the monster clip roles; nobody examined the FBX takes. The actual UE 5.8 import is **UNVERIFIED**. Resolve the applicable terms before you download, and examine the files after the one acquisition session. Do not mark R3 or E5.1 complete because of this research alone. During the research, nobody made purchases, account changes, downloads of vendor content, imports or game-code changes.

## Decision table

| Category | Selected option | Reason and compatibility evidence | Acquisition / remaining gate |
| --- | --- | --- | --- |
| Player | **Epic Manny**, installed UE 5.8.3 template mesh/idle/walk/attack/death; the installed legacy mannequin run, retargeted to Manny | The actual local files cover all five roles. Native UE5 mannequin skeleton, but the UE4 run donor is different. A large sample-project download is not necessary. | Installed now; zero more download. The editor playback and the UE4→UE5 retarget result must have validation. |
| Monster | **[Animated Cockatrice][cockatrice] — Marko J / Marko Jäntti** | Free. The publisher explicitly advertises idle, walking, attack and death. FBX and Blender source give a practical import route on Linux. | In the one session, download the original FBX, the textures and the available Blender source. Licence label, exact take names and UE 5.8 import **UNVERIFIED**. Give credit to the author. |
| Environment, optional | **[Stylized Low Poly Nature Lite][nature] — JustCreate** | Free. Six static models, with advertised FBX and Unreal versions. The scope is small, and it is applicable to decorate the test zone that exists. | Prefer the original FBX + textures from Fab. Actual bytes, licence label and UE 5.8 import **UNVERIFIED**. If it is not available, do not use it. It must not block combat. |

The necessary baseline is available without a Windows/macOS launcher, on purpose: Manny is local, and the monster advertises interchange files that you can download. The Unreal-native route for the wolf is a separate, optional acquisition in the same session. It is not a second mandatory owner task.

## Candidate evidence

### Player: installed Epic templates — selected

- **Publisher / listing:** Epic Games. These are templates that the engine installs. They are not a separate Fab purchase. [Third Person template documentation][third-person]; [Epic Linux engine download][linux-engine]. The documentation describes Manny/Quinn and the Combat variant. But the exact inventory below is **local filesystem evidence**. It is not a claim that the webpage lists these files.
- **Version:** The installed engine of this clone is at `/home/matt-woodruff/Linux_Unreal_Engine_5.8.3`. `Engine/Build/Build.version` shows 5.8.3, changelist 58210709. The template files are in that installation. This research did not test the UE editor load/playback.
- **Licence:** [Unreal Engine EULA][ue-eula]. It is not an assumed Fab Standard or Epic Content licence. Section 1 includes the installed Samples/Templates content in **Examples**. §5(b) lets you distribute Examples in source or object form. Thus these identified template assets are an exception to the usual prohibition on public raw-vendor redistribution. You can use them in a shipped game, subject to the EULA. But keep them ignored, for consistent provenance, and to prevent the accidental inclusion of unrelated restricted content. Do not relicense them as Nightfall code.
- **Skeleton:** The Manny/Quinn files refer to `/Game/Characters/Mannequins/Meshes/SK_Mannequin`. The legacy run refers to `/Game/Mannequin/Character/Mesh/UE4_Mannequin_Skeleton`. It must have IK retargeting, not a new assignment of its Skeleton property.

The paths below are relative to `Templates/TemplateResources/High/` in the installed engine. The research examined the file presence and the embedded AnimSequence/skeleton references. The motion quality, root motion and duration stay **UNVERIFIED** until an editor inspection.

| Purpose | Exact local file |
| --- | --- |
| Player mesh | `Characters/Content/Mannequins/Meshes/SKM_Manny_Simple.uasset` |
| Alternative mesh, not necessary | `Characters/Content/Mannequins/Meshes/SKM_Quinn_Simple.uasset` |
| Idle | `Characters/Content/Mannequins/Anims/Unarmed/MM_Idle.uasset` |
| Walk | `Characters/Content/Mannequins/Anims/Unarmed/Walk/MF_Unarmed_Walk_Fwd.uasset` |
| Jog, same UE5 skeleton | `Characters/Content/Mannequins/Anims/Unarmed/Jog/MF_Unarmed_Jog_Fwd.uasset` |
| Separate run donor | `Mannequin/Content/Animations/ThirdPersonRun.uasset` |
| Attack | `Characters/Content/Mannequins/Anims/Unarmed/Attack/MM_Attack_01.uasset` |
| Death | `Characters/Content/Mannequins/Anims/Death/MM_Death_Front_01.uasset` |

Other local sequences that the research found include `MM_Attack_02`, `MM_Attack_03`, `MM_ChargedAttack`, `MM_Death_Front_02`, `MM_Death_Front_03`, `MM_Death_Back_01`, `MM_Death_Left_01` and `MM_Death_Right_01`. These are alternatives, not more requirements. Do not describe the jog as a run animation that has separate verification. If the walk/jog set is not sufficient, use the explicit run donor.

**Size:** The measured size of the full `Characters/Content/Mannequins` tree is 131,381,051 bytes (~125.3 MiB). The legacy `Mannequin/Content` tree is 41,208,714 bytes (~39.3 MiB). Together, ~164.6 MiB is on the disk now. The size of the imported dependency subset is **UNVERIFIED**, and the additional download is **0 bytes**. The Manny mesh alone is ~15.1 MiB. These are installed asset sizes, not compressed download estimates.

### Player alternative: Game Animation Sample — not selected

- **[Fab listing][gas-fab], Epic Games:** Free at the time of the research; Unreal project. The [official sample documentation][gas-doc] describes locomotion/traversal, and retargeting from the UEFN mannequin of the sample to other characters, which include UE5 mannequins.
- **Clips:** The documentation gives the walking, running, jumping/falling and vaulting/climbing roles. The exact sequence filenames are **UNVERIFIED**. **The examined listing/manual does not document attack and death. Their presence or absence in the current download is UNVERIFIED.** This is not evidence that the binary has none. But it is sufficient to reject GAS as the only verified source of combat animation. Do not confuse Game Animation Sample with the disabled Gameplay Ability System.
- **Compatibility:** Official documentation is available for UE 5.8. The precise Fab downloadable project versions and a successful 5.8 load are **UNVERIFIED**. Skeleton: the sample UEFN mannequin, with documented retargeting. This is not a promise of a bone layout that is identical to each Manny pack.
- **Licence / size:** The licence label and the download bytes of the current listing are **UNVERIFIED**. Do not infer a licence from the Epic authorship or from the word “Sample”. Fab content that has a separate distribution can have separate terms. The shipped/raw redistribution permissions must follow the actual acquisition licence, with the rules below.
- **Route if it is ever a substitute:** Use the launcher **Create Project**, then the agent migrates the selected assets. Do not look for **Add to Project** on a complete-project sample. It is not on the baseline owner download list.

### Player alternative: MC Sample Animation Pack — free combat supplement

- **[Fab listing][mc-fab], MoCap Central:** Free, 120+ animations, UE5 mannequin target. The [publisher product page][mc-product] identifies v1.2.1, and a UE5.0 project that supports UE5.0–5.8, with source FBX available through the distribution of the publisher. The exact correspondence of that version/FBX payload to the Fab download is **UNVERIFIED**.
- **Exact advertised identifiers:** The [publisher animation inventory][mc-clips] includes `am_Stand_Idle_03_LookAround`, `am_OrcHammer_Loco_Walk_Fwd`, `am_OrcHammer_Loco_Walk_Fwd_NoRM`, `am_Ready_Fight_01_Knockdown_A`, `af_Ready_Fight_01_Knockdown_B`, `am_Ready_Fight_01_Kickdown_A`, and `am_InjuredOnKnee_Die`. A separate run clip is **UNVERIFIED**. The attack material is paired fight/knockdown choreography. The death starts with the character injured/on a knee. Thus these clips must have a preview, and possibly trimming, for a standing auto-attack loop.
- **Compatibility / skeleton:** The publisher explicitly supports 5.8 and the UE5 mannequin. Validate the actual delivered skeleton, and specially its v1.2.1 root-node change. If the layouts are different, IK Rig retargeting is available.
- **Licence / size:** The Fab licence label, the Fab build version and the archive size are **UNVERIFIED**. Do not import the terms of a different storefront. This is an identified free attack/death supplement if GAS is the selection. But the installed template clips are a simpler baseline. The selected plan does not need an owner download.

### Monster alternative: ANIMAL VARIETY PACK wolf — preferred appearance, unverified clips

- **[Fab listing][animals], PROTOFACTOR INC:** Free now. The wolf is advertised with **26 animations**, 33 bones, approximately 10.11k triangles, two materials and 4K textures. The [standalone Wolf product of the publisher][wolf-publisher] shows a custom quadruped skeleton. It is not the Epic humanoid skeleton.
- **Necessary clip evidence:** For idle, walk, attack and death, **the exact names and the presence in this Fab payload are all UNVERIFIED**. The Fab page gives a count, not a named inventory. Its linked [Sketchfab preview][wolf-preview] shows only one combined `Take 001` through the [preview animation endpoint][wolf-preview-anims]. This is not an inventory of the clips that you can download.
- **Version:** The UE versions that Fab supports, and native UE 5.8 support, are **UNVERIFIED**. The separate Wolf product of the publisher says UE4.15+ and lists 29 animations. That is not the same as the 26 of the Fab pack. Do not silently use it as evidence for this payload. A generic skeleton in most conditions must have its own AnimBP, not Manny retargeting.
- **Licence / size:** The actual Fab licence label, the total pack bytes and the size of the isolated wolf dependency are **UNVERIFIED**. “Free” alone does not give commercial or raw redistribution permission. The separate paid Wolf product is **not** the acquisition target.
- **Decision:** It is not the baseline until the exact coverage of the four roles and the terms are known. If the optional launcher route is available during the same session, get the free full pack as an audition candidate together with the baseline monster. If it fails validation, do not assume that a later owner session will occur.

### Monster: Animated Cockatrice — selected baseline

- **[Fab listing][cockatrice], Marko J (credit name Marko Jäntti):** Free; rigged creature, approximately 2.2k triangles. Advertised labels: **Idle (flapping wings), Basic idle, Idle turn head (left), Walk cycle, Attack (peck), Receiving damage, Death**. These are the labels of the publisher, not verified filenames/FBX take names.
- **Skeleton:** For this nonhumanoid model, the expected skeleton is a custom creature rig. The hierarchy, bone count, root bone, frame rate and separate-action packaging are **UNVERIFIED**. Keep its animations on its own imported skeleton.
- **Version / UE compatibility:** FBX, OBJ and Blender source are advertised. The research did not verify an explicit UE5.8 claim or a versioned UE package. FBX is an import route that the engine supports, **not proof of a successful import**. Download the original FBX, the textures and the available `.blend` source. OBJ cannot carry the necessary skeletal animation. The agent can possibly have to export the individual Blender actions.
- **Licence:** The applicable Fab licence label is **UNVERIFIED**. The publisher explicitly requests author credit. Keep that credit, whether the actual licence is Fab Standard or CC-BY. Do not label it CC0, and do not assume that the credit sentence makes it CC-BY.
- **Approximate size:** The same publisher has an [ArtStation file inventory][cockatrice-size]. It lists FBX ~4.4 MB, textures ~11 MB, Blender source ~15 MB, and ~7.3 MB of DAE/OBJ/material files. Thus FBX+textures is approximately **15.4 MB**, and all listed formats are approximately **37.7 MB** on that storefront. **The Fab download size is UNVERIFIED.** This is a planning reference, not a measured Fab archive. The ArtStation licence is separate. Do not copy it into the Fab acquisition record.

### Optional environment: Stylized Low Poly Nature Lite

- **[Fab listing][nature], JustCreate:** Free. It advertises six static low-poly models, FBX files and an Unreal/Unity demo. Skeleton and animation clips: **N/A**.
- **Version / compatibility:** The release number of the publisher and the list of supported UE versions are **UNVERIFIED**. Prefer the FBX import into UE5.8. The material reconstruction and the editor/cook validation are still necessary.
- **Licence / size:** The applicable Fab licence label and the approximate download size are **UNVERIFIED**. "Six models" describes the scope, not the disk size. There is no basis to promise a download of less than 100 MB. If you cannot get the free source payload in the same session, do not get it. The geometry of the current test zone is sufficient.

## Licence and public-repository policy

Fab supports many licence types, which include legacy Marketplace terms on some migrated products. Read the applicable listing/download terms. Do not assign one licence to all Fab items. [Fab licensing documentation][fab-licenses]

| Licence, only when actually confirmed | Shipped game | Raw or editable assets in a public repository |
| --- | --- | --- |
| [Fab Standard License][fab-eula] | Lets you use the assets in commercial projects, change them, and distribute them as part of a project. | Does not let you redistribute the source by itself. You can share privately only with collaborators who work on the project. Keep the originals and the derivatives out of public git. Personal/Professional have the same usage rights. Select the tier for which you are eligible. Do not select a tier for which you are not eligible to get a price. |
| [Epic Content License Agreement][content-eula], if applicable | Lets you distribute the assets in projects, in object/cooked form. Obey all UE-only designations. | You can share raw source only with permitted employees, affiliates and contractors that develop the project. Do not publish loose assets in a public repo. |
| [CC BY 4.0][cc-by], only if explicitly identified | Lets you use the assets commercially and make adaptations, with attribution, licence/source links and modification notices. | Lets you share, also the raw assets, subject to its conditions. But this project's policy is to keep vendor files ignored. |
| [Unreal Engine EULA Examples][ue-eula], for the identified installed template files | Permitted under the engine terms. | §5(b) lets you distribute Examples in source/object form. This exception does not automatically apply to unrelated Fab or engine content. |
| **UNVERIFIED** | Do not certify shipping rights until the actual terms are recorded. | Keep ignored; no public redistribution. |

`apps/client-unreal/.gitignore` has the unanchored directory pattern **`Vendor/`**. It covers the staging directory `apps/client-unreal/Vendor/` and the import directory `apps/client-unreal/Content/Vendor/`, which include the retargeted/edited vendor animations and textures. Git LFS is storage, not redistribution permission. It does not untrack files that are tracked now. Before the implementation adds content, it must examine all vendor paths that are tracked now. The public source must include importer code, manifests and attribution text. It must not include copies of restricted assets, or acquisition receipts with account details.

## Exactly one owner download session

This is the complete planned checklist. It is **not a request to start downloading before the unresolved licence gate is closed**. The owner has no manual editor import, retargeting, Blueprint editing or material work. The selected baseline needs only the Fab website and the engine installation that exists.

Before the session, the agent prepares these ignored directories. If the optional wolf route is usable, the agent also prepares a scratch project on a supported host of the owner. The launcher must see this project:

```text
apps/client-unreal/Vendor/
  Downloads/Cockatrice/       # original FBX, textures, available Blender source
  Downloads/NatureLite/       # optional original FBX + texture files
  Projects/Phase1Intake/      # optional launcher Add to Project destination
  Evidence/                  # private licence/download receipts and version records
  Inventory/                 # checksums and actual file/clip inspection results
```

1. **Sign in one time** to [Fab](https://www.fab.com/) with the Epic account that will hold the assets. For the optional launcher route, use the same account in Epic Games Launcher. Make sure that each requested product still shows **Free**. This manifest does not authorize a paid substitute.
2. Open **[Animated Cockatrice][cockatrice]**. Use **Add to My Library**. Examine the actual licence that the acquisition/download dialog shows. Keep its label/link and the receipt or a screenshot in `Vendor/Evidence/`. If the terms are not compatible, stop this item. Do not guess. Use **Download** for the **original FBX and texture files**, and for the available Blender/source archives. Save all of them in `Vendor/Downloads/Cockatrice/`. Do not change the original filenames. A library entitlement alone is not a file download.
3. In the **same session**, you can add **[Stylized Low Poly Nature Lite][nature]** to the library and record its terms. Download the original **FBX + textures/source archive** to `Vendor/Downloads/NatureLite/`. Do not select a Unity-only payload. If only an Unreal payload is available, use the scratch-project route below on a supported host, or do not get this optional pack.
4. **Optional wolf audition, same session only:** Add **[ANIMAL VARIETY PACK][animals]** to the library, and record its actual terms. In Epic Games Launcher → Unreal Engine → Library → Fab Library, refresh/search for the pack and select **Add to Project**. Target the **Phase1Intake** project that the agent prepared in `Vendor/Projects/`, not the live Nightfall project. If 5.8 is not available, use a compatible scratch project that the agent prepared for a version that is actually on the list. The agent will upgrade a copy and migrate it later. Do not invent a supported version, and do not force an unknown selection. Keep the cockatrice download, also when you get the wolf.
5. **A player download is not necessary:** The identified Manny and legacy run assets are in the local 5.8.3 engine now. Do not download Game Animation Sample or the MC pack for the baseline. For reference, complete sample projects use **Create Project**. Content packs use **Add to Project** when it is available.
6. Before you stop the session, make sure that the actual files are in staging: the monster FBX, the textures and all available animation/source archives; the optional nature files; and, if requested, the `Content/` of the populated scratch project. Keep the complete scratch project and its content paths. If the launcher ran on a different machine, copy that full project directory into `Vendor/Projects/Phase1Intake/` of this clone. Do this as part of this same bundled handoff. Record the product/version/format selections and the actual download sizes. Do not put assets directly in the generated `Content/Blueprints`, `Input`, `UI` or `Maps`.
7. Give the staged files to the agent. The owner work is then complete. The agent does the unpacking, clip inspection, import, retargeting and wiring. If an optional wolf or environment import fails, this must not make another owner download task.

**Platform constraint:** [Fab in Launcher now supports Windows and macOS][fab-launcher]. This clone is Linux. The [Fab acquisition documentation][fab-download] gives two different types of payload. You can download interchange formats in a browser. You get Unreal-native payloads through the Launcher/in-editor integration. Do not promise a browser ZIP of an Unreal-only pack. Do not assume that a Linux launcher exists. Do not ask the owner to do an in-editor import as a workaround. The optional wolf route must have a supported launcher host and a scratch project that is prepared **before** the session. If not, do not use it. The baseline FBX route does not have that dependency.

## Agent implementation plan after handoff

1. **Inventory and rights gate.** In a machine-readable inventory, record the source URL/ID, publisher, selected licence/tier, acquisition date, release/version, original filenames, byte counts and SHA-256 hashes. Make sure that the monster has separate usable idle/walk/attack/death motions. Examine the FBX takes/Blender actions, frame ranges, loop boundaries, root motion, skeleton and materials. Replace each applicable **UNVERIFIED** entry with evidence. A price label, an animation count or a preview reel is not a clip test. Do not certify shipping with an unknown licence.
2. **Implement reproducible intake.** Add an importer (the planned `Scripts/import_phase1_assets.py` and a shell entry point). The importer accepts explicit source roots and an inventory, reports missing inputs, and imports into stable asset paths. Keep the downloaded originals immutable and ignored. Track only the code/config that Nightfall writes. Record the import options and the exact role-to-sequence mappings. Then a clean clone and legitimately acquired files can reproduce the content without a second manual editor session.
3. **Manny migration.** Build a temporary scratch project from the installed templates, so that the original `/Game/Characters/...` and `/Game/Mannequin/...` references resolve. Use the Unreal asset operations to move the selected dependency closures into `/Game/Vendor/EpicMannequins/` and `/Game/Vendor/LegacyMannequin/`. Repair the redirectors. Then [Migrate][migrate] into the **`Content/` root** of Nightfall. Unreal migration keeps the package-relative paths. Do not target an arbitrary nested folder, and do not rename `.uasset` files with filesystem operations. Include the skeletal meshes, skeletons, physics assets, materials/textures and the necessary sequences. Do not include the donor game modes, input and gameplay Blueprints.
4. **Player retargeting.** Keep the native Manny sequences on their matching UE5 skeleton. For `ThirdPersonRun`, make a UE4-source **IK Rig** and a Manny-target **IK Rig**, with pelvis retarget roots and the related spine/arm/leg chains. Match the reference poses and use an **IK Retargeter**. Examine the feet/hips, and bake a target animation in `/Game/Vendor/Retargeted/`. Validate the root translation and the foot contact. For movement that the server controls, prefer in-place playback. [Epic IK Rig/retargeting documentation][ik-retarget] Do not assume that an included Control Rig is an IK Rig.
5. **Creature import.** Import the original FBX of the cockatrice into `/Game/Vendor/Cockatrice/` with its own skeleton. Import each take/action against that skeleton. If the FBX has only a combined take, use the available Blender actions or documented frame boundaries to export separate FBX animations. The [UE FBX pipeline][fbx-pipeline] uses FBX 2020.2. Validate the conversion. Do not assume that all exporter versions operate correctly. Build the materials again from the supplied maps. There is no plan for a quadruped/creature-to-Manny retarget. If you audition the wolf, verify its four roles and licence independently before you use the same scratch migration procedure. The migration of an older package is a candidate path, not proof of compatibility. [UE5 migration guide][ue-migration]
6. **Animation Blueprint wiring.** Make separate player and creature AnimBPs outside the content directories that the script makes again. Where necessary, put them in an ignored vendor-derived subtree. Player locomotion uses idle/walk/run speed blending. The monster uses idle/walk. Attack and death use explicit states/montages. Death does not loop, and it holds its final pose until the server authorizes removal/respawn. The server combat cycle IDs, the start/impact/ready ticks and the life state control the playback. Animation notifies never calculate/apply damage or decide death. Root motion must not move the authoritative entity.
7. **Attach to the actors that exist.** `ARemoteEntityActor::Body` and `ANightfallCharacter::Body` are **static** mesh components now. Add a skeletal presentation component for `BP_RemoteEntity`. For the player, use the inherited skeletal mesh of the character (`GetMesh()`), and retire/hide the cylinder bodies as applicable. Select the mesh/AnimBP by entity kind/template. Keep the collision/click targeting. Update the content-generation script, so that the `BP_RemoteEntity` and player pawn wiring that it makes again refer to these stable imported assets. Do not put imports that you cannot replace in `Content/{Input,Blueprints,UI,Maps}`, because `create_content.py` makes these again. Keep the 150 ms remote interpolation and the local movement reconciliation. WorldProxySubsystem must still not duplicate the entity of the own player. See the [client architecture and content contract][client-readme].
8. **Environment, if acquired.** Import only the necessary static meshes/materials into `/Game/Vendor/NatureLite/`. Decorate the test zone that exists through its generator. Keep the server tile/cm convention, the ground click trace and the navigation. Do not replace the map/game mode of Nightfall with the vendor demo.
9. **Complete the E5.1 validation gates.** In 5.8.3, load the mesh, the skeleton, all five player roles and all four monster roles. Preview attack/death and their transitions. Run the importer two times. Make sure that the asset paths are stable, that there are no duplicate assets, and that the role mappings do not change. Make the normal content again, and make sure that the references stay. Run the editor asset-load and cook checks, with the missing-material/redirector checks. When the E5.4 wiring merges, test the spawn, move, attack, death/corpse and respawn presentation against server events. This research task **did not run** these checks; they are **planned**.

## Attribution / licence file plan

During intake, when the actual terms are known, make **`apps/client-unreal/THIRD_PARTY_ASSETS.md`** and put it under version control. It must have one record for each imported source (which includes template assets and all animation donors), with:

- The product title, publisher/credit name, canonical listing URL/ID, acquired version/date/format and the checksum inventory reference.
- The exact licence name/version and the canonical licence URL; the permitted shipped-game use, the raw-source restrictions, all UE-only conditions and the necessary attribution. Never replace an unknown licence with “free.”
- The original-to-imported asset paths and the changes: conversions, retargeting, trimming, material changes and generated derivatives. Include the provenance of the run donor separately.
- A short credits entry for **Marko Jäntti**, with a link to the cockatrice listing and the confirmed licence, and with modification notices where necessary. Include that credit in the credits or bundled notices of the shipped game, and also in the repository document.
- A statement that the vendor files are intentionally absent from the public repository, and that you must get them under their applicable terms. Keep account details, order identifiers and private receipts only in the ignored `Vendor/Evidence/`.

Do not make a licence ledger that looks final, with guessed licence labels. Do not copy the licence from a different storefront. Do not bundle a public raw-asset archive to make it easier for contributors.

## Open questions and acceptance limits

1. **Fab licence metadata:** Which licence/version is actually available for the cockatrice, the optional nature pack and the optional wolf? The public listing bodies that the research got show an empty licence field. This is the open rights gate before download. The authenticated acquisition dialog must supply the evidence. No item is certified for shipping until that is resolved.
2. **Monster import:** What are the exact delivered take/action names, root-motion settings and durations? Does the cockatrice import/cook correctly in UE5.8.3? The advertised role coverage is verified, but the binary behaviour is not. There is no fully tested monster asset in this research result.
3. **Wolf preference:** Does the free Fab payload actually have the four necessary clips? The public sources did not show this, and the standalone publisher counts are different. Keep it optional until an inspection. Do not advertise that its inclusion is decided.
4. **Owner platform, only for optional Unreal-native acquisitions:** Is a Windows/macOS launcher host available, with a compatible intake project that the agent prepared? If not, the selected website-download baseline still applies.
5. **Actual versions and storage:** The Fab release identifiers and byte counts for the web candidates are still unknown. Record them during acquisition. Do not reserve disk space only from the estimate of the cockatrice on the other storefront.
6. **Player motion quality:** The presence of the local sequences is verified. But the legacy run retarget, the attack readability and the death transition must still have editor validation. No more manual player download is expected.

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
