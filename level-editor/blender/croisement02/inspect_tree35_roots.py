"""Read-only close root views, restricted to the approved tree35 wood."""
import json,sys,math
from pathlib import Path
import bpy
from PIL import Image
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json
from render_slots import acquire,release
from render_multiview_asset import render

def main():
    worker=tree_workspace(35);out=OUT/'tree35-root-research/baseline-v2';out.mkdir(parents=True,exist_ok=False);digest=sha(worker/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
    wood=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==worker.name and o.get('projection_component')!='crown']
    points=[o.matrix_world@v.co for o in wood for v in o.data.vertices];low=[v for v in points if v.z<35];bounds=[[min(v[i] for v in low),max(v[i] for v in low)] for i in range(3)];target=Vector([sum(b)/2 for b in bounds]);scale=max(b[1]-b[0] for b in bounds)*1.25
    packet=json.loads((worker/'modified/views.json').read_text());packet['object_names']=[o.name for o in wood];packet.pop('render_object_names',None);packet['tile_size']=[320,320];packet['source_blend']=str(worker/'model.blend')
    for index,view in enumerate(packet['views']):
        az=math.radians(index*45);e=math.radians(35);position=target+Vector((math.sin(az)*math.cos(e),-math.cos(az)*math.cos(e),math.sin(e)))*5000;rotation=(target-position).to_track_quat('-Z','Y').to_euler();matrix=rotation.to_matrix().to_4x4();matrix.translation=position
        view.update(camera_location=list(position),camera_rotation_euler=list(rotation),camera_matrix_world=[list(r) for r in matrix],ortho_scale=scale,crop=dict(width=320,height=320))
    for obj in bpy.context.scene.objects:
        if obj.type=='MESH' and obj.get('asset_group')==worker.name:obj.hide_render=obj not in wood
    write_json(out/'views.json',packet);scene=bpy.data.scenes[packet['scene_name']];scene.render.engine='CYCLES';scene.cycles.samples=8
    render(out/'views.json',out/'views',modes=('textured','solid'),width=320)
    for mode in ['textured','solid']:
        sheet=Image.new('RGB',(1280,640))
        for i in range(8):sheet.paste(Image.open(out/f'views/view-{i}-{mode}.png'),((i%4)*320,(i//4)*320))
        sheet.save(out/f'{mode}.png')
    write_json(out/'evidence.json',dict(worker=str(worker),model_sha256=digest,root_bounds=bounds,wood=[dict(name=o.name,vertices=len(o.data.vertices),min_local_z=min(v.co.z for v in o.data.vertices),max_local_z=max(v.co.z for v in o.data.vertices)) for o in wood],source_model_unchanged=sha(worker/'model.blend')==digest))

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
