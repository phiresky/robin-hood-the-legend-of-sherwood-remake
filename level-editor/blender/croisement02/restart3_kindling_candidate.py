"""New kindling worker with exact native ownership and untouched outside scene."""
import argparse,json,sys,shutil
from pathlib import Path
import bpy
from PIL import Image
import numpy as np
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json,record_recipe
from refinement_workspace import prepare,modified,validate,_geometry
from tree_geometry import replace_mesh
from render_tree import render_workspace
from render_slots import acquire,release

def main():
 parser=argparse.ArgumentParser();parser.add_argument('--version',type=int,choices=[1,2],default=1);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
 asset='croisement02-southwest-kindling-bundle';old=OUT/'scenery-round-1/assets'/asset;worker=OUT/f'restart3-kindling/candidate-v{args.version}/assets'/asset;geometry=OUT/f'restart3-kindling/outline-v{args.version}/geometry.json';baseline_hash='1ad1b4e508e8511d42d5caf135935aaa81f6b5ca34f2a6a9f57329f9df29e26b'
 if worker.exists():raise FileExistsError(worker)
 if sha(old/'model.blend')!=baseline_hash:raise ValueError('Approved geometry changed')
 cfg=json.loads((old/'workspace.json').read_text());bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'));bpy.context.view_layer.update();bpy.context.preferences.filepaths.save_version=0
 objects=list(bpy.data.collections[cfg['collection_name']].all_objects);target,=[o for o in objects if o.type=='MESH' and o.get('asset_group')==asset];name=target.name;outside={o.name:_geometry(o,True) for o in objects if o!=target}
 prepare(worker,asset_id=asset,scene_name=cfg['scene_name'],collection_name=cfg['collection_name'],source_path=old/'reference/source.png',grouping_manifest=old/'reference/grouping.json',inventory_path=old/'reference/inventory.json',review_path=old/'reference/grouping-review.json',source_mask_manifest=old/'source-masks.json',width=384,height=384,framing_padding=2.3,lighting=cfg['lighting'])
 data=json.loads(geometry.read_text());neutral=bpy.data.materials.new('Kindling unknown hidden wood');neutral.use_nodes=True;neutral.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.25,.25,.25,1);neutral.node_tree.nodes.get('Principled BSDF').inputs['Roughness'].default_value=1
 topology=replace_mesh(target,data['vertices'],data['faces'],materials=[neutral]);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'));modified(worker)
 snapshot=worker/'projected-before-restore.blend';shutil.copyfile(worker/'model.blend',snapshot)
 bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'));bpy.context.view_layer.update();target=bpy.data.objects[name]
 with bpy.data.libraries.load(str(snapshot),link=False) as (_,loaded):loaded.objects=[name]
 source,=loaded.objects
 if source is None:raise ValueError('Missing projected receiver')
 target.data=source.data;bpy.data.objects.remove(source,do_unlink=True);bpy.context.view_layer.update()
 if outside!={o.name:_geometry(o,True) for o in bpy.data.collections[cfg['collection_name']].all_objects if o!=target}:raise ValueError('Outside scene changed')
 bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'));validate(worker);inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
 native=next(r for r in json.loads((OUT/'baseline/masks/manifest.json').read_text())['masks'] if r['index']==104);x,y=native['box_top_left'];mw,mh=native['box_size'];packetdir=worker/'native-source';packetdir.mkdir();image=Image.open(old/'reference/source.png').convert('RGBA').crop((x,y,x+mw,y+mh));image.putalpha(Image.open(OUT/'baseline/masks'/native['png']).convert('L'));image.save(packetdir/'complete-source.png');write_json(packetdir/'packet.json',dict(native_bbox=[x,y,mw,mh],source_sha256=sha(old/'reference/source.png'),mask_sha256=sha(OUT/'baseline/masks'/native['png'])))
 proof=dict(model_sha256=sha(worker/'model.blend'),previous_worker=str(old),previous_model_sha256=baseline_hash,geometry_file=str(geometry),geometry_sha256=sha(geometry),source_rgba_sha256=sha(packetdir/'complete-source.png'),source_mask_sha256=sha(worker/'source-masks.json'),topology=topology,outside_geometry_and_appearance_exact=True,protected_outside=outside,limitations=data['limitations']+['Previously generated hidden texture does not transfer to new geometry; fresh fill requires geometry approval.'],mask=104,source_packet=str(packetdir/'packet.json'),status='Private kindling candidate; source and ground/neighbour review pending')
 write_json(inspection/'refinement.json',proof);record_recipe(worker,Path(__file__));render_workspace(worker,384,release_slot=False)
 if sha(old/'model.blend')!=baseline_hash:raise ValueError('Original changed')
 print(worker)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
