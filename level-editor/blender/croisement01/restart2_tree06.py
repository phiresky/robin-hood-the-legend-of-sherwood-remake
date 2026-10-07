"""Native mask06 decorative tree, without inventing a gameplay obstacle association."""
import argparse,json,math,random,shutil,sys
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageDraw,ImageChops
sys.path.insert(0,str(Path(__file__).parent))
from restart2_tree18 import OUT,SIN,COS,union,assign_mesh,fit_native_width
import restart2_tree05 as tube_helpers
tube_helpers.DENSE=True
tube=tube_helpers.tube
from refinement_workspace import prepare,modified,validate
from refinement_inventory import inventory
from evidence_io import sha
from render_slots import acquire

def main():
 if shutil.disk_usage(OUT).free<25*1024**3:raise ValueError('Disk floor25GiB')
 parser=argparse.ArgumentParser();parser.add_argument('--revision',type=int,default=1);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else []);dest=OUT/f'restart2/tree06-v{args.revision}';dest.mkdir(exist_ok=False);acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement01-grouped.blend'));bpy.context.preferences.filepaths.save_version=0;working=bpy.data.collections['Croisement01 Working'];asset='croisement01-tree-06';wood_node='scenery-tree06-wood';crown_node='foliage-tree06-inferred-crown';name='Northwest Slender Forked Tree'
 native=json.loads((OUT/'baseline/masks/manifest.json').read_text())
 for row in native['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
 row=next(r for r in native['masks'] if r['index']==6);assert row['obstacle_indices']==[];alpha=Image.open(row['png']).convert('L');wood=Image.open(OUT/'restart2/tree06-wood-source-v1/wood-domain.png').convert('L')
 wood.save(dest/'wood-domain.png');ImageChops.subtract(alpha,wood).save(dest/'deferred-domain.png');native['masks'].append(dict(row,index=206,png=str(dest/'wood-domain.png')))
 base_y=-356/SIN
 def point(x,y):return Vector((x,base_y,(356-y)/COS))
 trace=[(386,358,22),(380,332,19),(381,290,17),(382,240,15),(382,190,14),(380,145,14),(377,100,13),(380,45,13),(383,-35,12)]
 body=tube('Slender continuous native trunk',[point(x,y) for x,y,r in trace]+[Vector((382,base_y,690)),Vector((375,base_y,785))],[r for x,y,r in trace]+[10,3]);body['defer_union']=True;rng=random.Random(106)
 domain=np.asarray(wood)>0;xx=np.arange(domain.shape[1])+334
 for sy in range(domain.shape[0]):
  if sy<140:domain[sy]&=(xx>=367)&(xx<=397)
  if sy>=360:domain[sy]=False
 fit_native_width(body,domain,'west-cut',x0=334,y0=0)
 for label,branch in [('Left fork',[(381,170,8),(365,115,6),(358,70,5),(346,0,5),(340,-45,4)]),('Right fork',[(385,200,9),(401,130,6),(407,70,4),(416,-40,4)])]:
  coords=[point(x,y) for x,y,r in branch]+[Vector((branch[-1][0],base_y,690)),Vector((branch[-1][0]+(-40 if label.startswith('Left') else 40),base_y,780))];part=tube(label,coords,[r for x,y,r in branch]+[4,1]);domain=np.asarray(wood)>0
  for sy in range(domain.shape[0]):domain[sy]&=((xx<367) if label.startswith('Left') else (xx>397))&(sy<140)
  fit_native_width(part,domain,'west-cut',x0=334,y0=0);union(body,part)
 root=tube('Observed descending slender root',[point(387,345),point(399,377),point(411,404),point(413,423)],[9,5,3,.5]);domain=np.asarray(wood)>0;domain[:372]=False
 fit_native_width(root,domain,'west-cut',x0=334,y0=0);union(body,root)
 for i in range(11):
  angle=math.tau*i/11;start=Vector((380,base_y,640+i%3*20));tip=Vector((380+math.cos(angle)*145,base_y+math.sin(angle)*173,775+rng.uniform(-35,35)));union(body,tube('Inferred supported bough',[start,start.lerp(tip,.55)+Vector((0,0,18)),tip],[7,3,.5]))
 bpy.context.view_layer.objects.active=body;modifier=body.modifiers.new('Continuous native wood','REMESH');modifier.mode='VOXEL';modifier.voxel_size=.9;modifier.use_remove_disconnected=False;bpy.ops.object.modifier_apply(modifier=modifier.name)
 mesh=body.data.copy();bpy.data.objects.remove(body,do_unlink=True);dummy=bpy.data.meshes.new('Decorative wood slot');fallback=bpy.data.materials.new('Unknown decorative wood');fallback.diffuse_color=(.34,.34,.34,1);dummy.materials.append(fallback);obj=bpy.data.objects.new('Tree06 decorative wood',dummy);working.objects.link(obj);obj['source_node']=wood_node;obj['asset_group']=asset;obj['asset_name']=name;obj['part_name']='Scenery wood; no gameplay obstacle';assign_mesh(obj,mesh)
 verts=[];faces=[]
 for i in range(2200):
  theta=rng.uniform(0,math.tau);zeta=rng.uniform(-1,1);rad=rng.random()**.35;center=Vector((380+165*rad*math.sqrt(1-zeta*zeta)*math.cos(theta),base_y+195*rad*math.sqrt(1-zeta*zeta)*math.sin(theta),785+145*rad*zeta));size=rng.uniform(3,6);normal=Vector((rng.uniform(-1,1),rng.uniform(-1,1),rng.uniform(-.7,.9))).normalized();side=normal.cross(Vector((0,0,1))).normalized();up=normal.cross(side).normalized();offset=len(verts);verts.extend([center-side*size,center+up*size*.65,center+side*size,center-up*size*.65]);faces.extend([(offset,offset+1,offset+2),(offset,offset+2,offset+3)])
 mesh=bpy.data.meshes.new('Inferred complete crown');mesh.from_pydata(verts,[],faces);mesh.update();mat=bpy.data.materials.new('Unknown off-map foliage');mat.diffuse_color=(.34,.34,.34,1);mesh.materials.append(mat);crown=bpy.data.objects.new('Tree06 inferred crown',mesh);working.objects.link(crown)
 for k,v in dict(source_node=crown_node,asset_group=asset,asset_name=name,part_name='Inferred crown',projection_component='crown').items():crown[k]=v
 uv=mesh.uv_layers.new(name='Source UV');known=mesh.color_attributes.new(name='Source ownership',type='FLOAT_COLOR',domain='CORNER');mesh.color_attributes.active_color=known
 for loop in mesh.loops:
  p=mesh.vertices[loop.vertex_index].co;uv.data[loop.index].uv=(p.x/1408,1-(-p.y*SIN-p.z*COS)/960);known.data[loop.index].color=(0,1,1,1)
 terrain_nodes={'ground'}|{f'building-{i:03d}' for i in [*range(10),*range(76,81)]};vertices=[];polygons=[];owners=[]
 for support in list(working.all_objects):
  if support.type!='MESH' or support.get('source_node') not in terrain_nodes:continue
  offset=len(vertices);vertices.extend(support.matrix_world@v.co for v in support.data.vertices);polygons.extend(tuple(offset+i for i in face.vertices) for face in support.data.polygons);owners.extend([support.get('source_node')]*len(support.data.polygons))
 tree=BVHTree.FromPolygons(vertices,polygons);ray=Vector((0,-COS,SIN));anchor=point(386,357);origin=anchor+ray*5000;skipped=[]
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
 (dest/'support-placement.json').write_text(json.dumps(dict(hit=list(hit),owner=anchor_owner,shift=list(shift),skipped=skipped,claim='Private placement only; contact review required'),indent=2)+'\n')
 keep={o for o in working.all_objects if o.type=='MESH' and (o in [obj,crown] or o.get('source_node') in terrain_nodes)}
 for other in list(bpy.data.objects):
  if other.type=='MESH' and other not in keep:bpy.data.objects.remove(other,do_unlink=True)
 bpy.data.orphans_purge(do_recursive=True);catalog=json.loads((OUT/'catalog.json').read_text());groups=[];nodes={o.get('source_node') for o in keep}
 for group in catalog['groups']:
  parts=[p for p in group['parts'] if (f"building-{p['obstacle']:03}" if 'obstacle' in p else p['node']) in nodes]
  if parts:groups.append(dict(group,parts=parts))
 groups.append(dict(id=asset,name=name,parts=[dict(node=wood_node,name='Native forked trunk and descending root'),dict(node=crown_node,name='Inferred complete crown')]))
 catalog.update(version=2,groups=groups,canonical_owners={f"building-{p['obstacle']:03}" if 'obstacle' in p else p['node']:g['id'] for g in groups for p in g['parts']});cat=dest/'catalog.json';cat.write_text(json.dumps(catalog,indent=2)+'\n');inventory(dest/'inventory',collection_name=working.name,map_name='Croisement01',source_path=OUT/'baseline/covered.png',patch_manifest=OUT/'source-states/layers.json')
 masks=dest/'masks.json';masks.write_text(json.dumps(native,indent=2)+'\n');source=dest/'source-masks.json';source.write_text(json.dumps(dict(version=1,mask_inventory=str(masks),projections=dict(exterior=dict(state='Native mask06 decorative tree wood',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=asset,mask_indices=[206])]))),indent=2)+'\n');review=dest/'grouping-review.json';review.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(cat),inventory_sha256=sha(dest/'inventory/inventory.json'),evidence='Mask06 owns the long forked trunk; the crossing leaf cluster and basal fern/mixed source remain separately deferred. Gameplay association remains to be measured from baseline visual footprints; no collision record is changed. Crown complete beyond source border; only permitted reference tree construction patterns used.'),indent=2)+'\n')
 worker=dest/'assets'/asset;prepare(worker,asset_id=asset,scene_name='Croisement01 Refinement',collection_name=working.name,source_path=OUT/'baseline/covered.png',grouping_manifest=cat,inventory_path=dest/'inventory/inventory.json',review_path=review,source_mask_manifest=source,width=384,height=384,framing_padding=1.2,lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05));validate(worker);modified(worker);(worker/'inspection').mkdir(exist_ok=True)
 (dest/'construction.json').write_text(json.dumps(dict(status='Private unapproved geometry candidate',model_sha256=sha(worker/'model.blend'),native_mask=6,native_obstacles=[],no_gameplay_mutation=True,wood_pixels=int(np.count_nonzero(np.asarray(wood))),deferred_pixels=int(np.count_nonzero(np.asarray(ImageChops.subtract(alpha,wood)))),crown_width=330,crown_depth=390,trace=trace),indent=2)+'\n');import render_candidate;sys.argv=['render','--',str(worker)];render_candidate.main()
if __name__=='__main__':main()
