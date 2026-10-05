"""Closed source-fitted hay mound with physical frayed straw, preserving canonical halves."""
import argparse,json,sys,shutil,math
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image
from scipy.ndimage import label,distance_transform_edt
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,scenery_workspace
from evidence_io import sha,write_json,record_recipe
from refinement_workspace import prepare,modified,validate,_geometry,_render
from source_projection_bake import bake
from tree_geometry import replace_mesh,SIN,COS,RAY
from scenery_geometry import Mesh
from render_tree import render_workspace
from render_slots import acquire,release

def main():
 parser=argparse.ArgumentParser();parser.add_argument('--geometry-worker',type=Path,required=True);parser.add_argument('--geometry-sha256',required=True);parser.add_argument('--label',required=True);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);raw=args.geometry_worker
 if sha(raw/'model.blend')!=args.geometry_sha256:raise ValueError('Raw geometry changed')
 asset='croisement02-south-field-haystack';old=scenery_workspace(asset);worker=OUT/'restart3-hay'/args.label/'assets'/asset;geom=raw/'topology.json';baseline_hash='39c50413bce3e9249866b3c34bd74a5ad2122c5656ddf5f9ac4489b72fea0b46'
 if worker.exists():raise FileExistsError(worker)
 if sha(old/'model.blend')!=baseline_hash:raise ValueError('Hay baseline changed')
 cfg=json.loads((old/'workspace.json').read_text());bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'));bpy.context.view_layer.update();bpy.context.preferences.filepaths.save_version=0;objects=list(bpy.data.collections[cfg['collection_name']].all_objects);targets={o['source_node']:o for o in objects if o.type=='MESH' and o.get('asset_group')==asset};outside={o.name:_geometry(o,True) for o in objects if o not in targets.values()}
 if set(targets)!={'building-140','building-141'}:raise ValueError('Canonical halves differ')
 template=OUT/'restart3-hay/candidate-v3/assets'/asset;worker.mkdir(parents=True)
 for name in ['baseline.blend','workspace.json','source-masks.json']:shutil.copyfile(template/name,worker/name)
 for name in ['input','reference','mask-reference']:shutil.copytree(template/name,worker/name)
 bpy.ops.wm.open_mainfile(filepath=str(raw/'model.blend'));bpy.context.view_layer.update();targets={o['source_node']:o for o in bpy.data.collections[cfg['collection_name']].all_objects if o.type=='MESH' and o.get('asset_group')==asset};topology={};strands=[]
 if outside!={o.name:_geometry(o,True) for o in bpy.data.collections[cfg['collection_name']].all_objects if o not in targets.values()}:raise ValueError('Raw outside changed')
 for node,obj in targets.items():
  bm=bmesh.new();bm.from_mesh(obj.data);topology[node]=dict(vertices=len(bm.verts),faces=len(bm.faces),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces));bm.free()
  if topology[node]['nonmanifold_edges'] or topology[node]['degenerate_faces']:raise ValueError(('Invalid raw geometry',node,topology[node]))
  if min((obj.matrix_world@v.co).z for v in obj.data.vertices)<-.001:raise ValueError('Raw geometry below ground')
 bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'));modified(worker);current=json.loads((worker/'workspace.json').read_text());bake('Croisement02',current['source_path'],worker/'source-self-ownership.json',receiver_nodes=sorted(targets),occluder_nodes=sorted(targets),projection_label='exterior',elevation_deg=35,preserve_authored=False,source_mask_manifest=current['source_mask_manifest']);(worker/'modified').rename(worker/'before-self-projection');_render(current,worker/'modified',worker/'input/views.json');bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'));snapshot=worker/'projected-before-restore.blend';shutil.copyfile(worker/'model.blend',snapshot)
 names={node:o.name for node,o in targets.items()};bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'));bpy.context.view_layer.update()
 with bpy.data.libraries.load(str(snapshot),link=False) as (_,loaded):loaded.objects=list(names.values())
 for name,source in zip(names.values(),loaded.objects):
  if source is None:raise ValueError('Missing projected hay half')
  bpy.data.objects[name].data=source.data
 for o in loaded.objects:bpy.data.objects.remove(o,do_unlink=True)
 bpy.context.view_layer.update()
 if outside!={o.name:_geometry(o,True) for o in bpy.data.collections[cfg['collection_name']].all_objects if o.name not in names.values()}:raise ValueError('Outside geometry/material changed')
 bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'));validate(worker);inspection=worker/'inspection';inspection.mkdir(exist_ok=True);native=next(r for r in json.loads((OUT/'baseline/masks/manifest.json').read_text())['masks'] if r['index']==124);x,y=native['box_top_left'];mw,mh=native['box_size'];packetdir=worker/'native-source';packetdir.mkdir();im=Image.open(old/'reference/source.png').convert('RGBA').crop((x,y,x+mw,y+mh));im.putalpha(Image.open(OUT/'baseline/masks'/native['png']).convert('L'));im.save(packetdir/'complete-source.png');write_json(packetdir/'packet.json',dict(native_bbox=[x,y,mw,mh],source_sha256=sha(old/'reference/source.png'),mask_sha256=sha(OUT/'baseline/masks'/native['png'])))
 write_json(inspection/'refinement.json',dict(model_sha256=sha(worker/'model.blend'),previous_worker=str(old),previous_model_sha256=baseline_hash,geometry_file=str(geom),geometry_sha256=sha(geom),raw_geometry_model=str(raw/'model.blend'),raw_geometry_sha256=args.geometry_sha256,source_mask_sha256=sha(worker/'source-masks.json'),topology=topology,frayed_straw=strands,outside_geometry_and_appearance_exact=True,protected_outside=outside,mask=124,source_packet=str(packetdir/'packet.json'),limitations=['One mound, two canonical closed halves; shared internal seam retained.','Hidden depth, straw thickness and attachment are inferred. Excess skirt trimmed by a closed native-ray volume; diagonal corner contacts separated by0.001 source pixels. Ground support corrected before contour intersection.','Original mask124 retained including disconnected straw. No ground reassignment.','Unknown rear remains gray; previous texture approval does not transfer.'],status='Private hay candidate; native/ground review pending'));record_recipe(worker,Path(__file__));render_workspace(worker,384,release_slot=False)
 if sha(old/'model.blend')!=baseline_hash:raise ValueError('Baseline changed')
 print(worker)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
