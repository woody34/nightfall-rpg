"""Imports the Phase 1 vendor art into Content/Vendor (git-ignored). Run headless with
Scripts/import-vendor.sh (moon: client-unreal:import-vendor), never by hand in the editor.

Inputs (never modified, never committed; see THIRD_PARTY_ASSETS.md):
  $NIGHTFALL_VENDOR_DIR/Cockatrice/cockatrice.fbx, coctrice.textures.zip   (Fab, Marko Jäntti)
  $NIGHTFALL_VENDOR_DIR/NatureLite/stylizednaturelite_fbx.zip              (Fab, JustCreate; optional)
  $NIGHTFALL_MANNY_STAGED: the wrapper copies Epic's template Manny into /Game/Characters/Mannequins
                           before the editor starts; this script moves it to /Game/Vendor/Mannequins.

Outputs (stable paths; the C++ proxies and BP_Keltir reference them by soft path):
  /Game/Vendor/Cockatrice/SK_Cockatrice (+ _Skeleton), M_Cockatrice, T_Cockatrice_*
  /Game/Vendor/Cockatrice/Anims/AS_Cockatrice_{Idle,Walk,Attack,Damage,Death} (+ every other FBX take)
  /Game/Vendor/Mannequins/...  SKM_Manny_Simple, MM_Idle, MF_Unarmed_Walk_Fwd, MF_Unarmed_Jog_Fwd,
                               MM_Attack_01, MM_Death_Front_01 (template paths below /Mannequins kept)
  /Game/Vendor/NatureLite/SM_*, M_NatureLite_*, T_*

Idempotent: the wrapper deletes Content/Vendor/{Cockatrice,NatureLite,Mannequins} first, so a rerun
reproduces the same assets at the same paths. The last lines of the log list every role clip.
"""

import os
import tempfile
import zipfile

import unreal

VENDOR = os.environ.get("NIGHTFALL_VENDOR_DIR", "")
WITH_MANNY = os.environ.get("NIGHTFALL_MANNY_STAGED", "") == "1"

COCKATRICE = "/Game/Vendor/Cockatrice"
NATURE = "/Game/Vendor/NatureLite"
MANNY_STAGED = "/Game/Characters/Mannequins"
MANNY = "/Game/Vendor/Mannequins"

# FBX take (Blender action) -> role. The FBX holds one AnimStack per Blender action, not a combined
# take, so no frame-range splitting is needed. The ".001"/".002" stacks are Blender duplicates kept
# by the exporter; the chosen ones are the ones whose length and motion matched the listing's
# labels when previewed (see THIRD_PARTY_ASSETS.md for the full inventory).
COCKATRICE_ROLES = {
    "Idle": "Idle1_001",
    "Walk": "Walk_001",
    "Attack": "Attack",
    "Damage": "Damage",
    "Death": "Dead",
}

tools = unreal.AssetToolsHelpers.get_asset_tools()
assets = unreal.EditorAssetLibrary
mel = unreal.MaterialEditingLibrary


def log(msg):
    unreal.log("import_vendor: " + msg)


def save_dir(path):
    if assets.does_directory_exist(path):
        assets.save_directory(path, only_if_is_dirty=False, recursive=True)


def import_files(files, dest, options=None, name=""):
    tasks = []
    for f in files:
        t = unreal.AssetImportTask()
        t.set_editor_property("filename", f)
        t.set_editor_property("destination_path", dest)
        t.set_editor_property("destination_name", name)
        t.set_editor_property("automated", True)
        t.set_editor_property("replace_existing", True)
        t.set_editor_property("save", False)
        if options is not None:
            t.set_editor_property("options", options)
        tasks.append(t)
    tools.import_asset_tasks(tasks)
    out = []
    for t in tasks:
        paths = list(t.get_editor_property("imported_object_paths"))
        if not paths:
            raise RuntimeError("nothing imported from " + t.get_editor_property("filename"))
        out.extend(paths)
    return out


