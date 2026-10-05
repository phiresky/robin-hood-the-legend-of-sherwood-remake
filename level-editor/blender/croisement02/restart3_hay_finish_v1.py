"""Resume the completed hay geometry after a rejected duplicate review destination."""
import json,sys,shutil
from pathlib import Path
import bpy,bmesh
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json,record_recipe
from refinement_workspace import validate,_geometry,_render
from source_projection_bake import bake
from render_tree import render_workspace
from render_slots import acquire,release

def main():
 asset='croisement02-south-field-haystack';old=OUT/'scenery-round-1/assets'/asset;w=OUT/'restart3-hay/candidate-v1/assets'/asset;baseline='39c50413bce3e9249866b3c34bd74a5ad2122c5656ddf5f9ac4489b72fea0b46';geom=OUT/'restart3-hay/outline-v1/geometry.json'
 if sha(old/'model.blend')!=baseline or (w/'inspection/refinement.json').exists():raise ValueError('Unexpected finished or changed baseline')
 cfg=json.loads((w/'workspace.json').read_text());bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'));bpy.context.view_layer.update();outside={o.name:_geometry(o,True) for o in bpy.data.collections[cfg['collection_name']].all_objects if o.get('asset_group')!=asset};names={o['source_node']:o.name for o in bpy.data.collections[cfg['collection_name']].all_objects if o.type=='MESH' and o.get('asset_group')==asset}
 bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));bpy.context.view_layer.update();bpy.context.preferences.filepaths.save_version=0;before={name:_geometry(bpy.data.objects[name],False) for name in names.values()};topology={}
 for node,name in names.items():
  bm=bmesh.new();bm.from_mesh(bpy.data.objects[name].data);topology[node]=dict(vertices=len(bm.verts),faces=len(bm.faces),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces));bm.free()
  if topology[node]['nonmanifold_edges'] or topology[node]['degenerate_faces']:raise ValueError('Hay half not closed')
 bake('Croisement02',cfg['source_path'],w/'source-self-ownership-resume.json',receiver_nodes=sorted(names),occluder_nodes=sorted(names),projection_label='exterior',elevation_deg=35,preserve_authored=False,source_mask_manifest=cfg['source_mask_manifest']);(w/'modified').rename(w/'before-self-projection');_render(cfg,w/'modified',w/'input/views.json')
 if before!={name:_geometry(bpy.data.objects[name],False) for name in names.values()}:raise ValueError('Source projection changed geometry/UV')
 bpy.ops.wm.save_as_mainfile(filepath=str(w/'model.blend'));snapshot=w/'projected-before-restore.blend'
 if snapshot.exists():raise FileExistsError(snapshot)
 shutil.copyfile(w/'model.blend',snapshot);bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'));bpy.context.view_layer.update()
 with bpy.data.libraries.load(str(snapshot),link=False) as (_,loaded):loaded.objects=list(names.values())
 for name,source in zip(names.values(),loaded.objects):
  if source is None:raise ValueError('Missing projected half')
  bpy.data.objects[name].data=source.data
 for o in loaded.objects:bpy.data.objects.remove(o,do_unlink=True)
 bpy.context.view_layer.update()
 if outside!={o.name:_geometry(o,True) for o in bpy.data.collections[cfg['collection_name']].all_objects if o.get('asset_group')!=asset}:raise ValueError('Outside geometry/material changed')
 bpy.ops.wm.save_as_mainfile(filepath=str(w/'model.blend'));validate(w);inspection=w/'inspection';inspection.mkdir(exist_ok=True);native=next(r for r in json.loads((OUT/'baseline/masks/manifest.json').read_text())['masks'] if r['index']==124);x,y=native['box_top_left'];mw,mh=native['box_size'];packetdir=w/'native-source';packetdir.mkdir();im=Image.open(old/'reference/source.png').convert('RGBA').crop((x,y,x+mw,y+mh));im.putalpha(Image.open(OUT/'baseline/masks'/native['png']).convert('L'));im.save(packetdir/'complete-source.png');write_json(packetdir/'packet.json',dict(native_bbox=[x,y,mw,mh],source_sha256=sha(old/'reference/source.png'),mask_sha256=sha(OUT/'baseline/masks'/native['png'])))
 write_json(inspection/'refinement.json',dict(model_sha256=sha(w/'model.blend'),previous_worker=str(old),previous_model_sha256=baseline,geometry_file=str(geom),geometry_sha256=sha(geom),source_mask_sha256=sha(w/'source-masks.json'),topology=topology,physical_straw_recipe=str(Path(__file__).with_name('restart3_hay_candidate.py')),physical_straw_recipe_sha256=sha(Path(__file__).with_name('restart3_hay_candidate.py')),outside_geometry_and_appearance_exact=True,protected_outside=outside,mask=124,source_packet=str(packetdir/'packet.json'),limitations=['One mound, two canonical closed halves; shared internal seam retained.','Hidden depth, straw thickness and attachment inferred from native silhouette.','Original mask124 retained including disconnected straw; no ground reassignment.','Unknown rear remains gray; prior texture approval does not transfer.'],status='Private hay candidate; source/ground review pending'));record_recipe(w,Path(__file__));record_recipe(w,Path(__file__).with_name('restart3_hay_candidate.py'));render_workspace(w,384,release_slot=False)
 if sha(old/'model.blend')!=baseline:raise ValueError('Baseline changed')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
