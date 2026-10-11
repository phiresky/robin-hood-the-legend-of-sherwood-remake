"""Read-only whole Tree38 context with a pinned installed crown.

The installed crown is context, not anatomical guidance or an approval claim.
Its cyan provenance marker is neutralized only in the temporary display copy.
"""
import argparse,json,sys,math
from pathlib import Path
import bpy
from PIL import Image
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace,scenery_workspace
from evidence_io import sha,write_json
from render_slots import acquire,release
from render_multiview_asset import render

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--mask',type=int,choices=(7,31,32,38),default=7);parser.add_argument('--model',type=Path);parser.add_argument('--output-name',default='base-review-v2');parser.add_argument('--solid-only',action='store_true');parser.add_argument('--target-z',type=float);parser.add_argument('--scale',type=float);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    worker=tree_workspace(args.mask);bank=scenery_workspace('croisement02-north-woodland-bank') if args.mask==7 else OUT/'ground-receiver-review-v5';support_asset=bank.name if args.mask==7 else 'croisement02-ground-receiver';out=OUT/f'tree{args.mask:02d}-root-research'/args.output_name;out.mkdir(parents=True,exist_ok=False)
    hashes={str(w):sha(w/'model.blend') for w in (worker,bank)}
    model=args.model.resolve() if args.model else worker/'model.blend';model_hash=sha(model)
    bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update()
    wood=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==worker.name and o.get('projection_component')!='crown']
    with bpy.data.libraries.load(str(bank/'model.blend'),link=False) as (source,loaded):loaded.collections=['Croisement02 Working']
    collection=loaded.collections[0];bpy.context.scene.collection.children.link(collection);bpy.context.view_layer.update()
    surfaces=[o for o in collection.all_objects if o.type=='MESH' and o.get('asset_group')==support_asset]
    if not wood or not surfaces:raise ValueError('Missing wood or supporting surface')
    if args.mask != 38:raise ValueError('This context review is scoped to Tree38')
    glb=ROOT/'level-editor/library/3d-assets/croisement02/croisement02-tree-38/model.glb'
    descriptor=glb.with_name('asset.json');glb_hash=sha(glb);descriptor_hash=sha(descriptor)
    old=set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=str(glb))
    imported=set(bpy.data.objects)-old
    crowns=[o for o in imported if o.type=='MESH' and o.get('projection_component')=='crown']
    if len(crowns)!=1:raise ValueError('Expected one installed crown mesh')
    bpy.context.view_layer.update()
    origin=Vector(json.loads(descriptor.read_text())['source_origin_scene'])
    for crown in crowns:
        world=crown.matrix_world.copy();crown.parent=None;crown.matrix_world=world; crown.location+=origin
        # Vertex colors encode source/inferred provenance, not cyan foliage.
        for attribute in crown.data.color_attributes:
            for value in attribute.data:value.color=(1,1,1,1)
    bpy.context.view_layer.update()
    points=[o.matrix_world@v.co for o in crowns for v in o.data.vertices]
    bounds=[[min(p[i] for p in points) for i in range(3)],[max(p[i] for p in points) for i in range(3)]]
    if not(1450<bounds[0][0]<1470 and 440<bounds[1][2]<460 and -1400<bounds[0][1]<-1370):raise ValueError(f'Unexpected imported crown transform: {bounds}')
    write_json(out/'crown-context.json',dict(glb=str(glb),glb_sha256=glb_hash,descriptor_sha256=descriptor_hash,bounds=bounds,scope='Current installed crown context only; source provenance colors displayed white; no canonical writes or inherited approval'))
    originals=wood+surfaces+crowns
    transforms={o:o.matrix_world.copy() for o in originals}
    scene=bpy.data.scenes.new('Tree07 lower wood and bank contact');bpy.context.window.scene=scene
    isolated=[]
    for original in originals:
        obj=original.copy();obj.parent=None;obj.matrix_world=transforms[original];obj.hide_render=False;obj['asset_group']=worker.name;scene.collection.objects.link(obj);isolated.append(obj)
    scene.world=bpy.data.worlds.new('Root contact neutral environment');scene.world.color=(.12,.12,.12)
    light_data=bpy.data.lights.new('Root contact sun','SUN');light_data.energy=2;light=bpy.data.objects.new(light_data.name,light_data);scene.collection.objects.link(light);light.rotation_euler=(.5,-.4,-.5)
    packet=json.loads((worker/'modified/views.json').read_text());packet['scene_name']=scene.name;packet['object_names']=[o.name for o in isolated];packet.pop('render_object_names',None);packet['tile_size']=[384,384]
    target=Vector({7:(762,-966,65),31:(950,-1137,25),32:(1090,-1298,25),38:(1583,-1225,25)}[args.mask])
    if args.target_z is not None:target.z=args.target_z
    if args.scale is not None and args.scale<=0:raise ValueError('Review scale must be positive')
    for index,view in enumerate(packet['views']):
        # The first card must use the source artwork camera; remaining views
        # use a lower orbit to expose root and terrain contacts.
        az=math.radians(index*45);e=math.radians(35 if index==0 else 25);position=target+Vector((math.sin(az)*math.cos(e),-math.cos(az)*math.cos(e),math.sin(e)))*5000;rotation=(target-position).to_track_quat('-Z','Y').to_euler();matrix=rotation.to_matrix().to_4x4();matrix.translation=position
        view.update(camera_location=list(position),camera_rotation_euler=list(rotation),camera_matrix_world=[list(r) for r in matrix],ortho_scale=args.scale if args.scale is not None else (120 if args.mask==7 else 140),crop=dict(width=384,height=384))
    write_json(out/'views.json',packet);scene=bpy.data.scenes[packet['scene_name']];scene.render.engine='CYCLES';scene.cycles.samples=8
    modes=('solid',) if args.solid_only else ('textured','solid')
    render(out/'views.json',out/'views',modes=modes,width=384)
    for mode in modes:
        sheet=Image.new('RGB',(1536,768))
        for i in range(8):sheet.paste(Image.open(out/f'views/view-{i}-{mode}.png'),((i%4)*384,(i//4)*384))
        sheet.save(out/f'{mode}.png')
    for path,digest in hashes.items():
        if sha(Path(path)/'model.blend')!=digest:raise ValueError('Source worker changed')
    write_json(out/'evidence.json',dict(workers=hashes,model=str(model),model_sha256=model_hash,objects=packet['object_names'],actual_sha256=sha(out/'textured.png') if (out/'textured.png').exists() else None,solid_sha256=sha(out/'solid.png'),camera_sha256=sha(out/'views.json'),limitation='Close lower wood and selected support only; crown and other foliage hidden. Other relief neighbours excluded; ground plane is a contact diagnostic. No model edits.'))
    if sha(model)!=model_hash:raise ValueError('Reviewed model changed')
    if sha(glb)!=glb_hash or sha(descriptor)!=descriptor_hash:raise ValueError('Installed crown changed during review')
    print(out)

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
