"""Closed source-fitted hay mound with physical frayed straw, preserving canonical halves."""
import json,sys,shutil,math
from pathlib import Path
import bpy,numpy as np
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
 asset='croisement02-south-field-haystack';old=scenery_workspace(asset);worker=OUT/'restart3-hay/candidate-v3/assets'/asset;geom=OUT/'restart3-hay/outline-v3/geometry.json';baseline_hash='39c50413bce3e9249866b3c34bd74a5ad2122c5656ddf5f9ac4489b72fea0b46'
 if worker.exists():raise FileExistsError(worker)
 if sha(old/'model.blend')!=baseline_hash:raise ValueError('Hay baseline changed')
 cfg=json.loads((old/'workspace.json').read_text());bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'));bpy.context.view_layer.update();bpy.context.preferences.filepaths.save_version=0;objects=list(bpy.data.collections[cfg['collection_name']].all_objects);targets={o['source_node']:o for o in objects if o.type=='MESH' and o.get('asset_group')==asset};outside={o.name:_geometry(o,True) for o in objects if o not in targets.values()}
 if set(targets)!={'building-140','building-141'}:raise ValueError('Canonical halves differ')
 prepare(worker,asset_id=asset,scene_name=cfg['scene_name'],collection_name=cfg['collection_name'],source_path=old/'reference/source.png',grouping_manifest=old/'reference/grouping.json',inventory_path=old/'reference/inventory.json',review_path=old/'reference/grouping-review.json',source_mask_manifest=old/'source-masks.json',width=384,height=384,framing_padding=1.65,lighting=cfg['lighting'])
 plan=json.loads(geom.read_text());neutral=bpy.data.materials.new('Hay unknown rear');neutral.use_nodes=True;neutral.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.25,.25,.25,1);neutral.node_tree.nodes.get('Principled BSDF').inputs['Roughness'].default_value=1;trees=[];meshes={}
 for part in plan['parts']:
  m=Mesh();m.vertices=[tuple(v) for v in part['vertices']];m.faces=[tuple(f) for f in part['faces']];meshes[part['source_node']]=m;trees.append(BVHTree.FromPolygons([Vector(v) for v in m.vertices],m.faces))
 residual=np.array(Image.open(geom.parent/'remaining-straw.png').convert('L'))>0;core=np.array(Image.open(OUT/'restart2-vegetation/hay-outline-research/core-domain.png').convert('L'))>0;_,nearest=distance_transform_edt(~core,return_indices=True);components,count=label(residual);strands=[]
 def hit(x,y):
  origin=Vector((float(x),-float(y)/SIN,0))+RAY*5000;hits=[t.ray_cast(origin,-RAY) for t in trees];hits=[h for h in hits if h[0] is not None];return min(hits,key=lambda h:h[3])[0] if hits else None
 def point(x,y,z):return Vector((float(x),-(float(y)+z*COS)/SIN,z))
 for component in range(1,count+1):
  yy,xx=np.nonzero(components==component);pixels=np.column_stack((xx+860.5,yy+930.5));center=pixels.mean(axis=0);axis=np.array([1.,0.])
  if len(pixels)>1:
   values,vectors=np.linalg.eigh(np.cov((pixels-center).T));axis=vectors[:,np.argmax(values)]
  across=np.array([-axis[1],axis[0]]);transverse=(pixels-center)@across;bins=np.floor((transverse-transverse.min())/1.3).astype(int)
  # Every visible fleck is represented by a short closed straw segment. The
  # hidden connector follows the nearest native mound contour and is inferred.
  for binid in np.unique(bins):
   selected=pixels[bins==binid];c=selected.mean(axis=0);along=(selected-c)@axis;a=c+axis*(along.min()-.4);b=c+axis*(along.max()+.4);local=np.clip(np.rint(c-[860.5,930.5]).astype(int),[0,0],[189,179]);iy,ix=nearest[:,local[1],local[0]];native_near=np.array([ix+860.5,iy+930.5]);anchor=None
   for fraction in np.linspace(1.,.7,61):
    p=np.array(plan['radial_origin'])+(native_near-np.array(plan['radial_origin']))*fraction;anchor=hit(*p)
    if anchor is not None:break
   if anchor is None:raise ValueError('Frayed straw has no supporting mound')
   z=max(.45,float(anchor.z)+.25);pa,pb=point(*a,z),point(*b,z);part='building-140' if c[0]<plan['parameters'][4] else 'building-141';m=meshes[part];m.tube(pa,pb,.62,.46,n=6);near=pa if (pa-anchor).length<(pb-anchor).length else pb
   if (near-anchor).length>1.:m.tube(anchor,near,.48,.30,n=6)
   strands.append(dict(source_component=component,native_pixels=len(selected),source_endpoints=[a.tolist(),b.tolist()],world_anchor=list(anchor),inferred_height=z,source_node=part))
 topology={}
 for node,m in meshes.items():
  topology[node]=replace_mesh(targets[node],m.vertices,m.faces,materials=[neutral])
  if topology[node]['nonmanifold_edges'] or topology[node]['degenerate_faces']:raise ValueError('Hay half not closed')
  for f in targets[node].data.polygons:f.use_smooth=f.index<512
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
 write_json(inspection/'refinement.json',dict(model_sha256=sha(worker/'model.blend'),previous_worker=str(old),previous_model_sha256=baseline_hash,geometry_file=str(geom),geometry_sha256=sha(geom),source_mask_sha256=sha(worker/'source-masks.json'),topology=topology,frayed_straw=strands,outside_geometry_and_appearance_exact=True,protected_outside=outside,mask=124,source_packet=str(packetdir/'packet.json'),limitations=['One mound, two canonical closed halves; shared internal seam retained.','Hidden depth, straw thickness and attachment are inferred from original native silhouette.','Original mask124 retained including disconnected straw. No ground reassignment.','Unknown rear remains gray; previous texture approval does not transfer.'],status='Private hay candidate; native/ground review pending'));record_recipe(worker,Path(__file__));render_workspace(worker,384,release_slot=False)
 if sha(old/'model.blend')!=baseline_hash:raise ValueError('Baseline changed')
 print(worker)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
