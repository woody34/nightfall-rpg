"""Creates the Story 6.2 content assets. Run headless with Scripts/create-content.sh (full editor,
-nullrhi: spawning actors into a level does not work in the pythonscript commandlet).

Idempotent: the wrapper deletes Content/{Input,Blueprints,UI,Maps} first and this recreates them, so rerunning after a C++ change gives the same
result. Hand edits made in the editor to these assets are lost on rerun; once someone starts
editing them by hand, stop running this script.

  /Game/Input/IA_ClickMove           InputAction (bool), left mouse via IMC_Default
  /Game/Input/IMC_Default            InputMappingContext
  /Game/Blueprints/BP_RemoteEntity   RemoteEntityActor with a cylinder placeholder
  /Game/Blueprints/BP_NightfallPC    NightfallPlayerController with IMC_Default + IA_ClickMove
  /Game/Blueprints/BP_NightfallGameMode  NightfallGameMode: BP_NightfallPC, EntityClass = BP_RemoteEntity
  /Game/UI/WBP_Login                 NightfallLoginScreen (empty tree: the C++ default layout)
  /Game/Maps/L_Login                 default map; GameMode NightfallLoginGameMode
  /Game/Maps/L_TestZone              256x256 tiles (25600 cm) ground, nav bounds, sun, sky
"""

import unreal

TILES = 256
UNITS_PER_TILE = 100.0
SIZE = TILES * UNITS_PER_TILE  # 25600 cm
CENTER = unreal.Vector(SIZE / 2, SIZE / 2, 0.0)

tools = unreal.AssetToolsHelpers.get_asset_tools()
assets = unreal.EditorAssetLibrary
levels = unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)
actors = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)


def log(msg):
    unreal.log("create_content: " + msg)




def create(name, folder, asset_class, factory):
    asset = tools.create_asset(name, folder, asset_class, factory)
    if asset is None:
        raise RuntimeError("could not create " + folder + "/" + name)
    return asset


def blueprint(name, folder, parent):
    factory = unreal.BlueprintFactory()
    factory.set_editor_property("parent_class", parent)
    bp = create(name, folder, unreal.Blueprint, factory)
    unreal.BlueprintEditorLibrary.compile_blueprint(bp)
    return bp


def cdo(bp):
    return unreal.get_default_object(unreal.BlueprintEditorLibrary.generated_class(bp))


def save(asset):
    if not assets.save_loaded_asset(asset, only_if_is_dirty=False):
        raise RuntimeError("could not save " + asset.get_path_name())


def main():
    # Input ------------------------------------------------------------------------------------------
    click = create("IA_ClickMove", "/Game/Input", unreal.InputAction, unreal.InputAction_Factory())
    click.set_editor_property("value_type", unreal.InputActionValueType.BOOLEAN)
    save(click)

    imc = create("IMC_Default", "/Game/Input", unreal.InputMappingContext, unreal.InputMappingContext_Factory())
    left_mouse = unreal.Key()
    left_mouse.set_editor_property("key_name", "LeftMouseButton")
    imc.map_key(click, left_mouse)
    save(imc)
    log("input assets")

    # Blueprints -------------------------------------------------------------------------------------
    remote = blueprint("BP_RemoteEntity", "/Game/Blueprints", unreal.RemoteEntityActor)
    cdo(remote).get_editor_property("body").set_editor_property(
        "static_mesh", unreal.load_asset("/Engine/BasicShapes/Cylinder.Cylinder"))
    unreal.BlueprintEditorLibrary.compile_blueprint(remote)
    save(remote)

    pc = blueprint("BP_NightfallPC", "/Game/Blueprints", unreal.NightfallPlayerController)
    cdo(pc).set_editor_property("default_mapping_context", imc)
    cdo(pc).set_editor_property("click_move_action", click)
    unreal.BlueprintEditorLibrary.compile_blueprint(pc)
    save(pc)

    gm = blueprint("BP_NightfallGameMode", "/Game/Blueprints", unreal.NightfallGameMode)
    cdo(gm).set_editor_property("player_controller_class", unreal.BlueprintEditorLibrary.generated_class(pc))
    cdo(gm).set_editor_property("entity_class", unreal.BlueprintEditorLibrary.generated_class(remote))
    unreal.BlueprintEditorLibrary.compile_blueprint(gm)
    save(gm)
    log("blueprints")

    widget_factory = unreal.WidgetBlueprintFactory()
    widget_factory.set_editor_property("parent_class", unreal.NightfallLoginScreen)
    login_widget = create("WBP_Login", "/Game/UI", unreal.WidgetBlueprint, widget_factory)
    unreal.BlueprintEditorLibrary.compile_blueprint(login_widget)
    save(login_widget)
    log("widget")


    # Maps -------------------------------------------------------------------------------------------
    def new_map(path, game_mode):
        if not levels.new_level(path):
            raise RuntimeError("could not create " + path)
        world = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_editor_world()
        world.get_world_settings().set_editor_property("default_game_mode", game_mode)
        return world


    def spawn(actor_class, location, rotation=unreal.Rotator(0, 0, 0), label=None):
        actor = actors.spawn_actor_from_class(actor_class, location, rotation)
        if label:
            actor.set_actor_label(label)
        return actor


    new_map("/Game/Maps/L_TestZone", unreal.BlueprintEditorLibrary.generated_class(gm))

    ground = actors.spawn_actor_from_object(unreal.load_asset("/Engine/BasicShapes/Plane.Plane"), CENTER)
    ground.set_actor_label("Ground")
    ground.set_actor_scale3d(unreal.Vector(TILES, TILES, 1.0))  # the engine plane is 100 x 100 cm

    nav = spawn(unreal.NavMeshBoundsVolume, CENTER, label="NavBounds")
    nav.set_actor_scale3d(unreal.Vector(SIZE / 200.0, SIZE / 200.0, 5.0))  # default brush is a 200 cm cube

    sun = spawn(unreal.DirectionalLight, CENTER + unreal.Vector(0, 0, 2000), unreal.Rotator(0, -50, -40), "Sun")
    sun.get_component_by_class(unreal.DirectionalLightComponent).set_editor_property("atmosphere_sun_light", True)
    spawn(unreal.SkyAtmosphere, unreal.Vector(0, 0, 0), label="SkyAtmosphere")
    sky_light = spawn(unreal.SkyLight, CENTER + unreal.Vector(0, 0, 1000), label="SkyLight")
    sky_light_component = sky_light.get_component_by_class(unreal.SkyLightComponent)
    sky_light_component.set_editor_property("mobility", unreal.ComponentMobility.MOVABLE)
    sky_light_component.set_editor_property("real_time_capture", True)
    spawn(unreal.ExponentialHeightFog, CENTER, label="Fog")
    spawn(unreal.PlayerStart, CENTER + unreal.Vector(0, 0, 120), label="PlayerStart")
    if not levels.save_current_level():
        raise RuntimeError("could not save L_TestZone")
    log("L_TestZone")

    new_map("/Game/Maps/L_Login", unreal.NightfallLoginGameMode)
    if not levels.save_current_level():
        raise RuntimeError("could not save L_Login")
    log("L_Login")

    log("done")


try:
    main()
except Exception as error:  # noqa: BLE001 - report anything, then quit with a marker the wrapper checks
    unreal.log_error("create_content: FAILED: " + repr(error))
    raise
finally:
    unreal.SystemLibrary.quit_editor()
