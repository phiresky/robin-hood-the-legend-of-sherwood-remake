"""Native mask04 decorative tree, without inventing a gameplay obstacle association."""
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
 parser=argparse.ArgumentParser();parser.add_argument('--revision',type=int,default=1);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else []);dest=OUT/f'restart2/tree04-v{args.revision}';dest.mkdir(exist_ok=False);acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement01-grouped.blend'));bpy.context.preferences.filepaths.save_version=0;working=bpy.data.collections['Croisement01 Working'];asset='croisement01-tree-04';wood_node='scenery-tree04-wood';crown_node='foliage-tree04-inferred-crown';name='Northwest Forked Foreground Tree'
 native=json.loads((OUT/'baseline/masks/manifest.json').read_text())
 for row in native['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
 row=next(r for r in native['masks'] if r['index']==4);assert row['obstacle_indices']==[];alpha=Image.open(row['png']).convert('L');wood=alpha.copy()
 wood.save(dest/'wood-domain.png');ImageChops.subtract(alpha,wood).save(dest/'deferred-domain.png');native['masks'].append(dict(row,index=204,png=str(dest/'wood-domain.png')))
 base_y=-590/SIN
 def point(x,y):return Vector((x,base_y,(590-y)/COS))
 trace=[(215,589,28),(213,570,28),(213,535,24),(212,480,25),(210,410,26),(213,350,28),(223,300,24),(226,240,23),(232,160,22),(234,80,22),(232,0,23),(230,-60,22)]
 body=tube('Foreground continuous main trunk',[point(x,y) for x,y,r in trace]+[Vector((230,base_y,880)),Vector((230,base_y,990))],[r for x,y,r in trace]+[20,9]);body['defer_union']=True;rng=random.Random(104)
 if args.revision>=2:
  domain=np.asarray(wood)>0;xx=np.arange(domain.shape[1])+81
  for sy in range(domain.shape[0]):
   if sy<240:domain[sy]&=(xx>=200)&(xx<=244)
   elif sy<350:domain[sy]&=xx>=190
  fit_native_width(body,domain,'west-cut',x0=81,y0=0)

 for label,branch in [('Left high fork',[(216,342,16),(209,270,12),(201,190,10),(190,110,8),(180,40,8),(182,-50,9)]),('Right high fork',[(224,275,16),(246,215,11),(257,145,10),(272,75,10),(282,20,10),(283,-55,11)]),('Broken left branch',[(216,358,21),(181,342,17),(146,325,13),(129,313,12),(125,293,12),(118,281,12),(101,280,9),(83,277,6)])]:
  coords=[point(x,y) for x,y,r in branch];radii=[r for x,y,r in branch]
  if args.revision>=2 and label!='Broken left branch':
   coords.extend([Vector((branch[-1][0],base_y,870)),Vector((branch[-1][0]+(-20 if label.startswith('Left') else 20),base_y,1000))]);radii.extend([7,2])
  branch_obj=tube(label,coords,radii)
  if args.revision>=3:
   domain=np.asarray(wood)>0;xx=np.arange(domain.shape[1])+81
   for sy in range(domain.shape[0]):
    if label=='Left high fork':domain[sy]&=(xx<210)&(sy<235)
    elif label=='Right high fork':domain[sy]&=(xx>240)&(sy<225)
    else:domain[sy]&=(xx<190)&(sy>=265)&(sy<348)
   fit_native_width(branch_obj,domain,'west-cut',x0=81,y0=0)
  union(body,branch_obj)
 for angle in [.5,2.4,4.4]:
  base=point(215,588);tip=base+Vector((math.cos(angle)*39,math.sin(angle)*45,0));tip.z=.1
  union(body,tube('Natural low buttress',[point(214,565),base.lerp(tip,.6)+Vector((0,0,3)),tip],[9,5,.6]))
 for i in range(12):
  angle=math.tau*i/12;start=Vector((230,base_y,850+i%3*15));tip=Vector((230+math.cos(angle)*175,base_y+math.sin(angle)*195,1020+rng.uniform(-40,45)));union(body,tube('Inferred supported bough',[start,start.lerp(tip,.55)+Vector((0,0,20)),tip],[10,5,1]))
 bpy.context.view_layer.objects.active=body;modifier=body.modifiers.new('Continuous forked foreground wood','REMESH');modifier.mode='VOXEL';modifier.voxel_size=1.2;modifier.use_remove_disconnected=False;bpy.ops.object.modifier_apply(modifier=modifier.name)
 if args.revision>=2:
  for vertex in body.data.vertices:
   sy=-vertex.co.y*SIN-vertex.co.z*COS
   if sy>589.5:vertex.co+=Vector((0,SIN,COS))*(sy-589.5)
  body.data.update()
 mesh=body.data.copy();bpy.data.objects.remove(body,do_unlink=True);dummy=bpy.data.meshes.new('Decorative wood slot');fallback=bpy.data.materials.new('Unknown decorative wood');fallback.diffuse_color=(.34,.34,.34,1);dummy.materials.append(fallback);obj=bpy.data.objects.new('Tree04 decorative wood',dummy);working.objects.link(obj);obj['source_node']=wood_node;obj['asset_group']=asset;obj['asset_name']=name;obj['part_name']='Scenery wood; no gameplay obstacle';assign_mesh(obj,mesh)
 verts=[];faces=[]
 for i in range(2400):
  theta=rng.uniform(0,math.tau);zeta=rng.uniform(-1,1);rad=rng.random()**.35;center=Vector((230+200*rad*math.sqrt(1-zeta*zeta)*math.cos(theta),base_y+230*rad*math.sqrt(1-zeta*zeta)*math.sin(theta),1020+180*rad*zeta));size=rng.uniform(3,6);normal=Vector((rng.uniform(-1,1),rng.uniform(-1,1),rng.uniform(-.7,.9))).normalized();side=normal.cross(Vector((0,0,1))).normalized();up=normal.cross(side).normalized();offset=len(verts);verts.extend([center-side*size,center+up*size*.65,center+side*size,center-up*size*.65]);faces.extend([(offset,offset+1,offset+2),(offset,offset+2,offset+3)])
 mesh=bpy.data.meshes.new('Inferred complete crown');mesh.from_pydata(verts,[],faces);mesh.update();mat=bpy.data.materials.new('Unknown off-map foliage');mat.diffuse_color=(.34,.34,.34,1);mesh.materials.append(mat);crown=bpy.data.objects.new('Tree04 inferred crown',mesh);working.objects.link(crown)
 for k,v in dict(source_node=crown_node,asset_group=asset,asset_name=name,part_name='Inferred crown',projection_component='crown').items():crown[k]=v
 uv=mesh.uv_layers.new(name='Source UV');known=mesh.color_attributes.new(name='Source ownership',type='FLOAT_COLOR',domain='CORNER');mesh.color_attributes.active_color=known
 for loop in mesh.loops:
  p=mesh.vertices[loop.vertex_index].co;uv.data[loop.index].uv=(p.x/1408,1-(-p.y*SIN-p.z*COS)/960);known.data[loop.index].color=(0,1,1,1)
 terrain_nodes={'ground'}|{f'building-{i:03d}' for i in [*range(10),*range(76,81)]};vertices=[];polygons=[];owners=[]
 for support in list(working.all_objects):
  if support.type!='MESH' or support.get('source_node') not in terrain_nodes:continue
  offset=len(vertices);vertices.extend(support.matrix_world@v.co for v in support.data.vertices);polygons.extend(tuple(offset+i for i in face.vertices) for face in support.data.polygons);owners.extend([support.get('source_node')]*len(support.data.polygons))
 tree=BVHTree.FromPolygons(vertices,polygons);ray=Vector((0,-COS,SIN));anchor=point(215,589);origin=anchor+ray*5000;skipped=[]
 while True:
  hit,normal,index,distance=tree.ray_cast(origin,-ray,10000)
  if hit is None:raise ValueError('No upward terrain support')
  if normal.z>.35:break
  skipped.append(dict(point=list(hit),node=owners[index],normal=list(normal)));origin=hit-ray*.01
 anchor_owner=owners[index]
 shift=ray*((hit-anchor).dot(ray)+.2)
 for target in [obj,crown]:
  for vertex in target.data.vertices:vertex.co+=shift
  target.data.update()
 if args.revision>=3:
  changes=[]
  original_volume=BVHTree.FromPolygons([obj.matrix_world@v.co for v in obj.data.vertices],[tuple(face.vertices) for face in obj.data.polygons])
  for vertex in obj.data.vertices:
   p=obj.matrix_world@vertex.co;sy=-p.y*SIN-p.z*COS
   if sy<540:continue
   origin=p+ray*80;remaining=160;nearest=None
   while remaining>0:
    support,normal,index,distance=tree.ray_cast(origin,-ray,remaining)
    if support is None:break
    if normal.z>.25 and abs((support-p).dot(ray))<80:
     nearest=support;break
    remaining-=distance+.02;origin=support-ray*.02
   if nearest is None:continue
   weight=min(1,max(0,(sy-540)/35));thickness=0.0
   if args.revision>=4:
    rear,_,_,_=original_volume.ray_cast(p-ray*300,ray,600)
    if rear is not None:thickness=max(0.0,(p-rear).dot(ray))
   target=nearest+ray*(.08+thickness);vertex.co=obj.matrix_world.inverted()@(p.lerp(target,weight));changes.append(dict(vertex=vertex.index,distance=float((target-p).dot(ray)),weight=weight))
  obj.data.update();(dest/'basal-ray-support.json').write_text(json.dumps(dict(method=('Basal bank support preserves each original camera-ray wood interval thickness; source projection unchanged.' if args.revision>=4 else 'Smooth basal continuation onto nearby upward archival bank faces along native camera rays; projected source unchanged.'),vertices=changes,terrain_provisional=True),indent=2)+'\n')
 (dest/'support-placement.json').write_text(json.dumps(dict(hit=list(hit),owner=anchor_owner,shift=list(shift),skipped=skipped,claim='Private placement only; contact review required'),indent=2)+'\n')
 keep={o for o in working.all_objects if o.type=='MESH' and (o in [obj,crown] or o.get('source_node') in terrain_nodes)}
 for other in list(bpy.data.objects):
  if other.type=='MESH' and other not in keep:bpy.data.objects.remove(other,do_unlink=True)
 bpy.data.orphans_purge(do_recursive=True);catalog=json.loads((OUT/'catalog.json').read_text());groups=[];nodes={o.get('source_node') for o in keep}
 for group in catalog['groups']:
  parts=[p for p in group['parts'] if (f"building-{p['obstacle']:03}" if 'obstacle' in p else p['node']) in nodes]
  if parts:groups.append(dict(group,parts=parts))
 groups.append(dict(id=asset,name=name,parts=[dict(node=wood_node,name='Forked foreground trunk and broken branch'),dict(node=crown_node,name='Inferred complete crown')]))
 catalog.update(version=2,groups=groups,canonical_owners={f"building-{p['obstacle']:03}" if 'obstacle' in p else p['node']:g['id'] for g in groups for p in g['parts']});cat=dest/'catalog.json';cat.write_text(json.dumps(catalog,indent=2)+'\n');inventory(dest/'inventory',collection_name=working.name,map_name='Croisement01',source_path=OUT/'baseline/covered.png',patch_manifest=OUT/'source-states/layers.json')
 masks=dest/'masks.json';masks.write_text(json.dumps(native,indent=2)+'\n');source=dest/'source-masks.json';source.write_text(json.dumps(dict(version=1,mask_inventory=str(masks),projections=dict(exterior=dict(state='Native mask04 decorative tree wood',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=asset,mask_indices=[204])]))),indent=2)+'\n');review=dest/'grouping-review.json';review.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(cat),inventory_sha256=sha(dest/'inventory/inventory.json'),evidence='Mask04 owns the large foreground forked trunk and broken left branch. Gameplay association remains to be measured from baseline visual footprints; no collision record is changed. Crown complete beyond source border; only permitted reference tree construction patterns used.'),indent=2)+'\n')
 worker=dest/'assets'/asset;prepare(worker,asset_id=asset,scene_name='Croisement01 Refinement',collection_name=working.name,source_path=OUT/'baseline/covered.png',grouping_manifest=cat,inventory_path=dest/'inventory/inventory.json',review_path=review,source_mask_manifest=source,width=384,height=384,framing_padding=1.2,lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05));validate(worker);modified(worker);(worker/'inspection').mkdir(exist_ok=True)
 (dest/'construction.json').write_text(json.dumps(dict(status='Private unapproved geometry candidate',model_sha256=sha(worker/'model.blend'),native_mask=4,native_obstacles=[],no_gameplay_mutation=True,wood_pixels=int(np.count_nonzero(np.asarray(wood))),deferred_pixels=int(np.count_nonzero(np.asarray(ImageChops.subtract(alpha,wood)))),crown_width=400,crown_depth=460,trace=trace),indent=2)+'\n');import render_candidate;sys.argv=['render','--',str(worker)];render_candidate.main()
if __name__=='__main__':main()
