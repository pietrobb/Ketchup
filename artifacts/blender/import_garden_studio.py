"""Imports the garden studio GLB into Blender and saves it as a .blend beside it.

Run with `blender --background --python import_garden_studio.py`. The .blend is
this script's output; it is not kept in the repository (the sample-files release
holds a copy)."""

import bpy
from pathlib import Path

here = Path(__file__).resolve().parent
source = here / "garden-studio-colored.glb"
destination = here / "garden-studio-colored.blend"

bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.delete(use_global=False)
result = bpy.ops.import_scene.gltf(filepath=str(source))
if "FINISHED" not in result:
    raise RuntimeError(f"GLB import failed: {result}")

objects = list(bpy.context.scene.objects)
mesh_objects = [obj for obj in objects if obj.type == "MESH"]
materials = {material.name for obj in mesh_objects for material in obj.data.materials if material}
if len(mesh_objects) != 140:
    raise RuntimeError(f"Expected 140 mesh objects, imported {len(mesh_objects)}")

bpy.context.scene["ketchup_source_glb"] = str(source)
bpy.context.scene["ketchup_imported_mesh_objects"] = len(mesh_objects)
bpy.context.scene["ketchup_imported_materials"] = len(materials)
bpy.ops.wm.save_as_mainfile(filepath=str(destination))
print(f"KETCHUP_GLTF_IMPORT_OK objects={len(objects)} meshes={len(mesh_objects)} materials={len(materials)} blend={destination}")
