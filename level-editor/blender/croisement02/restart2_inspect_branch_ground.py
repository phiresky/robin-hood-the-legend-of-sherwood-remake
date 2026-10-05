"""Inspect lower tree wood against its unchanged bank or flat ground receiver."""
import argparse,json,sys,math
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace,scenery_workspace
from evidence_io import sha,write_json
from render_slots import acquire,release
from render_multiview_asset import render

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--mask',type=int,choices=(43,45,46),default=7);parser.add_argument('--model',type=Path);parser.add_argument('--output-name',default='base-review-v2');parser.add_argument('--solid-only',action='store_true');parser.add_argument('--output',type=Path,required=True);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    worker=tree_workspace(args.mask);bank=scenery_workspace('croisement02-north-woodland-bank') if args.mask==7 else OUT/'ground-receiver-review-v5';support_asset=bank.name if args.mask==7 else 'croisement02-ground-receiver';out=args.output.resolve();out.mkdir(parents=True,exist_ok=False)
    hashes={str(w):sha(w/'model.blend') for w in (worker,bank)}
    model=args.model.resolve() if args.model else worker/'model.blend';model_hash=sha(model)
    bpy.ops.wm.open_mainfile(filepath=str(bank/'model.blend'));bpy.context.view_layer.update()
    expected_support=sorted([[list(row) for row in o.matrix_world] for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==support_asset],key=str)
    bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update()
    wood=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==worker.name and o.get('projection_component')!='crown']
    with bpy.data.libraries.load(str(bank/'model.blend'),link=False) as (source,loaded):loaded.collections=['Croisement02 Working']
    collection=loaded.collections[0];bpy.context.scene.collection.children.link(collection);bpy.context.view_layer.update()
    surfaces=[o for o in collection.all_objects if o.type=='MESH' and o.get('asset_group')==support_asset]
    if not wood or not surfaces:raise ValueError('Missing wood or supporting surface')
    actual_support=sorted([[list(row) for row in o.matrix_world] for o in surfaces],key=str)
    if expected_support!=actual_support:raise ValueError('Linked support transforms differ from separately opened source')
    originals=wood+surfaces
    transforms={o:o.matrix_world.copy() for o in originals}
    scene=bpy.data.scenes.new('Native-direction lower wood and ground contact');bpy.context.window.scene=scene
    isolated=[]
    for original in originals:
        obj=original.copy();obj.parent=None;obj.matrix_world=transforms[original];obj.hide_render=False;obj['asset_group']=worker.name;scene.collection.objects.link(obj);isolated.append(obj)
    scene.world=bpy.data.worlds.new('Root contact neutral environment');scene.world.color=(.12,.12,.12)
    light_data=bpy.data.lights.new('Root contact sun','SUN');light_data.energy=2;light=bpy.data.objects.new(light_data.name,light_data);scene.collection.objects.link(light);light.rotation_euler=(.5,-.4,-.5)
    packet=json.loads((worker/'modified/views.json').read_text());packet['scene_name']=scene.name;packet['object_names']=[o.name for o in isolated];packet.pop('render_object_names',None);packet['tile_size']=[384,384]
    lowpoints=np.array([o.matrix_world@v.co for o in wood for v in o.data.vertices if (o.matrix_world@v.co).z<30]);target=Vector((float(np.median(lowpoints[:,0])),float(np.median(lowpoints[:,1])),25.))
    for index,view in enumerate(packet['views']):
        az=math.radians(index*45);e=math.radians(35);position=target+Vector((math.sin(az)*math.cos(e),-math.cos(az)*math.cos(e),math.sin(e)))*5000;rotation=(target-position).to_track_quat('-Z','Y').to_euler();matrix=rotation.to_matrix().to_4x4();matrix.translation=position
        view.update(camera_location=list(position),camera_rotation_euler=list(rotation),camera_matrix_world=[list(r) for r in matrix],ortho_scale=120 if args.mask==7 else 140,crop=dict(width=384,height=384))
    write_json(out/'views.json',packet);scene=bpy.data.scenes[packet['scene_name']];scene.render.engine='CYCLES';scene.cycles.samples=8
    modes=('solid',) if args.solid_only else ('textured','solid')
    render(out/'views.json',out/'views',modes=modes,width=384)
    for mode in modes:
        sheet=Image.new('RGB',(1536,768))
        for i in range(8):sheet.paste(Image.open(out/f'views/view-{i}-{mode}.png'),((i%4)*384,(i//4)*384))
        sheet.save(out/f'{mode}.png')
    for path,digest in hashes.items():
        if sha(Path(path)/'model.blend')!=digest:raise ValueError('Source worker changed')
    write_json(out/'evidence.json',dict(native_view0_elevation=35,support_transforms_verified=True,support_world_matrices=actual_support,workers=hashes,model=str(model),model_sha256=model_hash,objects=packet['object_names'],actual_sha256=sha(out/'textured.png') if (out/'textured.png').exists() else None,solid_sha256=sha(out/'solid.png'),camera_sha256=sha(out/'views.json'),limitation='Close lower wood and selected support only; crown and other foliage hidden. Other relief neighbours excluded; ground plane is a contact diagnostic. No model edits.'))
    if sha(model)!=model_hash:raise ValueError('Reviewed model changed')
    print(out)

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
