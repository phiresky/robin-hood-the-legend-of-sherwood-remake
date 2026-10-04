"""Private source-traced wattle correction, preserving the approved predecessor."""
import json,sys,math,hashlib,copy
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from scipy.ndimage import grey_closing,median_filter
from scipy.signal import find_peaks
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,scenery_workspace
from tree_geometry import SIN,COS
from scenery_geometry import Mesh,bevel
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import _geometry
from source_projection_bake import bake
from render_multiview_asset import render

def main():
 asset='croisement02-southwest-path-wattle-fence';original=scenery_workspace(asset);base=OUT/'mixed-wood-audit/boundary-roles76-93-v1';dst=OUT/'wattle99-source-candidate/v2';dst.mkdir(exist_ok=False,parents=True);original_hash=sha(original/'model.blend')
 native=np.asarray(Image.open(OUT/'baseline/masks/000099.png').convert('L'))>0;top=np.array([np.where(native[:,x])[0].min()if native[:,x].any()else np.nan for x in range(native.shape[1])]);bottom=np.array([np.where(native[:,x])[0].max()if native[:,x].any()else np.nan for x in range(native.shape[1])]);valid=np.where(np.isfinite(top))[0];top=np.interp(np.arange(len(top)),valid,top[valid]);bottom=np.interp(np.arange(len(bottom)),valid,bottom[valid]);body=grey_closing(top,size=11);bottom=median_filter(bottom,size=7);peaks,_=find_peaks(bottom-top,prominence=5,distance=8)
 xs=np.array(sorted(set([int(valid[0]+2),*map(int,peaks),int(valid[-1]-2)])));nodes=[]
 for x in xs:
  source_x=int(x+530);source_base=float(bottom[x]+750);source_top=float(top[x]+750);height=(source_base-source_top)/COS;nodes.append(dict(source_x=source_x,source_base_y=source_base,source_top_y=source_top,height=height,foot=(source_x,-source_base/SIN,0),certainty='Source silhouette hypothesis; hidden ground contact and stake depth inferred'))
 # Keep every source-role change private and explicit. The new148 boundary
 # pixels lie outside native99; they are inferred wood roles, not observed99.
 manifest=copy.deepcopy(json.loads((original/'source-masks.json').read_text()));inventory_path=Path(manifest['mask_inventory']);inventory=json.loads(inventory_path.read_text());inventory=copy.deepcopy(inventory)
 for row in inventory['masks']:row['png']=str((inventory_path.parent/row['png']).resolve())
 domain=np.zeros((1152,1792),bool);domain[750:1021,530:785]=native;edge=np.asarray(Image.open(base/'76-wattle99.png').convert('L'))>0;domain|=edge;domain_path=dst/'private-wattle-domain.png';Image.fromarray(domain.astype('uint8')*255).save(domain_path)
 row76=next(r for r in inventory['masks']if r['index']==76);raw=np.asarray(Image.open(row76['png']).convert('L'))>0;excluded=np.zeros_like(domain);x,y=row76['box_top_left'];excluded[y:y+raw.shape[0],x:x+raw.shape[1]]=raw;excluded&=~edge;exclude_path=dst/'private-foreground76.png';Image.fromarray(excluded.astype('uint8')*255).save(exclude_path)
 assert not any(r['index']in[6000,6001]for r in inventory['masks'])
 for index,path in [(6000,domain_path),(6001,exclude_path)]:inventory['masks'].append(dict(index=index,layer=0,png=str(path),box_top_left=[0,0],box_size=[1792,1152],provenance='Private inferred wattle/foreground boundary correction; no canonical source-domain revision'))
 assignment=next(r for r in manifest['projections']['exterior']['assignments']if r.get('asset_group')==asset);assignment['mask_indices']=[6000];assignment['exclude_mask_indices']=[6001,42,129];assignment['exclusions_reviewed']=True;assignment['exclusion_reason']='Source-only provisional wood boundary proposal restores148 contiguous post/rail edge pixels, retaining all remaining foreground76 exclusion and conservatively excluding overlapping tree42/canopy129 occupancy.'
 write_json(dst/'mask-inventory.json',inventory);manifest['mask_inventory']=str(dst/'mask-inventory.json');write_json(dst/'source-masks.json',manifest)
 bpy.ops.wm.open_mainfile(filepath=str(original/'model.blend'));bpy.context.preferences.filepaths.save_version=0;scene=bpy.context.scene;objects=list(bpy.data.collections['Croisement02 Working'].all_objects);selected=[o for o in objects if o.type=='MESH' and o.get('asset_group')==asset];assert len(selected)==1;obj=selected[0];outside={o.name:_geometry(o,protect_appearance=True)for o in objects if o.type=='MESH' and o!=obj};mesh=Mesh()
 for i,node in enumerate(nodes):
  foot=Vector(node['foot']);height=node['height'];mesh.tube(foot,foot+Vector((0,0,height)),1.85,1.55,10)
 for left,right in zip(nodes,nodes[1:]):
  a,b=left['source_x']-530,right['source_x']-530
  for row in range(12):
   points=[]
   for step in range(7):
    t=step/6;x=a+(b-a)*t;base_y=float(np.interp(x,np.arange(len(bottom)),bottom)+750);top_y=float(np.interp(x,np.arange(len(body)),body)+750);height=(base_y-top_y)/COS;z=2+(height-3)*(row+.5)/12;depth=1.35*math.sin(t*math.pi*2+row*math.pi);points.append(Vector((x+530,-base_y/SIN+depth,z)))
   for aa,bb in zip(points,points[1:]):mesh.tube(aa,bb,1.75,n=8)
 mesh.apply(obj);topology=bevel(obj,.12,1);assert topology['nonmanifold_edges']==0 and topology['degenerate_faces']==0
 bpy.ops.wm.save_as_mainfile(filepath=str(dst/'geometry.blend'));cfg=json.loads((original/'workspace.json').read_text());bake('Croisement02',cfg['source_path'],dst/'source-ownership.json',receiver_nodes=['building-021'],receiver_object_names=[obj.name],occluder_nodes=['building-021'],projection_label='exterior',preserve_authored=False,source_mask_manifest=str(dst/'source-masks.json'))
 assert outside=={o.name:_geometry(o,protect_appearance=True)for o in objects if o.type=='MESH' and o!=obj};bpy.ops.wm.save_as_mainfile(filepath=str(dst/'model.blend'));digest=sha(dst/'model.blend');packet=json.loads((original/'modified/views.json').read_text());packet.pop('render_object_names',None);packet['source_blend']=str(dst/'model.blend');packet['object_names']=[obj.name]
 for view in packet['views']:view['crop']=dict(width=384,height=384)
 write_json(dst/'views.json',packet);scene.render.engine='CYCLES';scene.cycles.samples=16;render(dst/'views.json',dst/'views',modes=('textured','solid'),width=384)
 for mode in ['textured','solid']:
  sheet=Image.new('RGB',(1536,768));
  for i in range(8):sheet.paste(Image.open(dst/f'views/view-{i}-{mode}.png'),((i%4)*384,(i//4)*384))
  sheet.save(dst/f'{mode}.png')
 assert sha(original/'model.blend')==original_hash
 write_json(dst/'evidence.json',dict(status='Private source-driven correction; native and all-eight review pending',model_sha256=digest,original_model_sha256=original_hash,nodes=nodes,topology=topology,outside_appearance_unchanged=True,source_domain_sha256=sha(domain_path),restored_inferred_boundary_pixels=int(edge.sum()),limitations=['Mask contours suggest member positions; foreground-covered stake positions and ground contact remain inferred.','This replaces earlier uniform member spacing in a private worker; approval does not carry.','Unknown back texture remains gray.','Native coverage and complete actual-material review required before catalog selection.']))
 print(dst)

if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
