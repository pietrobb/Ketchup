import bpy
from pathlib import Path

source = Path(r"C:\Sources8\Ketchup\artifacts\blender\garden-studio-colored.glb")
destination = Path(r"C:\Sources8\Ketchup\artifacts\blender\garden-studio-colored.blend")

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
