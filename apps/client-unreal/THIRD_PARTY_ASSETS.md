# Third-party assets

Art the Nightfall client uses but does not own. **None of these files are in this repository.**
Downloads live in `Vendor/` and imported/derived assets in `Content/Vendor/`; both are git-ignored
(`apps/client-unreal/.gitignore`: `Vendor/`). To reproduce them, acquire each item under its own
terms (below), put the original downloads in `Vendor/Downloads/`, and run
`moon run client-unreal:import-vendor` (see README, "Art and animation"). The repository holds only
the importer, these records and the soft references that name the imported assets.

Private acquisition evidence (licence acceptance screenshot, dates) is kept in the ignored
`Vendor/Evidence/`, never committed.

## Credits

> **Animated Cockatrice** by **Marko Jäntti** —
> <https://www.fab.com/listings/c2c18fbc-48f8-41dc-8817-27bd6a6c9d74>, used under the Fab Standard
> License. Modified: re-imported, materials rebuilt, takes renamed (see below).
>
> **Stylized Low Poly Nature Lite** by **JustCreate** —
> <https://www.fab.com/listings/fef6b0c6-19a4-47bc-913f-c8328910cec4>, used under the Fab Standard
> License.
>
> **Unreal Engine Mannequin (Manny) and animations** by **Epic Games** — Unreal Engine 5.8.3
> template content, used under the Unreal Engine EULA.

The cockatrice credit is required by its author ("Free to use with credit (Marko Jäntti)" on the
listing) and must also appear in any shipped build's credits or bundled notices.

## Records

### Animated Cockatrice

| Field | Value |
|---|---|
| Publisher / credit | Marko J on Fab; credit name **Marko Jäntti** |
| Listing | <https://www.fab.com/listings/c2c18fbc-48f8-41dc-8817-27bd6a6c9d74> (free) |
| Licence | **Fab Standard License** — the Fab End User License Agreement, last updated 2024-10-01, <https://www.fab.com/eula>. The listing's licence field reads "Standard License"; the repo owner accepted the EULA in the Fab download dialog on 2026-10-08 and confirmed this licence. |
| Attribution | Required by the author (listing text). The Fab EULA itself requires none. |
| Acquired | 2026-10-08, browser download from Fab, by the repo owner. Listing version: not shown (**UNVERIFIED**). |
| Files (sha256) | `cockatrice.fbx` 11,239,324 B `fcb41efd3771b7c377c3c838223481372092b54633820aa344e2c882e72afd5f`; `coctrice.textures.zip` 11,151,130 B `2f118ee7010b257b5d2c0591fb58f5c88b2059c7b758f3a5ddecadc052cdd715`; `cockatrice.blend` 15,330,019 B `bfdaaaacb5f4b3b47f2d6a336ddd505dbdd9a45430ea4bec6ecd087794c6400b`; `cockatrice.obj` 219,924 B `9f3c0f3105c987a6f497b7ae3cfc67b0cfde57f79da5caa170d5e0bb07c55a9b` (`.blend` and `.obj` are not used by the importer) |
| Imported to | `/Game/Vendor/Cockatrice/`: `SK_Cockatrice` (+ `_Skeleton`, its own creature rig, ~165 x 115 x 107 cm), `M_Cockatrice` (rebuilt from the base colour, normal, roughness, metallic and emission maps), `T_Cockatrice_*`, `Anims/AS_Cockatrice_{Idle,Walk,Attack,Damage,Death}` and every other take as `Anims/AS_Cockatrice_Take_*` |
| Modifications | FBX takes renamed by role; root locked on the role clips; one material rebuilt; frame-snapped import (the `Dead` action is 2.996 s at 24 fps). No retargeting. Used for the keltir (`BP_Keltir`), at 0.8 scale. |

What the FBX contains (inspected with the import; the listing's labels were *Idle (flapping wings),
Basic idle, Idle turn head (left), Walk cycle, Attack (peck), Receiving damage, Death*): one
AnimStack per Blender action, so no frame-range splitting was needed — `Idle1.001`, `Idle1.002`,
`Idle2`, `Idle2.001`, `Idle3`, `Idle3.001` (8.29 s each), `Walk.001`, `Walk.002` (14.58 s),
`Attack`, `Attack.001` (4.17 s), `Damage`, `Damage.001` (2.08 s), `Dead` (3.00 s), `Dead.001`
(2.08 s), and six camera/empty actions. Rendered and measured, **both `Damage` takes carry no bone
motion** (the head bone does not move across the clip), so the client's flinch uses the first
0.35 s of the death clip instead.

