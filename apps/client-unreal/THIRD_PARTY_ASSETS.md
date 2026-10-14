# Third-party assets

The Nightfall client uses this art, but the art is not the property of the project. **None of these files are in this repository.**
The downloads are in `Vendor/`. The imported assets and the assets made from them are in `Content/Vendor/`. Git ignores the two directories
(`apps/client-unreal/.gitignore`: `Vendor/`). To make these files again, do these steps:

1. Get each item under its own terms (below).
2. Put the original downloads in `Vendor/Downloads/`.
3. Run `moon run client-unreal:import-vendor` (see README, "Art and animation").

The repository has only the importer, these records and the soft references that name the imported assets.

The private evidence of the acquisition (the screenshot of the licence acceptance, and the dates) is in the ignored
`Vendor/Evidence/` directory. Do not commit this evidence.

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

The author of the cockatrice asks for the credit ("Free to use with credit (Marko Jäntti)" on the
listing). The credit must also be in the credits or the bundled notices of each build that the project releases.

## Records

### Animated Cockatrice

| Field | Value |
|---|---|
| Publisher / credit | Marko J on Fab; credit name **Marko Jäntti** |
| Listing | <https://www.fab.com/listings/c2c18fbc-48f8-41dc-8817-27bd6a6c9d74> (free) |
| Licence | **Fab Standard License** — the Fab End User License Agreement, last updated 2024-10-01, <https://www.fab.com/eula>. The licence field of the listing shows "Standard License". The repo owner accepted the EULA in the Fab download dialog on 2026-10-08 and said that this is the licence. |
| Attribution | The author asks for it (listing text). The Fab EULA does not ask for attribution. |
| Acquired | 2026-10-08, browser download from Fab, by the repo owner. Listing version: not shown (**UNVERIFIED**). |
| Files (sha256) | `cockatrice.fbx` 11,239,324 B `fcb41efd3771b7c377c3c838223481372092b54633820aa344e2c882e72afd5f`; `coctrice.textures.zip` 11,151,130 B `2f118ee7010b257b5d2c0591fb58f5c88b2059c7b758f3a5ddecadc052cdd715`; `cockatrice.blend` 15,330,019 B `bfdaaaacb5f4b3b47f2d6a336ddd505dbdd9a45430ea4bec6ecd087794c6400b`; `cockatrice.obj` 219,924 B `9f3c0f3105c987a6f497b7ae3cfc67b0cfde57f79da5caa170d5e0bb07c55a9b` (the importer does not use `.blend` and `.obj`) |
| Imported to | `/Game/Vendor/Cockatrice/`: `SK_Cockatrice` (+ `_Skeleton`, its own creature rig, approximately 165 x 115 x 107 cm), `M_Cockatrice` (made again from the base colour, normal, roughness, metallic and emission maps), `T_Cockatrice_*`, `Anims/AS_Cockatrice_{Idle,Walk,Attack,Damage,Death}`, and all other takes as `Anims/AS_Cockatrice_Take_*` |
| Modifications | The FBX takes have new names, by role. The root is locked on the role clips. One material is made again. The import snaps to frames (the `Dead` action is 2.996 s at 24 fps). No retargeting. The keltir (`BP_Keltir`) uses it, at 0.8 scale. |

We examined the contents of the FBX during the import. The labels on the listing were *Idle (flapping wings),
Basic idle, Idle turn head (left), Walk cycle, Attack (peck), Receiving damage, Death*. The FBX has one
AnimStack for each Blender action. Thus, it was not necessary to split frame ranges. The actions are:

- `Idle1.001`, `Idle1.002`, `Idle2`, `Idle2.001`, `Idle3`, `Idle3.001` (8.29 s each)
- `Walk.001`, `Walk.002` (14.58 s)
- `Attack`, `Attack.001` (4.17 s)
- `Damage`, `Damage.001` (2.08 s)
- `Dead` (3.00 s), `Dead.001` (2.08 s)
- six camera/empty actions.

We rendered and measured the takes. **The two `Damage` takes have no bone
motion** (the head bone does not move during the clip). Thus, the flinch of the client uses the first
0.35 s of the death clip.

