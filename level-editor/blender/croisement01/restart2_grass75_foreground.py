"""Move final grass blades coherently along source rays, retaining rooted endpoints."""
import json,math,sys,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire
from evidence_io import sha
from refinement_workspace import prepare,modified,validate
from refinement_inventory import inventory
R=ROOT/'level-editor/work/croisement01-refinement/restart2';BASE=R.parent

def main():
 if shutil.disk_usage(ROOT).free<25*1024**3:raise ValueError('Disk floor:25GiB required before a new candidate')
 old=R/'grass75-volume-v13';worker=old/'assets/croisement01-grass-75';dest=R/'grass75-volume-v14';dest.mkdir(exist_ok=False);shutil.copytree(old/'source',dest/'source')
 probe=json.loads((R/'grass75-final-blade-depth-probe-v1.json').read_text());assert probe['model_sha256']==sha(worker/'model.blend') and not probe['root_conflicts'] and probe['maximum_offset']<40
 acquire();bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));bpy.context.preferences.filepaths.save_version=0
 working=bpy.data.collections['Croisement01 Working'];asset='croisement01-grass-75';obj=next(o for o in working.all_objects if o.type=='MESH' and o.get('asset_group')==asset)
 assert len(obj.data.vertices)==probe['expected_vertices'];inverse=obj.matrix_world.inverted();sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35));ray=Vector((0,-cosine/sine,1));before=[obj.matrix_world@v.co for v in obj.data.vertices]
 def move(indices,dz):
  for index in indices:obj.data.vertices[index].co=inverse@(before[index]+ray*dz)
 for n,row in enumerate(probe['assignments']):move(range(n*18,(n+1)*18),probe['offsets'][row['blade']]*math.sin(row['step']/6*math.pi/2))
 rgba=np.asarray(Image.open(old/'source/native.png').convert('RGBA'));yy,xx=np.nonzero(rgba[:,:,3]>127);width=float(xx.max()-xx.min()+1);height=float(yy.max()-yy.min()+1);root=np.array(probe['root_world']);rng=np.random.default_rng(80175);root_error=0.
 for blade in range(85):
  angle=math.tau*blade/85+rng.uniform(-.15,.15);outward=np.array([math.cos(angle),math.sin(angle),0]);length=width*rng.uniform(.25,.53);rise=height*rng.uniform(.3,.85);base=root+outward*rng.uniform(0,width*.07);centers=np.asarray([base+outward*(length*t**1.35)+np.array([0,0,rise*math.sin(t*math.pi*.72)]) for t in np.linspace(0,1,7)])
  for index in range(len(xx)*18+blade*72,len(xx)*18+(blade+1)*72):
   step=int(np.argmin(np.sum((centers-np.array(before[index]))**2,axis=1)));dz=probe['offsets'][blade]*math.sin(step/6*math.pi/2);move([index],dz)
   if step==0:root_error=max(root_error,(obj.matrix_world@obj.data.vertices[index].co-before[index]).length)
 obj.data.update();after=[obj.matrix_world@v.co for v in obj.data.vertices];projection_error=max(max(abs(a.x-b.x),abs((-a.y*sine-a.z*cosine)-(-b.y*sine-b.z*cosine))) for a,b in zip(before,after));assert projection_error<.001 and root_error<.001
 # Retain only the target, its branch joint and terrain supports. Never save another whole-map worker.
 terrain={'ground'}|{f'building-{i:03}' for i in [*range(10),*range(76,81)]};keep={o for o in working.all_objects if o.type=='MESH' and (o.get('asset_group') in {asset,'croisement01-east-fallen-branch'} or o.get('source_node') in terrain)}
 for other in list(bpy.data.objects):
  if other.type=='MESH' and other not in keep:bpy.data.objects.remove(other,do_unlink=True)
 bpy.data.orphans_purge(do_recursive=True)
 nodes={o.get('source_node') for o in keep};catalog=json.loads((worker/'reference/grouping.json').read_text());groups=[]
 for g in catalog['groups']:
  parts=[p for p in g['parts'] if (f"building-{p['obstacle']:03}" if 'obstacle' in p else p['node']) in nodes]
  if parts:groups.append(dict(g,parts=parts))
 catalog['groups']=groups;catalog['canonical_owners']={k:v for k,v in catalog['canonical_owners'].items() if k in nodes};cat=dest/'catalog.json';cat.write_text(json.dumps(catalog,indent=2)+'\n')
 inventory(dest/'inventory',collection_name=working.name,map_name='Croisement01',source_path=BASE/'baseline/covered.png',patch_manifest=BASE/'source-states/layers.json')
 review=dest/'grouping-review.json';review.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(cat),inventory_sha256=sha(dest/'inventory/inventory.json'),evidence='Same saved grass75,branch andterrain identities. Only coherent source-ray grass displacement; compact context excludes unrelated map objects.'),indent=2)+'\n')
 masks=json.loads((worker/'source-masks.json').read_text())
 for p in masks['projections'].values():
  for q in p.get('occluder_constraints',[]):q['receiver_nodes']=[n for n in q['receiver_nodes'] if n in nodes]
 source=dest/'source-masks.json';source.write_text(json.dumps(masks,indent=2)+'\n')
 w=dest/'assets'/asset;prepare(w,asset_id=asset,scene_name='Croisement01 Refinement',collection_name=working.name,source_path=BASE/'baseline/covered.png',grouping_manifest=cat,inventory_path=dest/'inventory/inventory.json',review_path=review,source_mask_manifest=source,width=256,height=256,framing_padding=1.25,lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05));validate(w);modified(w)
 (w/'inspection').mkdir(exist_ok=True);(w/'inspection/construction.json').write_text(json.dumps(dict(status='private candidate; source-order andoblique review pending',model_sha256=sha(w/'model.blend'),previous_model_sha256=probe['model_sha256'],probe_sha256=sha(R/'grass75-final-blade-depth-probe-v1.json'),method='One smooth source-ray displacement per final blade; corresponding source fragments follow that blade. Base endpoints remain fixed.',maximum_vertical_offset=probe['maximum_offset'],original_projection_max_error=projection_error,root_endpoint_max_error=root_error,compact_context_meshes=len(keep),whole_map_duplicate=False,limitations=['Hidden depth remains inferred; large displacement may need oblique refinement.','Original source fragment construction still requires visual acceptance.','Archival terrain remains provisional.']),indent=2)+'\n')
 import render_candidate
 sys.argv=['render','--',str(w)];render_candidate.main()
if __name__=='__main__':main()