def import_texture(png, dest, name, normal=False, linear=False):
    path = import_files([png], dest, name=name)[0]
    tex = unreal.load_asset(path)
    if normal:
        tex.set_editor_property("compression_settings", unreal.TextureCompressionSettings.TC_NORMALMAP)
        tex.set_editor_property("srgb", False)
    elif linear:
        tex.set_editor_property("compression_settings", unreal.TextureCompressionSettings.TC_MASKS)
        tex.set_editor_property("srgb", False)
    return tex


def texture_param(material, tex, x, y, sampler=None):
    node = mel.create_material_expression(material, unreal.MaterialExpressionTextureSample, x, y)
    node.set_editor_property("texture", tex)
    if sampler is not None:
        node.set_editor_property("sampler_type", sampler)
    return node


def unzip(archive, members_prefix=""):
    out = tempfile.mkdtemp(prefix="nightfall-vendor-")
    with zipfile.ZipFile(archive) as z:
        for m in z.namelist():
            if m.startswith(members_prefix) and not m.endswith("/"):
                z.extract(m, out)
    return out


def skeletal_fbx_options(skeleton=None):
    ui = unreal.FbxImportUI()
    ui.set_editor_property("automated_import_should_detect_type", False)
    ui.set_editor_property("original_import_type", unreal.FBXImportType.FBXIT_SKELETAL_MESH)
    ui.set_editor_property("mesh_type_to_import", unreal.FBXImportType.FBXIT_SKELETAL_MESH)
    ui.set_editor_property("import_mesh", skeleton is None)
    ui.set_editor_property("import_as_skeletal", True)
    ui.set_editor_property("import_animations", True)
    ui.set_editor_property("import_materials", False)
    ui.set_editor_property("import_textures", False)
    ui.set_editor_property("create_physics_asset", False)
    if skeleton is not None:
        ui.set_editor_property("skeleton", skeleton)
    sk = ui.get_editor_property("skeletal_mesh_import_data")
    sk.set_editor_property("import_morph_targets", False)
    sk.set_editor_property("convert_scene", True)
    an = ui.get_editor_property("anim_sequence_import_data")
    an.set_editor_property("animation_length", unreal.FBXAnimationLengthImportType.FBXALIT_EXPORTED_TIME)
    an.set_editor_property("import_bone_tracks", True)
    an.set_editor_property("remove_redundant_keys", False)
    # Blender exports some action ranges on sub-frame boundaries (Dead is 2.996 s at 24 fps).
    an.set_editor_property("snap_to_closest_frame_boundary", True)
    return ui


# --- Cockatrice ------------------------------------------------------------------------------------

