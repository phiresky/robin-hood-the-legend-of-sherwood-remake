"""Compact source-traced leaning east-border tree with complete inferred crown."""
import json,math,random,shutil,sys
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
from PIL import Image,ImageDraw,ImageChops
sys.path.insert(0,str(Path(__file__).parent))
from restart2_tree18 import OUT,SIN,COS,union,assign_mesh
from refinement_workspace import prepare,modified,validate
from refinement_inventory import inventory
from evidence_io import sha
from render_slots import acquire

def tube(name,centers,radii):
 """Transport each ring frame continuously through changes of trunk lean."""
 verts=[];faces=[];count=20;previous=None
 for i,point in enumerate(centers):
  tangent=(centers[min(i+1,len(centers)-1)]-centers[max(i-1,0)]).normalized()
  if previous is None:
   helper=min([Vector((1,0,0)),Vector((0,1,0)),Vector((0,0,1))],key=lambda a:abs(a.dot(tangent)))
   side=tangent.cross(helper).normalized()
  else:
   side=previous-tangent*previous.dot(tangent)
   if side.length<1e-7:raise ValueError('Tube frame transport became singular')
   side.normalize()
   if side.dot(previous)<0:side=-side
  up=side.cross(tangent).normalized();previous=side.copy()
  for j in range(count):
   angle=math.tau*j/count;v=point+radii[i]*(math.cos(angle)*side+math.sin(angle)*up);v.z=max(.02,v.z);verts.append(v)
 faces.append(tuple(reversed(range(count))))
 for i in range(len(centers)-1):
  for j in range(count):faces.append((i*count+j,i*count+(j+1)%count,(i+1)*count+(j+1)%count,(i+1)*count+j))
 faces.append(tuple(range((len(centers)-1)*count,len(centers)*count)))
 mesh=bpy.data.meshes.new(name);mesh.from_pydata(verts,[],faces);mesh.update()
 bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free()
 obj=bpy.data.objects.new(name,mesh);bpy.context.scene.collection.objects.link(obj);return obj

