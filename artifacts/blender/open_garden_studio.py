import bpy


def frame_imported_scene():
    bpy.ops.object.select_all(action="SELECT")
    for window in bpy.context.window_manager.windows:
        screen = window.screen
        for area in screen.areas:
            if area.type != "VIEW_3D":
                continue
            region = next((item for item in area.regions if item.type == "WINDOW"), None)
            if region is None:
                continue
            area.spaces.active.shading.type = "MATERIAL"
            with bpy.context.temp_override(window=window, screen=screen, area=area, region=region):
                bpy.ops.view3d.view_selected(use_all_regions=False)
    return None


bpy.app.timers.register(frame_imported_scene, first_interval=0.75)