def import_cockatrice():
    src = os.path.join(VENDOR, "Cockatrice")
    fbx = os.path.join(src, "cockatrice.fbx")
    if not os.path.isfile(fbx):
        raise RuntimeError("missing " + fbx)

    imported = import_files([fbx], COCKATRICE, skeletal_fbx_options(), name="SK_Cockatrice")
    mesh = None
    for p in imported:
        a = unreal.load_asset(p)
        if isinstance(a, unreal.SkeletalMesh):
            mesh = a
    if mesh is None:
        raise RuntimeError("cockatrice.fbx produced no skeletal mesh: " + ", ".join(imported))
    skeleton = mesh.get_editor_property("skeleton")
    log("cockatrice mesh " + mesh.get_path_name() + " skeleton " + skeleton.get_path_name())

    # Every take as its own AnimSequence, moved under Anims/ with a stable name.
    assets.make_directory(COCKATRICE + "/Anims")
    takes = {}
    for p in assets.list_assets(COCKATRICE, recursive=False):
        a = unreal.load_asset(p)
        if isinstance(a, unreal.AnimSequence):
            take = a.get_name().split("_Anim_", 1)[-1]
            takes[take] = a
    if not takes:
        raise RuntimeError("cockatrice.fbx produced no animations")
    for take, seq in sorted(takes.items()):
        log("cockatrice take %s: %.3f s, %d keys" % (
            take, unreal.AnimationLibrary.get_sequence_length(seq),
            unreal.AnimationLibrary.get_num_keys(seq)))

    for role, take in COCKATRICE_ROLES.items():
        if take not in takes:
            raise RuntimeError("cockatrice take %s for role %s not found; have %s" % (take, role, sorted(takes)))
    for take, seq in takes.items():
        role = next((r for r, t in COCKATRICE_ROLES.items() if t == take), None)
        name = "AS_Cockatrice_" + role if role else "AS_Cockatrice_Take_" + take
        if not assets.rename_asset(seq.get_path_name(), COCKATRICE + "/Anims/" + name):
            raise RuntimeError("could not rename " + seq.get_path_name())
    for role in COCKATRICE_ROLES:
        seq = unreal.load_asset(COCKATRICE + "/Anims/AS_Cockatrice_" + role)
        # The server moves the entity; any root translation in a clip must not move the mesh.
        seq.set_editor_property("force_root_lock", True)
        seq.set_editor_property("enable_root_motion", False)

    # Textures and one material rebuilt from the supplied maps.
    tex_dir = unzip(os.path.join(src, "coctrice.textures.zip"))
    t = COCKATRICE + "/Textures"
    base = import_texture(os.path.join(tex_dir, "Material_BaseColor.png"), t, "T_Cockatrice_BaseColor")
    normal = import_texture(os.path.join(tex_dir, "Material_Normal.png"), t, "T_Cockatrice_Normal", normal=True)
    rough = import_texture(os.path.join(tex_dir, "Material_Roughness.png"), t, "T_Cockatrice_Roughness", linear=True)
    metal = import_texture(os.path.join(tex_dir, "Material_Metallic.png"), t, "T_Cockatrice_Metallic", linear=True)
    emissive = import_texture(os.path.join(tex_dir, "Material_Emission.png"), t, "T_Cockatrice_Emission")

    material = tools.create_asset("M_Cockatrice", COCKATRICE, unreal.Material, unreal.MaterialFactoryNew())
    # Without the usage flag a cooked/-game build renders the default material on the skeletal mesh.
    material.set_editor_property("used_with_skeletal_mesh", True)
    mel.connect_material_property(texture_param(material, base, -400, -200), "RGB", unreal.MaterialProperty.MP_BASE_COLOR)
    mel.connect_material_property(texture_param(material, normal, -400, 0, unreal.MaterialSamplerType.SAMPLERTYPE_NORMAL), "RGB", unreal.MaterialProperty.MP_NORMAL)
    mel.connect_material_property(texture_param(material, rough, -400, 200, unreal.MaterialSamplerType.SAMPLERTYPE_MASKS), "R", unreal.MaterialProperty.MP_ROUGHNESS)
    mel.connect_material_property(texture_param(material, metal, -400, 400, unreal.MaterialSamplerType.SAMPLERTYPE_MASKS), "R", unreal.MaterialProperty.MP_METALLIC)
    mel.connect_material_property(texture_param(material, emissive, -400, 600), "RGB", unreal.MaterialProperty.MP_EMISSIVE_COLOR)
    mel.recompile_material(material)

    mats = mesh.get_editor_property("materials")
    for i in range(len(mats)):
        m = mats[i]
        m.set_editor_property("material_interface", material)
        mats[i] = m
    mesh.set_editor_property("materials", mats)
    save_dir(COCKATRICE)

    bounds = mesh.get_bounds()
    log("cockatrice bounds origin %s extent %s, %d materials" % (bounds.origin, bounds.box_extent, len(mats)))


# --- Manny (Epic template) -------------------------------------------------------------------------

MANNY_ROLES = {
    "Idle": "Anims/Unarmed/MM_Idle",
    "Walk": "Anims/Unarmed/Walk/MF_Unarmed_Walk_Fwd",
    "Run": "Anims/Unarmed/Jog/MF_Unarmed_Jog_Fwd",
    "Attack": "Anims/Unarmed/Attack/MM_Attack_01",
    "Death": "Anims/Death/MM_Death_Front_01",
}