| Role | Take | Length | Timing used |
|---|---|---|---|
| idle | `Idle1.001` | 8.29 s | loop |
| walk | `Walk.001` | 14.58 s | loop |
| attack | `Attack` | 4.17 s | impact (head at its lowest and most forward position) at 1.58 s. The clip plays at 1.6x. Thus, the wind-up is approximately 0.99 s, which agrees with the approximately 1.0 s server impact delay of the keltir (P.Atk.Spd 253: 1976 ms interval, impact at half) |
| death | `Dead` | 3.00 s | then stays on the last frame |
| flinch | `Dead` 0–0.35 s | 0.35 s | a temporary replacement, see above |

### Stylized Low Poly Nature Lite (optional set dressing)

| Field | Value |
|---|---|
| Publisher | JustCreate |
| Listing | <https://www.fab.com/listings/fef6b0c6-19a4-47bc-913f-c8328910cec4> (free) |
| Licence | **Fab Standard License** (the licence field of the listing shows "Standard License"; the same EULA as above). The owner accepted the Fab EULA on 2026-10-08, and that acceptance applies to this download (Vendor/Evidence). |
| Attribution | The Fab EULA does not ask for it. The credit above is a courtesy. |
| Acquired | 2026-10-08, browser download from Fab. Version: not shown (**UNVERIFIED**). |
| Files (sha256) | `stylizednaturelite_fbx.zip` 13,753,681 B `ec242046c8b7b47a71af73572b2fc548bf3ecb5151953a89b1635b97bb9d5e9b` |
| Imported to | `/Game/Vendor/NatureLite/`: `SM_{Tree_01,Rock_01,Plant_01,Plant_02,Mushroom_01,Branch_01}`, and `M_NatureLite_{All,Tree}` made again from the `UE/` texture set (the import does not include the `Unity/` maps) |
| Use | `VendorPropScatter` puts 15 props in `L_TestZone` (soft paths; it does not put them if they are not there). No collision, no effect on navigation. |

### Unreal Engine Mannequin (Manny)

| Field | Value |
|---|---|
| Publisher | Epic Games |
| Source | The installed engine, `$UE_ROOT/Templates/TemplateResources/High/Characters/Content/Mannequins` (UE 5.8.3, CL 58210709). No download. |
| Licence | Unreal Engine EULA (<https://www.unrealengine.com/eula/unreal>). Template content is "Examples" (§1), and you can distribute it in source or object form (§5(b)). The project ignores it in git, because all vendor content follows one policy. |
| Imported to | `/Game/Vendor/Mannequins/` (copied, then moved from the template path with the rename function of Unreal, so that Unreal writes the references again): `Meshes/SKM_Manny_Simple`, `SK_Mannequin`, `Rigs/PA_Mannequin`, the Manny materials and textures, and the clips below |
| Clips | idle `Anims/Unarmed/MM_Idle` (7.57 s); walk `Walk/MF_Unarmed_Walk_Fwd` (1.50 s); run `Jog/MF_Unarmed_Jog_Fwd` (1.77 s, plays at server speed ≥ 3.5 tiles/s; players move at 5); attack `Attack/MM_Attack_01` (1.00 s, impact = right hand at full reach, 0.40 s); death `Death/MM_Death_Front_01` (1.10 s); flinch = first 0.2 s of the death clip. Root locked. |
| Not used | The legacy UE4 `ThirdPersonRun` in the asset manifest must have an IK retarget. Thus, the project uses the native UE5 jog as the run. |

## What this licence allows (Fab Standard License, summarised; the EULA governs)

The EULA is the authority. This is a summary:

- You can use the content for commercial and private use. You can change it and put it into a project (§3(a)). This is not
  limited to Unreal Engine.
- You can distribute a project (for example, the game) that has the content as an included dependency. End users
  can use the content only as it is in the object code, and they must not extract it
  (§4(c)).
- **No standalone redistribution** (§5(a), §6(b)(ii)): you can share the content only with
  collaborators who develop the project (directly or through a *private* repository). These collaborators must not give
  it to other persons. Thus: no vendor files in this public repository, no LFS copies, no public archive.
- Do not combine the content with copyleft terms that change the licence of the content (§6(a): GPL, LGPL except
  dynamic linking, CC BY-SA). Do not use it in world/level-editing tools or templates that let other persons
  export it (§6(b)(iii)). Do not use NoAI-tagged content in generative AI training (§16(l)).
