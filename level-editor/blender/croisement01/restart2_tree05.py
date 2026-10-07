"""Native mask05 decorative tree, without inventing a gameplay obstacle association."""
import argparse,json,math,random,shutil,sys
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageDraw,ImageChops
sys.path.insert(0,str(Path(__file__).parent))
from restart2_tree18 import OUT,SIN,COS,union,assign_mesh,fit_native_width
from restart2_tree71 import tube
from refinement_workspace import prepare,modified,validate
from refinement_inventory import inventory
from evidence_io import sha
from render_slots import acquire

def main():
 if shutil.disk_usage(OUT).free<35*1024**3:raise ValueError('Disk floor35GiB')
 parser=argparse.ArgumentParser();parser.add_argument('--revision',type=int,default=1);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else []);dest=OUT/f'restart2/tree05-v{args.revision}';dest.mkdir(exist_ok=False);acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement01-grouped.blend'));bpy.context.preferences.filepaths.save_version=0;working=bpy.data.collections['Croisement01 Working'];asset='croisement01-tree-05';wood_node='scenery-tree05-wood';crown_node='foliage-tree05-inferred-crown';name='Northwest Rear Forked Tree'
 native=json.loads((OUT/'baseline/masks/manifest.json').read_text())
 for row in native['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
 row=next(r for r in native['masks'] if r['index']==5);assert row['obstacle_indices']==[];alpha=Image.open(row['png']).convert('L');wood=alpha.copy()
 foreground=next(r for r in native['masks'] if r['index']==4);cut=Image.new('L',alpha.size);cut.paste(Image.open(foreground['png']).convert('L'),(foreground['box_top_left'][0]-row['box_top_left'][0],foreground['box_top_left'][1]-row['box_top_left'][1]));wood=ImageChops.subtract(wood,cut)
 if args.revision>=2:
  for index in [6,8]:
   foreground=next(r for r in native['masks'] if r['index']==index);cut=Image.new('L',alpha.size);cut.paste(Image.open(foreground['png']).convert('L'),(foreground['box_top_left'][0]-row['box_top_left'][0],foreground['box_top_left'][1]-row['box_top_left'][1]));wood=ImageChops.subtract(wood,cut)
 wood.save(dest/'wood-domain.png');ImageChops.subtract(alpha,wood).save(dest/'deferred-domain.png');native['masks'].append(dict(row,index=205,png=str(dest/'wood-domain.png')))
 base_y=-225/SIN
 def point(x,y):return Vector((x,base_y,(225-y)/COS))
 trace=[(320,222,17),(326,211,18),(329,190,16),(331,165,11),(331,139,9),(330,117,8)]
 body=tube('Rear forked trunk',[point(x,y) for x,y,r in trace],[r for x,y,r in trace]);body['defer_union']=True;rng=random.Random(105)
 if args.revision>=2:
  domain=np.asarray(alpha)>0;domain[:140]=False;domain[208:]=False;xx=np.arange(domain.shape[1])+265
  if args.revision<3:domain[175:]&=(xx>=306)&(xx<=350)
  fit_native_width(body,domain,'west-cut',x0=265,y0=0)
  for vertex in body.data.vertices:
   sy=-vertex.co.y*SIN-vertex.co.z*COS
   if sy>208:vertex.co+=Vector((0,SIN,COS))*(sy-208)
  body.data.update()

 for n,branch in enumerate([[(330,135,8),(317,110,6),(305,70,5),(295,30,5),(293,-25,5)],[(330,137,7),(328,100,5),(322,60,5),(317,15,5),(318,-25,5)],[(331,137,7),(342,94,5),(351,48,5),(360,10,5),(366,-25,4)]]):
  coords=[point(x,y) for x,y,r in branch]+[Vector((branch[-1][0],base_y,365)),Vector((330+(n-1)*40,base_y,430))];part=tube('Connected observed fork '+str(n),coords,[r for x,y,r in branch]+[4,1])
  if args.revision>=2:
   domain=np.asarray(alpha)>0;domain[135:]=False;xx=np.arange(domain.shape[1])+265
   for sy in range(135):
    centers=[np.interp(sy,[-25,15,30,70,110,135],[293,294,295,305,317,330]),np.interp(sy,[-25,15,60,100,137],[318,317,322,328,330]),np.interp(sy,[-25,10,48,94,137],[366,360,351,342,331])];domain[sy]&=np.argmin(np.abs(xx[:,None]-np.array(centers)),axis=1)==n
   fit_native_width(part,domain,'west-cut',x0=265,y0=0)
  union(body,part)
 if args.revision>=2:
  roots=[(282,201),(296,220),(344,219),(360,204)]
  for n,(x,y) in enumerate(roots):
   root=tube('Source-supported tapered root',[point(329,190),point((329+x)/2,(190+y)/2),point(x,y)],[8,4,.5])
   if args.revision>=3:
    domain=np.asarray(alpha)>0;domain[:190]=False
    for sy in range(190,domain.shape[0]):
     centers=[np.interp(sy,[190,yy],[329,xx]) for xx,yy in roots];domain[sy]&=np.argmin(np.abs((np.arange(domain.shape[1])+265)[:,None]-np.asarray(centers)),axis=1)==n
    if np.count_nonzero(np.any(domain,axis=1))>=20:fit_native_width(root,domain,'west-cut',x0=265,y0=0)
   union(body,root)
 else:
  for angle in [.2,1.9,3.5,5]:
   base=point(325,220);tip=base+Vector((math.cos(angle)*29,math.sin(angle)*34,0));tip.z=.1;union(body,tube('Tapered root flare',[point(328,203),base.lerp(tip,.6)+Vector((0,0,2)),tip],[5,3,.35]))
 for i in range(9):
  angle=math.tau*i/9;start=Vector((330,base_y,340+i%3*12));tip=Vector((330+math.cos(angle)*80,base_y+math.sin(angle)*92,430+rng.uniform(-15,20)));union(body,tube('Inferred supported bough',[start,start.lerp(tip,.55)+Vector((0,0,10)),tip],[4,2,.3]))
 bpy.context.view_layer.objects.active=body;modifier=body.modifiers.new('Continuous rear forked wood','REMESH');modifier.mode='VOXEL';modifier.voxel_size=.8;modifier.use_remove_disconnected=False;bpy.ops.object.modifier_apply(modifier=modifier.name)
 if args.revision>=3:
  domain=np.asarray(alpha)>0;domain[:145]=False;domain[208:]=False
  before=[v.co.copy() for v in body.data.vertices];fit_native_width(body,domain,'west-cut',x0=265,y0=0)
  for vertex,old in zip(body.data.vertices,before):
   sy=-old.y*SIN-old.z*COS;weight=min(1,max(0,(sy-145)/10))*min(1,max(0,(215-sy)/7));vertex.co=old.lerp(vertex.co,weight)
  body.data.update()
 for vertex in body.data.vertices:
  sy=-vertex.co.y*SIN-vertex.co.z*COS
  if sy>227:vertex.co+=Vector((0,SIN,COS))*(sy-227)
 body.data.update()
 mesh=body.data.copy();bpy.data.objects.remove(body,do_unlink=True);dummy=bpy.data.meshes.new('Decorative wood slot');fallback=bpy.data.materials.new('Unknown decorative wood');fallback.diffuse_color=(.34,.34,.34,1);dummy.materials.append(fallback);obj=bpy.data.objects.new('Tree05 decorative wood',dummy);working.objects.link(obj);obj['source_node']=wood_node;obj['asset_group']=asset;obj['asset_name']=name;obj['part_name']='Scenery wood; no gameplay obstacle';assign_mesh(obj,mesh)
 verts=[];faces=[]
 for i in range(1500):
  theta=rng.uniform(0,math.tau);zeta=rng.uniform(-1,1);rad=rng.random()**.35;center=Vector((330+95*rad*math.sqrt(1-zeta*zeta)*math.cos(theta),base_y+112*rad*math.sqrt(1-zeta*zeta)*math.sin(theta),440+90*rad*zeta));size=rng.uniform(3,6);normal=Vector((rng.uniform(-1,1),rng.uniform(-1,1),rng.uniform(-.7,.9))).normalized();side=normal.cross(Vector((0,0,1))).normalized();up=normal.cross(side).normalized();offset=len(verts);verts.extend([center-side*size,center+up*size*.65,center+side*size,center-up*size*.65]);faces.extend([(offset,offset+1,offset+2),(offset,offset+2,offset+3)])
 mesh=bpy.data.meshes.new('Inferred complete crown');mesh.from_pydata(verts,[],faces);mesh.update();mat=bpy.data.materials.new('Unknown off-map foliage');mat.diffuse_color=(.34,.34,.34,1);mesh.materials.append(mat);crown=bpy.data.objects.new('Tree05 inferred crown',mesh);working.objects.link(crown)
 for k,v in dict(source_node=crown_node,asset_group=asset,asset_name=name,part_name='Inferred crown',projection_component='crown').items():crown[k]=v
 uv=mesh.uv_layers.new(name='Source UV');known=mesh.color_attributes.new(name='Source ownership',type='FLOAT_COLOR',domain='CORNER');mesh.color_attributes.active_color=known
 for loop in mesh.loops:
  p=mesh.vertices[loop.vertex_index].co;uv.data[loop.index].uv=(p.x/1408,1-(-p.y*SIN-p.z*COS)/960);known.data[loop.index].color=(0,1,1,1)
 terrain_nodes={'ground'}|{f'building-{i:03d}' for i in [*range(10),*range(76,81)]};vertices=[];polygons=[];owners=[]
 for support in list(working.all_objects):
  if support.type!='MESH' or support.get('source_node') not in terrain_nodes:continue
  offset=len(vertices);vertices.extend(support.matrix_world@v.co for v in support.data.vertices);polygons.extend(tuple(offset+i for i in face.vertices) for face in support.data.polygons);owners.extend([support.get('source_node')]*len(support.data.polygons))
 tree=BVHTree.FromPolygons(vertices,polygons);ray=Vector((0,-COS,SIN));anchor=point(325,222);origin=anchor+ray*5000;skipped=[]
 while True:
  hit,normal,index,distance=tree.ray_cast(origin,-ray,10000)
  if hit is None:raise ValueError('No upward terrain support')
  if normal.z>.35:break
  skipped.append(dict(point=list(hit),node=owners[index],normal=list(normal)));origin=hit-ray*.01
 shift=ray*((hit-anchor).dot(ray)+.2)
 for target in [obj,crown]:
  for vertex in target.data.vertices:vertex.co+=shift
  target.data.update()
 if args.revision>=3:
  original_volume=BVHTree.FromPolygons([obj.matrix_world@v.co for v in obj.data.vertices],[tuple(face.vertices) for face in obj.data.polygons]);changes=[]
  for vertex in obj.data.vertices:
   p=obj.matrix_world@vertex.co;sy=-p.y*SIN-p.z*COS
   if sy<165:continue
   origin=p+ray*80;remaining=160;nearest=None
   while remaining>0:
    support,normal,index,distance=tree.ray_cast(origin,-ray,remaining)
    if support is None:break
    if normal.z>.25 and abs((support-p).dot(ray))<80:nearest=support;break
    remaining-=distance+.02;origin=support-ray*.02
   if nearest is None:continue
   rear,_,_,_=original_volume.ray_cast(p-ray*300,ray,600)
   if rear is None:continue
   thickness=max(0.0,(p-rear).dot(ray));weight=min(1,max(0,(sy-165)/43));target=nearest+ray*(.08+thickness);vertex.co=obj.matrix_world.inverted()@p.lerp(target,weight);changes.append(dict(vertex=vertex.index,thickness=thickness,weight=weight))
  obj.data.update();(dest/'basal-volume-support.json').write_text(json.dumps(dict(method='Preserve each original native camera ray wood interval while seating its rear on nearby upward archival terrain; source projection unchanged.',vertices=changes,terrain_provisional=True),indent=2)+'\n')
 (dest/'support-placement.json').write_text(json.dumps(dict(hit=list(hit),owner=owners[index],shift=list(shift),skipped=skipped,claim='Private placement only; contact review required'),indent=2)+'\n')
 keep={o for o in working.all_objects if o.type=='MESH' and (o in [obj,crown] or o.get('source_node') in terrain_nodes)}
 for other in list(bpy.data.objects):
  if other.type=='MESH' and other not in keep:bpy.data.objects.remove(other,do_unlink=True)
 bpy.data.orphans_purge(do_recursive=True);catalog=json.loads((OUT/'catalog.json').read_text());groups=[];nodes={o.get('source_node') for o in keep}
 for group in catalog['groups']:
  parts=[p for p in group['parts'] if (f"building-{p['obstacle']:03}" if 'obstacle' in p else p['node']) in nodes]
  if parts:groups.append(dict(group,parts=parts))
 groups.append(dict(id=asset,name=name,parts=[dict(node=wood_node,name='Decorative woody stem'),dict(node=crown_node,name='Inferred complete crown')]))
 catalog.update(version=2,groups=groups,canonical_owners={f"building-{p['obstacle']:03}" if 'obstacle' in p else p['node']:g['id'] for g in groups for p in g['parts']});cat=dest/'catalog.json';cat.write_text(json.dumps(catalog,indent=2)+'\n');inventory(dest/'inventory',collection_name=working.name,map_name='Croisement01',source_path=OUT/'baseline/covered.png',patch_manifest=OUT/'source-states/layers.json')
 masks=dest/'masks.json';masks.write_text(json.dumps(native,indent=2)+'\n');source=dest/'source-masks.json';source.write_text(json.dumps(dict(version=1,mask_inventory=str(masks),projections=dict(exterior=dict(state='Native mask05 decorative tree wood',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=asset,mask_indices=[205])]))),indent=2)+'\n');review=dest/'grouping-review.json';review.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(cat),inventory_sha256=sha(dest/'inventory/inventory.json'),evidence='Mask05 is a rear three-fork scenery tree; foreground Tree04 overlap excluded. Baseline visual ownership must be checked before integration; no gameplay records changed. Crown complete beyond source border; only permitted reference tree construction patterns used.'),indent=2)+'\n')
 worker=dest/'assets'/asset;prepare(worker,asset_id=asset,scene_name='Croisement01 Refinement',collection_name=working.name,source_path=OUT/'baseline/covered.png',grouping_manifest=cat,inventory_path=dest/'inventory/inventory.json',review_path=review,source_mask_manifest=source,width=384,height=384,framing_padding=1.2,lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05));validate(worker);modified(worker);(worker/'inspection').mkdir(exist_ok=True)
 (dest/'construction.json').write_text(json.dumps(dict(status='Private unapproved geometry candidate',model_sha256=sha(worker/'model.blend'),native_mask=5,native_obstacles=[],no_gameplay_mutation=True,wood_pixels=int(np.count_nonzero(np.asarray(wood))),deferred_pixels=int(np.count_nonzero(np.asarray(ImageChops.subtract(alpha,wood)))),crown_width=190,crown_depth=224,trace=trace),indent=2)+'\n');import render_candidate;sys.argv=['render','--',str(worker)];render_candidate.main()
if __name__=='__main__':main()