def move_manny():
    if not assets.does_directory_exist(MANNY_STAGED):
        raise RuntimeError("Manny was not staged at " + MANNY_STAGED)
    # Renaming fixes every reference inside the copied packages and leaves redirectors behind.
    if not assets.rename_directory(MANNY_STAGED, MANNY):
        raise RuntimeError("could not move " + MANNY_STAGED + " to " + MANNY)
    registry = unreal.AssetRegistryHelpers.get_asset_registry()
    redirectors = [a.get_asset() for a in registry.get_assets_by_path("/Game/Characters", recursive=True)
                   if str(a.asset_class_path.asset_name) == "ObjectRedirector"]
    if redirectors:
        tools.fixup_referencers(redirectors)
    save_dir(MANNY)
    for role, rel in MANNY_ROLES.items():
        seq = unreal.load_asset(MANNY + "/" + rel)
        if seq is None:
            raise RuntimeError("Manny %s clip missing: %s" % (role, rel))
        seq.set_editor_property("force_root_lock", True)
        seq.set_editor_property("enable_root_motion", False)
    save_dir(MANNY)
    log("manny moved to " + MANNY)


# --- Nature Lite (optional) ------------------------------------------------------------------------

def import_nature():
    archive = os.path.join(VENDOR, "NatureLite", "stylizednaturelite_fbx.zip")
    if not os.path.isfile(archive):
        log("NatureLite archive absent; skipped (optional)")
        return
    root = unzip(archive)
    ui = unreal.FbxImportUI()
    ui.set_editor_property("automated_import_should_detect_type", False)
    ui.set_editor_property("mesh_type_to_import", unreal.FBXImportType.FBXIT_STATIC_MESH)
    ui.set_editor_property("import_mesh", True)
    ui.set_editor_property("import_as_skeletal", False)
    ui.set_editor_property("import_materials", False)
    ui.set_editor_property("import_textures", False)
    ui.set_editor_property("import_animations", False)
    sm = ui.get_editor_property("static_mesh_import_data")
    sm.set_editor_property("combine_meshes", True)
    sm.set_editor_property("generate_lightmap_u_vs", True)
    sm.set_editor_property("auto_generate_collision", False)
    fbx = sorted(os.path.join(root, f) for f in os.listdir(root) if f.lower().endswith(".fbx"))
    meshes = [unreal.load_asset(p) for p in import_files(fbx, NATURE, ui)]
    meshes = [m for m in meshes if isinstance(m, unreal.StaticMesh)]

    ue = os.path.join(root, "UE")
    t = NATURE + "/Textures"
    all_base = import_texture(os.path.join(ue, "All_lp_All_01_BaseColor.png"), t, "T_NatureLite_All_BaseColor")
    all_n = import_texture(os.path.join(ue, "All_lp_All_01_Normal.png"), t, "T_NatureLite_All_Normal", normal=True)
    all_orm = import_texture(os.path.join(ue, "All_lp_All_01_OcclusionRoughnessMetallic.png"), t, "T_NatureLite_All_ORM", linear=True)
    tree_base = import_texture(os.path.join(ue, "Tree_01_lp_Tree_BaseColor.png"), t, "T_NatureLite_Tree_BaseColor")
    tree_n = import_texture(os.path.join(ue, "Tree_01_lp_Tree_Normal.png"), t, "T_NatureLite_Tree_Normal", normal=True)
    leaf_alpha = import_texture(os.path.join(root, "T_TreeAlpha_02.png"), t, "T_NatureLite_TreeAlpha", linear=True)

    def make_material(name, base, normal, orm=None, mask=None):
        m = tools.create_asset(name, NATURE, unreal.Material, unreal.MaterialFactoryNew())
        if mask is not None:
            # Leaf cards: cut out by the alpha map, lit from both sides. The tree atlas has no leaf
            # colour (the pack's own material tints them), so a flat stylised green.
            m.set_editor_property("blend_mode", unreal.BlendMode.BLEND_MASKED)
            m.set_editor_property("two_sided", True)
            mel.connect_material_property(texture_param(m, mask, -400, 400, unreal.MaterialSamplerType.SAMPLERTYPE_MASKS), "R", unreal.MaterialProperty.MP_OPACITY_MASK)
            green = mel.create_material_expression(m, unreal.MaterialExpressionConstant3Vector, -400, -400)
            green.set_editor_property("constant", unreal.LinearColor(0.12, 0.30, 0.06, 1.0))
            mel.connect_material_property(green, "", unreal.MaterialProperty.MP_BASE_COLOR)
            mel.recompile_material(m)
            return m
        mel.connect_material_property(texture_param(m, base, -400, -200), "RGB", unreal.MaterialProperty.MP_BASE_COLOR)
        mel.connect_material_property(texture_param(m, normal, -400, 0, unreal.MaterialSamplerType.SAMPLERTYPE_NORMAL), "RGB", unreal.MaterialProperty.MP_NORMAL)
        if orm is not None:
            s = texture_param(m, orm, -400, 200, unreal.MaterialSamplerType.SAMPLERTYPE_MASKS)
            mel.connect_material_property(s, "R", unreal.MaterialProperty.MP_AMBIENT_OCCLUSION)
            mel.connect_material_property(s, "G", unreal.MaterialProperty.MP_ROUGHNESS)
            mel.connect_material_property(s, "B", unreal.MaterialProperty.MP_METALLIC)
        else:
            rough = mel.create_material_expression(m, unreal.MaterialExpressionConstant, -200, 200)
            rough.set_editor_property("r", 0.85)
            mel.connect_material_property(rough, "", unreal.MaterialProperty.MP_ROUGHNESS)
        mel.recompile_material(m)
        return m

    m_all = make_material("M_NatureLite_All", all_base, all_n, all_orm)
    m_tree = make_material("M_NatureLite_Tree", tree_base, tree_n)
    m_leaves = make_material("M_NatureLite_Leaves", tree_base, tree_n, mask=leaf_alpha)
    for mesh in meshes:
        mats = mesh.get_editor_property("static_materials")
        for i in range(len(mats)):
            slot = mats[i]
            slot_name = str(slot.get_editor_property("material_slot_name")).lower()
            slot.set_editor_property("material_interface",
                                     m_leaves if "leaves" in slot_name else m_tree if "tree" in slot_name else m_all)
            mats[i] = slot
        mesh.set_editor_property("static_materials", mats)
        log("nature mesh %s slots %s" % (mesh.get_name(), [str(s.get_editor_property("material_slot_name")) for s in mats]))
    save_dir(NATURE)
    log("nature: %d meshes" % len(meshes))