| Role | Take | Length | Timing used |
|---|---|---|---|
| idle | `Idle1.001` | 8.29 s | loop |
| walk | `Walk.001` | 14.58 s | loop |
| attack | `Attack` | 4.17 s | impact (head lowest and furthest forward) at 1.58 s; played at 1.6x, so the wind-up is ~0.99 s, matching the keltir's ~1.0 s server impact delay (P.Atk.Spd 253: 1976 ms interval, impact at half) |
| death | `Dead` | 3.00 s | then held on the last frame |
| flinch | `Dead` 0–0.35 s | 0.35 s | stand-in, see above |

### Stylized Low Poly Nature Lite (optional set dressing)

| Field | Value |
|---|---|
| Publisher | JustCreate |
| Listing | <https://www.fab.com/listings/fef6b0c6-19a4-47bc-913f-c8328910cec4> (free) |
| Licence | **Fab Standard License** (listing licence field "Standard License"; same EULA as above). The owner's acceptance of the Fab EULA on 2026-10-08 covers this download (Vendor/Evidence). |
| Attribution | Not required by the Fab EULA; credited above as a courtesy. |
| Acquired | 2026-10-08, browser download from Fab. Version: not shown (**UNVERIFIED**). |
| Files (sha256) | `stylizednaturelite_fbx.zip` 13,753,681 B `ec242046c8b7b47a71af73572b2fc548bf3ecb5151953a89b1635b97bb9d5e9b` |
| Imported to | `/Game/Vendor/NatureLite/`: `SM_{Tree_01,Rock_01,Plant_01,Plant_02,Mushroom_01,Branch_01}`, `M_NatureLite_{All,Tree}` rebuilt from the `UE/` texture set (the `Unity/` maps are not imported) |
| Use | 15 props placed in `L_TestZone` by `VendorPropScatter` (soft paths; skipped when absent). No collision, no navigation effect. |

### Unreal Engine Mannequin (Manny)

| Field | Value |
|---|---|
| Publisher | Epic Games |
| Source | The installed engine, `$UE_ROOT/Templates/TemplateResources/High/Characters/Content/Mannequins` (UE 5.8.3, CL 58210709). No download. |
| Licence | Unreal Engine EULA (<https://www.unrealengine.com/eula/unreal>); template content is "Examples" (§1), distributable in source or object form (§5(b)). Kept ignored anyway for one consistent vendor policy. |
| Imported to | `/Game/Vendor/Mannequins/` (copied, then moved from the template path with Unreal's rename so references are rewritten): `Meshes/SKM_Manny_Simple`, `SK_Mannequin`, `Rigs/PA_Mannequin`, the Manny materials and textures, and the clips below |
| Clips | idle `Anims/Unarmed/MM_Idle` (7.57 s); walk `Walk/MF_Unarmed_Walk_Fwd` (1.50 s); run `Jog/MF_Unarmed_Jog_Fwd` (1.77 s, played at server speed ≥ 3.5 tiles/s; players move at 5); attack `Attack/MM_Attack_01` (1.00 s, impact = right hand at full reach, 0.40 s); death `Death/MM_Death_Front_01` (1.10 s); flinch = first 0.2 s of the death clip. Root locked. |
| Not used | The legacy UE4 `ThirdPersonRun` named in the asset manifest needs an IK retarget; the native UE5 jog is used as the run instead. |

## What this licence allows (Fab Standard License, summarised; the EULA governs)

- Commercial and private use, modification, and incorporation into a project (§3(a)); not limited
  to Unreal Engine.
- Distributing a project (e.g. the game) that includes the content as an included dependency, with
  end users allowed to use it only as incorporated in object code and restricted from extracting it
  (§4(c)).
- **No standalone redistribution** (§5(a), §6(b)(ii)): the content may be shared only with
  collaborators developing the project (directly or via a *private* repository), who may not pass
  it on. Hence: no vendor files in this public repository, no LFS copies, no public archive.
- No combining with copyleft terms that would relicense the content (§6(a): GPL, LGPL except
  dynamic linking, CC BY-SA); no use in world/level-editing tools or templates that let others
  export it (§6(b)(iii)); no NoAI-tagged content in generative AI training (§16(l)).
