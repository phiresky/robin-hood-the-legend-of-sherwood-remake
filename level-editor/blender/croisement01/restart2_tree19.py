"""Compact northern forked tree and its source-owned crossing branch."""
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

from restart2_tree71 import tube

def main():
 if shutil.disk_usage(OUT).free<25*1024**3:raise ValueError('Disk floor25GiB')
 dest=OUT/'restart2/tree19-v2';dest.mkdir(exist_ok=False)
 acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement01-grouped.blend'));bpy.context.preferences.filepaths.save_version=0
 working=bpy.data.collections['Croisement01 Working'];asset='croisement01-tree-19';name='Northern Forked Oak'
 objects={int(o['source_node'].split('-')[-1]):o for o in working.all_objects if o.type=='MESH' and o.get('source_node') in ['building-050','building-051']};assert len(objects)==2
 for o in objects.values():o['asset_group']=asset;o['asset_name']=name
 native=json.loads((OUT/'baseline/masks/manifest.json').read_text())
 for row in native['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
 row=next(r for r in native['masks'] if r['index']==19);alpha=Image.open(row['png']).convert('L');wood=Image.new('L',alpha.size);draw=ImageDraw.Draw(wood)
 # Conservatively trace observed wood through separate native leaf silhouettes.
 main_outline=[(1175,0),(1195,0),(1196,60),(1191,105),(1189,145),(1184,190),(1177,226),(1170,249),(1161,271),(1137,275),(1144,246),(1150,218),(1152,183),(1157,145),(1162,108),(1167,75),(1170,40)]
 draw.polygon([(x-1123,y) for x,y in main_outline],fill=255)
 draw.line([(1136-1123,0),(1141-1123,48),(1147-1123,90),(1155-1123,131),(1168-1123,175)],fill=255,width=18)
 draw.line([(1178-1123,97),(1195-1123,83),(1215-1123,60),(1226-1123,40),(1237-1123,10),(1245-1123,0)],fill=255,width=21)
 # The crossing branch overlap was already excluded from published tree20.
 other=next(r for r in native['masks'] if r['index']==20);overlap_canvas=Image.new('L',alpha.size);overlap_canvas.paste(Image.open(other['png']).convert('L'),(other['box_top_left'][0]-1123,other['box_top_left'][1]));wood=ImageChops.lighter(wood,ImageChops.multiply(alpha,overlap_canvas))
 wood=ImageChops.multiply(alpha,wood)
 wood.save(dest/'wood-domain.png');ImageChops.subtract(alpha,wood).save(dest/'deferred-foliage-domain.png')
 native['masks'].append(dict(row,index=219,png=str(dest/'wood-domain.png')))
 base_y=-265/SIN
 def point(x,y):return Vector((x,base_y,(265-y)/COS))
 trace=[(1158,265,18),(1160,246,20),(1164,221,19),(1168,190,17),(1172,153,16),(1175,116,14),(1179,77,13),(1182,40,12),(1184,-10,11)]
 centers=[point(x,y) for x,y,r in trace]+[Vector((1190,base_y-5,400)),Vector((1185,base_y-8,475)),Vector((1190,base_y-10,540))]
 body=tube('Continuous forked forest stem',centers,[r for x,y,r in trace]+[9,6,1]);body['defer_union']=True
 union(body,tube('Left source fork',[point(1168,175),point(1155,131),point(1147,90),point(1141,48),point(1136,-10),Vector((1130,base_y,400)),Vector((1140,base_y-5,480)),Vector((1145,base_y-8,535))],[9,8,7.5,7,6.5,5,3,1]))
 union(body,tube('Diagonal foreground branch crossing tree20',[point(1178,101),point(1195,87),point(1215,64),point(1226,44),point(1237,14),point(1245,-6),Vector((1250,base_y+8,410)),Vector((1230,base_y+6,480)),Vector((1230,base_y+5,535))],[12,10,9,8,7,6,5,3,1]))
 # The long ground twig belongs to native mask70's fallen branch, not this tree.
 rng=random.Random(191901)
 for i in range(8):
  a=math.tau*i/8;start=Vector((1190,base_y-10,470+i*6));tip=Vector((1195+math.cos(a)*80,base_y-10+math.sin(a)*100,550+rng.uniform(-15,40)))
  union(body,tube('Inferred crown bough'+str(i),[start,start.lerp(tip,.55)+Vector((0,0,12)),tip],[5,3,.7]))
 bpy.context.view_layer.objects.active=body;mod=body.modifiers.new('Continuous wood junctions','REMESH');mod.mode='VOXEL';mod.voxel_size=1.15;mod.use_remove_disconnected=False;bpy.ops.object.modifier_apply(modifier=mod.name)
 bm=bmesh.new();bm.from_mesh(body.data);remaining=set(bm.verts);components=[]
 while remaining:
  pending=[remaining.pop()];component=[]
  while pending:
   vertex=pending.pop();component.append(vertex)
   for edge in vertex.link_edges:
    other=edge.other_vert(vertex)
    if other in remaining:remaining.remove(other);pending.append(other)
  components.append(component)
 removed=[];owned=np.asarray(wood)>0
 for component in sorted(components,key=len,reverse=True)[1:]:
  if len(component)>32:raise ValueError('Disconnected substantial wood component')
  points=[v.co.copy() for v in component]
  for point_on_island in points:
   x=int(math.floor(point_on_island.x-1123));y=int(math.floor(-point_on_island.y*SIN-point_on_island.z*COS))
   if 0<=y<owned.shape[0] and 0<=x<owned.shape[1] and owned[y,x]:raise ValueError('Tiny island intersects observed source domain')
  removed.append(dict(vertices=len(component),bounds=[[min(p[i] for p in points),max(p[i] for p in points)] for i in range(3)],source_domain_intersection=False));bmesh.ops.delete(bm,geom=component,context='VERTS')
 bm.to_mesh(body.data);bm.free();body.data.update();(dest/'removed-offmap-islands.json').write_text(json.dumps(removed,indent=2)+'\n')
 mesh=body.data.copy();bpy.data.objects.remove(body,do_unlink=True);assign_mesh(objects[50],mesh)
 # Partition existing surface faces between native gameplay references. Their
 # union stays exact; no internal caps or shifted seams are introduced.
 combined=objects[50].data.copy();world=objects[50].matrix_world.copy()
 for index,lower in [(50,True),(51,False)]:
  bm=bmesh.new();bm.from_mesh(combined)
  for v in bm.verts:v.co=world@v.co
  remove=[]
  for f in bm.faces:
   center=f.calc_center_median();is_left=center.x<1163 and center.z>100 and -center.y*SIN-center.z*COS<180
   if is_left==lower:remove.append(f)
  bmesh.ops.delete(bm,geom=remove,context='FACES')
  bmesh.ops.delete(bm,geom=[v for v in bm.verts if not v.link_faces],context='VERTS')
  target=bpy.data.meshes.new('Leaning tree wood'+str(index));bm.to_mesh(target);bm.free()
  for layer in list(target.uv_layers):target.uv_layers.remove(layer)
  for layer in list(target.color_attributes):target.color_attributes.remove(layer)
  target.materials.clear();assign_mesh(objects[index],target)
 verts=[];faces=[]
 for i in range(1300):
  theta=rng.uniform(0,math.tau);zeta=rng.uniform(-1,1);rad=rng.random()**.35
  center=Vector((1195+115*rad*math.sqrt(1-zeta*zeta)*math.cos(theta),base_y-10+135*rad*math.sqrt(1-zeta*zeta)*math.sin(theta),550+76*rad*zeta))
  size=rng.uniform(4,8);normal=Vector((rng.uniform(-1,1),rng.uniform(-1,1),rng.uniform(-.7,.9))).normalized();side=normal.cross(Vector((0,0,1))).normalized();up=normal.cross(side).normalized();offset=len(verts)
  verts.extend([center-side*size,center+up*size*.65,center+side*size,center-up*size*.65]);faces.extend([(offset,offset+1,offset+2),(offset,offset+2,offset+3)])
 mesh=bpy.data.meshes.new('Complete inferred leaning-tree crown');mesh.from_pydata(verts,[],faces);mesh.update();mat=bpy.data.materials.new('Unknown off-map foliage');mat.diffuse_color=(.34,.34,.34,1);mesh.materials.append(mat)
 crown=bpy.data.objects.new('Tree19 inferred off-map crown',mesh);working.objects.link(crown);crown_node='foliage-tree19-inferred-crown'
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
 foot=Vector((1158,base_y,.02))
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
  parts=[p for p in group['parts'] if p.get('obstacle') not in (50,51) and (f"building-{p['obstacle']:03}" if 'obstacle' in p else p['node']) in nodes]
  if parts:groups.append(dict(group,parts=parts))
 groups.append(dict(id=asset,name=name,parts=[dict(obstacle=50,name='Main stem and crossing branch'),dict(obstacle=51,name='Left source fork'),dict(node=crown_node,name='Inferred complete crown')]))
 catalog.update(version=2,groups=groups,canonical_owners={f"building-{p['obstacle']:03}" if 'obstacle' in p else p['node']:g['id'] for g in groups for p in g['parts']})
 cat=dest/'catalog.json';cat.write_text(json.dumps(catalog,indent=2)+'\n')
 inventory(dest/'inventory',collection_name=working.name,map_name='Croisement01',source_path=OUT/'baseline/covered.png',patch_manifest=OUT/'source-states/layers.json')
 masks=dest/'masks.json';masks.write_text(json.dumps(native,indent=2)+'\n');source=dest/'source-masks.json';source.write_text(json.dumps(dict(version=1,mask_inventory=str(masks),projections=dict(exterior=dict(state='Initial northern forked oak wood only',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=asset,mask_indices=[219])]))),indent=2)+'\n')
 review=dest/'grouping-review.json';review.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(cat),inventory_sha256=sha(dest/'inventory/inventory.json'),evidence='Native mask19 and parts50/51 show a main trunk and left fork. The diagonal source branch crosses in front of neighboring tree20 and is separately excluded from that published tree. Conservative wood ownership excludes foreground leaf patches; off-map crown remains inferred.'),indent=2)+'\n')
 worker=dest/'assets'/asset
 prepare(worker,asset_id=asset,scene_name='Croisement01 Refinement',collection_name=working.name,source_path=OUT/'baseline/covered.png',grouping_manifest=cat,inventory_path=dest/'inventory/inventory.json',review_path=review,source_mask_manifest=source,width=384,height=384,framing_padding=1.2,lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05))
 validate(worker);modified(worker);(worker/'inspection').mkdir(exist_ok=True)
 (dest/'construction.json').write_text(json.dumps(dict(status='Private candidate; self-review required',model_sha256=sha(worker/'model.blend'),native_parts=[50,51],wood_pixels=int(np.count_nonzero(np.asarray(wood))),deferred_pixels=int(np.count_nonzero(np.asarray(ImageChops.subtract(alpha,wood)))),trace=trace,crown_width=230,crown_depth=270,whole_map_duplicate=False,limitations=['Native ground and final foot seating need contact review.','All off-map wood and crown depth are inferred; hidden appearance is gray.','Source branch order against published tree20 requires joint review.']),indent=2)+'\n')
 import render_candidate
 sys.argv=['render','--',str(worker)];render_candidate.main()
if __name__=='__main__':main()