# --- Verification ----------------------------------------------------------------------------------

def verify():
    """Loads every role asset from disk and prints one line per clip; fails on anything missing."""
    required = {
        "cockatrice mesh": COCKATRICE + "/SK_Cockatrice",
        "cockatrice material": COCKATRICE + "/M_Cockatrice",
    }
    for role in COCKATRICE_ROLES:
        required["cockatrice " + role.lower()] = COCKATRICE + "/Anims/AS_Cockatrice_" + role
    if WITH_MANNY:
        required["manny mesh"] = MANNY + "/Meshes/SKM_Manny_Simple"
        for role, rel in MANNY_ROLES.items():
            required["manny " + role.lower()] = MANNY + "/" + rel
    missing = []
    for label, path in sorted(required.items()):
        a = unreal.load_asset(path)
        if a is None:
            missing.append(label + " " + path)
            continue
        if isinstance(a, unreal.AnimSequence):
            log("verify %-20s %s %.3f s" % (label, path, unreal.AnimationLibrary.get_sequence_length(a)))
        else:
            log("verify %-20s %s" % (label, path))
    if missing:
        raise RuntimeError("missing after import: " + "; ".join(missing))


def main():
    if not VENDOR or not os.path.isdir(VENDOR):
        raise RuntimeError("NIGHTFALL_VENDOR_DIR does not exist: %r" % VENDOR)
    # The legacy FBX importer honours FbxImportUI exactly (take names, no Interchange pipeline
    # assets), which keeps the output paths stable between runs.
    unreal.SystemLibrary.execute_console_command(None, "Interchange.FeatureFlags.Import.FBX 0")
    import_cockatrice()
    if WITH_MANNY:
        move_manny()
    import_nature()
    verify()
    log("done")


try:
    main()
except Exception as error:  # noqa: BLE001 - report anything, then quit with a marker the wrapper checks
    unreal.log_error("import_vendor: FAILED: " + repr(error))
    raise
finally:
    unreal.SystemLibrary.quit_editor()
