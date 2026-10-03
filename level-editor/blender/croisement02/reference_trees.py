"""Inspect only the two user-selected Leicester tree references."""
import json
import math
import sys
from pathlib import Path
import bpy
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/croisement02-refinement/tree-references'
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from render_slots import acquire
from setup_map import fit_camera
from render_views import render_views
from evidence_io import sha


def main():
    acquire()
    OUT.mkdir(exist_ok=False)
    reports=[]
    for key in ('moat-bank-tree', 'southeast-cottage-tree'):
        source = ROOT / f'level-editor/library/3d-assets/leicester/leicester-{key}/model.glb'
        scene=bpy.data.scenes.new(key);bpy.context.window.scene=scene
        bpy.ops.import_scene.gltf(filepath=str(source))
        objects=[o for o in scene.objects if o.type=='MESH']
        points=[o.matrix_world@v.co for o in objects for v in o.data.vertices]
        bounds=[[min(v[i] for v in points),max(v[i] for v in points)] for i in range(3)]
        center=Vector(tuple(sum(b)/2 for b in bounds));views={}
        for label,yaw,pitch in [('front',0,35),('side',90,35),('back',180,35),('plan',0,90)]:
            a,e=math.radians(yaw),math.radians(pitch)
            data=bpy.data.cameras.new(label);data.type='ORTHO'
            camera=bpy.data.objects.new(label,data);scene.collection.objects.link(camera)
            camera.location=center+Vector((math.sin(a)*math.cos(e),-math.cos(a)*math.cos(e),math.sin(e)))*3000
            camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler()
            fit_camera(camera,objects,1)
            views[label]=camera.name
        scene.render.engine='CYCLES';scene.cycles.samples=4
        scene.render.resolution_x=512;scene.render.resolution_y=512
        scene.view_settings.view_transform='Standard';scene.world=bpy.data.worlds.new(key)
        scene.world.color=(.15,.15,.15)
        render_views(scene.name,views,OUT/key,width=512)
        reports.append(dict(id='leicester-'+key,source=str(source),sha256=sha(source),bounds=bounds,
            width=bounds[0][1]-bounds[0][0],depth=bounds[1][1]-bounds[1][0],
            meshes=[dict(name=o.name,vertices=len(o.data.vertices),faces=len(o.data.polygons),materials=len(o.data.materials)) for o in objects]))
    (OUT/'reference-audit.json').write_text(json.dumps(reports,indent=2)+'\n')
    print(json.dumps(reports))

if __name__=='__main__':main()