def main():
 if shutil.disk_usage(OUT).free<25*1024**3:raise ValueError('Disk floor25GiB')
 dest=OUT/'restart2/tree71-v4';dest.mkdir(exist_ok=False)
 acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement01-grouped.blend'));bpy.context.preferences.filepaths.save_version=0
 working=bpy.data.collections['Croisement01 Working'];asset='croisement01-tree-71';name='Eastern Leaning Oak'
 objects={int(o['source_node'].split('-')[-1]):o for o in working.all_objects if o.type=='MESH' and o.get('source_node') in ['building-062','building-063']};assert len(objects)==2
 for o in objects.values():o['asset_group']=asset;o['asset_name']=name
 native=json.loads((OUT/'baseline/masks/manifest.json').read_text())
 for row in native['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
 row=next(r for r in native['masks'] if r['index']==71);alpha=Image.open(row['png']).convert('L');wood=Image.new('L',alpha.size);draw=ImageDraw.Draw(wood)
 # Foreground greenery crosses the lower trunk; no colored leaf sample becomes bark.
 outline=[(1407,243),(1392,270),(1362,301),(1350,327),(1345,351),(1335,347),(1327,321),(1317,314),(1310,320),(1317,343),(1321,365),(1320,394),(1315,420),(1308,437),(1308,462),(1328,484),(1340,487),(1353,470),(1360,450),(1370,426),(1388,433),(1392,416),(1407,398)]
 draw.polygon([(x-1304,y-243) for x,y in outline],fill=255);wood=ImageChops.multiply(alpha,wood)
 wood.save(dest/'wood-domain.png');ImageChops.subtract(alpha,wood).save(dest/'deferred-foliage-domain.png')
 native['masks'].append(dict(row,index=271,png=str(dest/'wood-domain.png')))
 base_y=-520/SIN
 def point(x,y):return Vector((x,base_y,(520-y)/COS))
 trace=[(1341,520,29),(1336,495,29),(1338,465,36),(1347,430,38),(1359,395,36),(1370,360,32),(1388,325,27),(1408,290,27),(1428,250,26)]
 centers=[point(x,y) for x,y,r in trace]+[Vector((1470,base_y-12,405)),Vector((1520,base_y-20,480)),Vector((1550,base_y-25,530))]
 body=tube('Continuous leaning wood',centers,[r for x,y,r in trace]+[22,15,4]);body['defer_union']=True
 union(body,tube('Source snapped left fork',[point(1350,425),point(1342,388),point(1332,349),point(1321,319)],[17,15,12,5.5]))
 rng=random.Random(717101)
 for i in range(8):
  a=math.tau*i/8;start=Vector((1520,base_y-20,460+i*7));tip=Vector((1550+math.cos(a)*92,base_y-25+math.sin(a)*118,540+rng.uniform(-15,40)))
  union(body,tube('Inferred crown bough'+str(i),[start,start.lerp(tip,.55)+Vector((0,0,12)),tip],[8,4,.8]))
 bpy.context.view_layer.objects.active=body;mod=body.modifiers.new('Continuous wood junctions','REMESH');mod.mode='VOXEL';mod.voxel_size=1.15;mod.use_remove_disconnected=False;bpy.ops.object.modifier_apply(modifier=mod.name)
 mesh=body.data.copy();bpy.data.objects.remove(body,do_unlink=True);assign_mesh(objects[62],mesh)
 # Partition existing surface faces between native gameplay references. Their
 # union stays exact; no internal caps or shifted seams are introduced.
 combined=objects[62].data.copy();world=objects[62].matrix_world.copy()
 for index,lower in [(62,True),(63,False)]:
  bm=bmesh.new();bm.from_mesh(combined)
  for v in bm.verts:v.co=world@v.co
  remove=[f for f in bm.faces if (f.calc_center_median().z>=150)==lower]
  bmesh.ops.delete(bm,geom=remove,context='FACES')
  bmesh.ops.delete(bm,geom=[v for v in bm.verts if not v.link_faces],context='VERTS')
  target=bpy.data.meshes.new('Leaning tree wood'+str(index));bm.to_mesh(target);bm.free()
  for layer in list(target.uv_layers):target.uv_layers.remove(layer)
  for layer in list(target.color_attributes):target.color_attributes.remove(layer)
  target.materials.clear();assign_mesh(objects[index],target)
 verts=[];faces=[]
 for i in range(1300):
  theta=rng.uniform(0,math.tau);zeta=rng.uniform(-1,1);rad=rng.random()**.35
  center=Vector((1550+125*rad*math.sqrt(1-zeta*zeta)*math.cos(theta),base_y-25+156*rad*math.sqrt(1-zeta*zeta)*math.sin(theta),550+78*rad*zeta))
  size=rng.uniform(4,8);normal=Vector((rng.uniform(-1,1),rng.uniform(-1,1),rng.uniform(-.7,.9))).normalized();side=normal.cross(Vector((0,0,1))).normalized();up=normal.cross(side).normalized();offset=len(verts)
  verts.extend([center-side*size,center+up*size*.65,center+side*size,center-up*size*.65]);faces.extend([(offset,offset+1,offset+2),(offset,offset+2,offset+3)])
 mesh=bpy.data.meshes.new('Complete inferred leaning-tree crown');mesh.from_pydata(verts,[],faces);mesh.update();mat=bpy.data.materials.new('Unknown off-map foliage');mat.diffuse_color=(.34,.34,.34,1);mesh.materials.append(mat)
 crown=bpy.data.objects.new('Tree71 inferred off-map crown',mesh);working.objects.link(crown);crown_node='foliage-tree71-inferred-crown'
 for k,v in dict(source_node=crown_node,asset_group=asset,asset_name=name,part_name='Inferred crown',projection_component='crown').items():crown[k]=v
 uv=mesh.uv_layers.new(name='Source UV');known=mesh.color_attributes.new(name='Source ownership',type='FLOAT_COLOR',domain='CORNER');mesh.color_attributes.active_color=known
 for loop in mesh.loops:
  p=mesh.vertices[loop.vertex_index].co;uv.data[loop.index].uv=(p.x/1408,1-(-p.y*SIN-p.z*COS)/960);known.data[loop.index].color=(0,1,1,1)
 # Seat the complete tree along the original camera ray on archived ground.
 from mathutils.bvhtree import BVHTree
 terrain_nodes={'ground'}|{f'building-{i:03}' for i in [*range(10),*range(76,81)]}
 context_vertices=[];context_faces=[]
 bpy.context.view_layer.update()
 for support in working.all_objects:
  if support.type!='MESH' or support.get('source_node') not in terrain_nodes:continue
  offset=len(context_vertices);context_vertices.extend(support.matrix_world@v.co for v in support.data.vertices)
  context_faces.extend(tuple(offset+i for i in f.vertices) for f in support.data.polygons)
 terrain_bvh=BVHTree.FromPolygons(context_vertices,context_faces);ray=Vector((0,-COS,SIN))
 foot=min((objects[62].matrix_world@v.co for v in objects[62].data.vertices),key=lambda p:p.z)
 hit=terrain_bvh.ray_cast(foot+ray*5000,-ray,10000)[0]
 if hit is None:raise ValueError('Missing archived root support')
 shift=ray*((hit-foot).dot(ray)+.2)
 for target in [*objects.values(),crown]:
  inverse=target.matrix_world.inverted()
  for vertex in target.data.vertices:vertex.co=inverse@(target.matrix_world@vertex.co+shift)
  target.data.update()
 (dest/'support-placement.json').write_text(json.dumps(dict(foot=list(foot),hit=list(hit),shift=list(shift),scope='Source-ray placement; final bank joint still requires visual review'),indent=2)+'\n')
 terrain={'ground'}|{f'building-{i:03}' for i in [*range(10),*range(76,81)]}
 keep={o for o in working.all_objects if o.type=='MESH' and (o in objects.values() or o==crown or o.get('source_node') in terrain)}
 for o in list(bpy.data.objects):
  if o.type=='MESH' and o not in keep:bpy.data.objects.remove(o,do_unlink=True)
 bpy.data.orphans_purge(do_recursive=True)
 catalog=json.loads((OUT/'catalog.json').read_text());groups=[];nodes={o.get('source_node') for o in keep}
 for group in catalog['groups']:
  parts=[p for p in group['parts'] if p.get('obstacle') not in (62,63) and (f"building-{p['obstacle']:03}" if 'obstacle' in p else p['node']) in nodes]
  if parts:groups.append(dict(group,parts=parts))
 groups.append(dict(id=asset,name=name,parts=[dict(obstacle=62,name='Root and lower stem'),dict(obstacle=63,name='Upper leaning stem and snapped branch'),dict(node=crown_node,name='Inferred complete crown')]))
 catalog.update(version=2,groups=groups,canonical_owners={f"building-{p['obstacle']:03}" if 'obstacle' in p else p['node']:g['id'] for g in groups for p in g['parts']})
 cat=dest/'catalog.json';cat.write_text(json.dumps(catalog,indent=2)+'\n')
 inventory(dest/'inventory',collection_name=working.name,map_name='Croisement01',source_path=OUT/'baseline/covered.png',patch_manifest=OUT/'source-states/layers.json')
 masks=dest/'masks.json';masks.write_text(json.dumps(native,indent=2)+'\n');source=dest/'source-masks.json';source.write_text(json.dumps(dict(version=1,mask_inventory=str(masks),projections=dict(exterior=dict(state='Initial eastern leaning oak wood only',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=asset,mask_indices=[271])]))),indent=2)+'\n')
 review=dest/'grouping-review.json';review.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(cat),inventory_sha256=sha(dest/'inventory/inventory.json'),evidence='Native mask71 and native parts62/63 show a leaning continuous tree with a snapped left branch. Dense lower foreground foliage excluded; complete crown beyond east edge is inferred.'),indent=2)+'\n')
 worker=dest/'assets'/asset
 prepare(worker,asset_id=asset,scene_name='Croisement01 Refinement',collection_name=working.name,source_path=OUT/'baseline/covered.png',grouping_manifest=cat,inventory_path=dest/'inventory/inventory.json',review_path=review,source_mask_manifest=source,width=384,height=384,framing_padding=1.2,lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05))
 validate(worker);modified(worker);(worker/'inspection').mkdir(exist_ok=True)
 (dest/'construction.json').write_text(json.dumps(dict(status='Private candidate; self-review required',model_sha256=sha(worker/'model.blend'),native_parts=[62,63],wood_pixels=int(np.count_nonzero(np.asarray(wood))),deferred_pixels=int(np.count_nonzero(np.asarray(ImageChops.subtract(alpha,wood)))),trace=trace,crown_width=250,crown_depth=312,whole_map_duplicate=False,limitations=['Native ground and final foot seating need contact review.','All off-map wood and crown depth are inferred; hidden appearance is gray.','Source mask56 foreground canopy remains separate.']),indent=2)+'\n')
 import render_candidate
 sys.argv=['render','--',str(worker)];render_candidate.main()
if __name__=='__main__':main()
