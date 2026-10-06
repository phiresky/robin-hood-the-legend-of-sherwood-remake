"""Read source endpoint placement support from approved terrain receivers."""
from pathlib import Path
import sys,json,hashlib
import bpy,numpy as np
from mathutils import Vector
from PIL import Image
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN
from refinement_review import _tree
from render_slots import acquire,release
h=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 D=OUT/'restart9-hole-endpoints/receiver-audit-v1';D.mkdir(parents=True,exist_ok=False);authority=OUT/'restart7-source-patch-delivery/physical-scope-v1/report.json';record=next(r for r in json.load(open(authority))['records']if r['profile']=='Croisement01 - hole');members=record['members'];positions=sorted(set(tuple(r['display_position'])for r in members));terminal=OUT/'source-states/mission-patches/mission-Emb05_FoB_MP-patch-009/transition-029.png';a=np.array(Image.open(terminal).convert('RGBA'));yy,xx=np.where(a[:,:,3]>0);offset=members[0]['terminal']['offset'];samples=[(float(x+offset[0])+.5,float(y+offset[1])+.5)for y,x in zip(yy,xx)if x%5==0 and y%5==0];samples.append((51.,57.));models=[('flat',OUT/'restart4-fence14-ground-candidate-v1/model.blend','4f4875bc62b5602417830eb8b458bfbe8dcc9244095699096616fdb4dfb58bf8'),('bank',OUT/'restart4-bank103-source-overlay-v2/model.blend','c0b8458ed39b0412fae16a03384c5585c3271cc5eb48072f0354380ab8f0072e')];hits={};pins=[]
 for key,path,digest in models:
  assert h(path)==digest;bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update();obs=[o for o in bpy.context.scene.objects if o.type=='MESH'and ((key=='flat'and o.name=='Croisement02 Terrain')or(key=='bank'and o.get('asset_group')=='croisement02-north-woodland-bank'))];assert obs,(key,[o.name for o in bpy.context.scene.objects if o.type=='MESH']);tree,owners,_=_tree(obs);pins.append(dict(path=str(path),sha256=digest,objects=[dict(name=o.name,matrix=[list(r)for r in o.matrix_world])for o in obs]))
  for pos in positions:
   rows=[]
   for x,y in samples:
    gx,gy=pos[0]+x,pos[1]+y;p,n,index,d=tree.ray_cast(Vector((gx,-gy/SIN,0))+RAY*6000,-RAY);rows.append(dict(local_xy=[x,y],global_xy=[gx,gy],hit=list(p)if p is not None else None,normal=list(n)if n is not None else None,owner=owners[index].name if p is not None else None,ray_distance=float(d)if p is not None else None))
   hits[(key,pos)]=rows
 rows=[]
 for pos in positions:
  combined=[]
  for i in range(len(samples)):
   candidates=[hits[(key,pos)][i]for key,_,_ in models if hits[(key,pos)][i]['hit']is not None];chosen=min(candidates,key=lambda r:r['ray_distance']);combined.append(chosen)
  z=[r['hit'][2]for r in combined];rows.append(dict(display_position=list(pos),instances=[r['id']for r in members if tuple(r['display_position'])==pos],native_elevation=0,minimum_receiver_z=min(z),maximum_receiver_z=max(z),center=combined[-1],sampled_receiver_names=sorted(set(r['owner']for r in combined)),samples=combined))
 (D/'report.json').write_text(json.dumps(dict(authority=str(authority),authority_sha256=h(authority),members=members,source_sha256=h(terminal),models=pins,positions=rows,scope='Read-only sampled native rays against approved flat ground and bank. Other foreground scenery is not yet included; receiver Z is physical support inference, not a rewrite of native elevation.'),indent=2)+'\n');print([(r['display_position'],r['minimum_receiver_z'],r['maximum_receiver_z'],r['sampled_receiver_names'])for r in rows],flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
