"""Classify northern source edge samples on current approved bank and ground."""
import sys,json,math
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
from mathutils.bvhtree import BVHTree
HERE=Path(__file__).resolve().parent;sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY
from review_bank_candidate import camera
from restart3_initial_fence_contact import link
from restore_ground75_source import geometry
D=OUT/'restart4-bank24-role-audit-v1';BANK=OUT/'restart2-bank321/packaged-v1/assets/croisement02-north-woodland-bank/model.blend';GROUND=OUT/'restart4-final-floor-bake-v1/model.blend'
def main():
 assert not D.exists();assert sha(BANK)=='69ecb7b704e30d6d64565a44aa810a21b924195609dbe7ac35818a0209137641';assert sha(GROUND)=='206c7c562b9aa9fc204ce42c0b0e9b1b78a22bee79ac00e6dab7fbbc59c64630';D.mkdir()
 coords=json.loads((OUT/'restart6-source-coverage/outside-mask-context-v1/report.json').read_text())['records'][0]['coordinates'];assert len(coords)==24
 bpy.ops.wm.open_mainfile(filepath=str(BANK));bpy.context.view_layer.update();bank=[o for o in bpy.data.objects if o.type=='MESH'and o.get('asset_group')=='croisement02-north-woodland-bank'];verts=[];tris=[];names=[]
 for o in bank:
  start=len(verts);verts.extend(o.matrix_world@v.co for v in o.data.vertices);o.data.calc_loop_triangles()
  for t in o.data.loop_triangles:tris.append(tuple(start+i for i in t.vertices));names.append(o.name)
 tree=BVHTree.FromPolygons(verts,tris,all_triangles=True);projected=np.array([(p.x,-p.y*SIN-p.z*COS)for p in verts]);edges=np.array([(t[i],t[(i+1)%3])for t in tris for i in range(3)]);a=projected[edges[:,0]];b=projected[edges[:,1]];delta=b-a;den=np.maximum((delta*delta).sum(1),1e-20)
 def hit(x,y):
  origin=Vector((x,-y/SIN,0))+RAY*10000;p,normal,index,dist=tree.ray_cast(origin,-RAY)
  return dict(bank_in_front=p is not None and dist<=origin.z/RAY.z+.001,owner=names[index]if p is not None else None,point=list(p)if p is not None else None)
 known=np.array(Image.open(OUT/'restart4-final-floor-bake-v1/known-native-domain.png').convert('L'))>0;domain=np.array(Image.open(OUT/'terrain-bank-candidate/bank-source-domain.png').convert('L'))>0;rows=[]
 for x,y in coords:
  center=hit(x+.5,y+.5);samples=[hit(x+dx,y+dy)for dx in np.arange(.1,1,.2)for dy in np.arange(.1,1,.2)];t=np.clip(((np.array([x+.5,y+.5])-a)*delta).sum(1)/den,0,1);dist=np.linalg.norm(a+t[:,None]*delta-[x+.5,y+.5],axis=1);j=int(dist.argmin());edge=edges[j];world=[list(verts[k])for k in edge];take=[v for v in samples if v['bank_in_front']]
  rows.append(dict(pixel=[x,y],center=center,subpixel_hits=len(take),subpixel_count=25,nearest_projected_edge_distance=float(dist[j]),nearest_edge_world=world,nearest_edge_z=[v[2]for v in world],subpixel_hit_z_range=[min(v['point'][2]for v in take),max(v['point'][2]for v in take)]if take else None,bank_source420=bool(domain[y,x]),ground_already_known_native=bool(known[y,x]),classification='subpixel physical bank-ground contour'if take and not center['bank_in_front']else 'review'))
 source=Image.open(OUT/'source-states/covered.png').convert('RGB');marked=np.array(source)
 for x,y in coords:marked[y,x]=[20,230,235]if known[y,x]else[250,150,30]
 box=(945,285,1055,405);sheet=Image.new('RGB',(880,512),'#292929');draw=ImageDraw.Draw(sheet)
 for i,(im,label)in enumerate([(source,'Native source'),(Image.fromarray(marked),'Cyan12 exact known / orange12 inferred underlay')]):sheet.paste(im.crop(box).resize((440,480),Image.Resampling.NEAREST),(i*440,32));draw.text((i*440+6,8),label,fill='white')
 sheet.save(D/'source24.png')
 scene=bpy.data.scenes.new('Current bank24 contact');bpy.context.window.scene=scene
 for o in bank:link(scene,o);o.hide_render=False
 with bpy.data.libraries.load(str(GROUND),link=False)as(src,dst):dst.objects=['Croisement02 Terrain']
 ground=dst.objects[0];link(scene,ground);ground.hide_render=False;bpy.context.view_layer.update();assert geometry(ground)=='d985f5556fd548f88d9d2d0aa9bf9c6e98bb9ef419cbd159afbf73496b7dd934'
 for label,direction in [('native',RAY),('oblique',Vector((.6,-.6,.5)).normalized())]:
  camera(scene,Vector((1000,-345/SIN,0)),direction,768,768,155);scene.render.filepath=str(D/(label+'.png'));bpy.ops.render.render(write_still=True,scene=scene.name)
 report=dict(status='Read-only physical role review; no receiver reassignment',bank_model_sha256=sha(BANK),ground_model_sha256=sha(GROUND),bank_source420_sha256=sha(OUT/'terrain-bank-candidate/bank-source-domain.png'),rows=rows,counts=dict(total=24,center_bank=sum(r['center']['bank_in_front']for r in rows),subpixel_bank=sum(r['subpixel_hits']>0 for r in rows),already_known_ground=sum(r['ground_already_known_native']for r in rows)),proposal='Retain current physical bank-ground contour and approved12 native ground samples if all24 are subpixel contour cases; do not extrude bank to force binary mask centers. Remaining12 underlay/source-state roles require separate interpretation, not automatic wood or bank volume.',geometry_changed=False,texture_changed=False,source_authority_changed=False,files={p.name:sha(p)for p in D.glob('*.png')})
 write_json(D/'report.json',report);print(json.dumps(report['counts']),flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
