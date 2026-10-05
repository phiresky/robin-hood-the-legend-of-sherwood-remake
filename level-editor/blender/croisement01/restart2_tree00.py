"""Compact west-border trunk with inferred complete depth beyond the map edge."""
import json,math,random,shutil,sys
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
from PIL import Image,ImageDraw,ImageChops
sys.path.insert(0,str(Path(__file__).parent))
from restart2_tree18 import OUT,SIN,COS,union,assign_mesh,fit_native_width
from refinement_workspace import prepare,modified,validate
from refinement_inventory import inventory
from evidence_io import sha
from render_slots import acquire

from restart2_tree71 import tube

def main():
 if shutil.disk_usage(OUT).free<25*1024**3:raise ValueError('Disk floor25GiB')
 dest=OUT/'restart2/tree00-v2';dest.mkdir(exist_ok=False)
 acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement01-grouped.blend'));bpy.context.preferences.filepaths.save_version=0
 working=bpy.data.collections['Croisement01 Working'];asset='croisement01-tree-00';name='Western Foreground Forest Tree'
 objects={int(o['source_node'].split('-')[-1]):o for o in working.all_objects if o.type=='MESH' and o.get('source_node') in ['building-030']};assert len(objects)==1
 for o in objects.values():o['asset_group']=asset;o['asset_name']=name
 native=json.loads((OUT/'baseline/masks/manifest.json').read_text())
 for row in native['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
 row=next(r for r in native['masks'] if r['index']==0);alpha=Image.open(row['png']).convert('L');wood=Image.new('L',alpha.size);draw=ImageDraw.Draw(wood)
 # Lower rocks and foreground plants are not bark; retain only the visible
 # continuous trunk strip while the rounded hidden west side stays complete.
 outline=[(0,0),(20,0),(21,80),(24,160),(29,240),(34,320),(39,400),(42,442),(48,480),(58,522),(62,541),(52,550),(40,547),(30,520),(26,486),(28,466),(8,425),(0,380)]
 draw.polygon(outline,fill=255);wood=ImageChops.multiply(alpha,wood)
 wood.save(dest/'wood-domain.png');ImageChops.subtract(alpha,wood).save(dest/'deferred-foliage-domain.png')
 native['masks'].append(dict(row,index=200,png=str(dest/'wood-domain.png')))
 base_y=-558/SIN
 def point(x,y):return Vector((x,base_y,(558-y)/COS))
 trace=[(46,558,19),(40,520,22),(31,480,19),(24,440,21),(18,400,22),(12,340,24),(8,280,24),(3,220,24),(-1,160,23),(-5,100,22),(-8,40,22),(-9,-10,22)]
 centers=[point(x,y) for x,y,r in trace]+[Vector((-12,base_y-5,760)),Vector((-20,base_y-10,835)),Vector((-20,base_y-15,920))]
 body=tube('Continuous west-border trunk',centers,[r for x,y,r in trace]+[17,12,2]);body['defer_union']=True
 rng=random.Random(1)
 for i in range(9):
  a=math.tau*i/9;start=Vector((-20,base_y-15,850+i*7));tip=Vector((-15+math.cos(a)*105,base_y-15+math.sin(a)*128,965+rng.uniform(-15,45)))
  union(body,tube('Inferred crown bough'+str(i),[start,start.lerp(tip,.55)+Vector((0,0,12)),tip],[8,4,.8]))
 bpy.context.view_layer.objects.active=body;mod=body.modifiers.new('Continuous wood junctions','REMESH');mod.mode='VOXEL';mod.voxel_size=1.15;mod.use_remove_disconnected=False;bpy.ops.object.modifier_apply(modifier=mod.name)
 fit=fit_native_width(body,np.asarray(wood)>0,'west-cut',x0=0,y0=0)
 (dest/'native-contour-fit.json').write_text(json.dumps(fit,indent=2)+'\n')
 mesh=body.data.copy();bpy.data.objects.remove(body,do_unlink=True);assign_mesh(objects[30],mesh)
 verts=[];faces=[]
 for i in range(1300):
  theta=rng.uniform(0,math.tau);zeta=rng.uniform(-1,1);rad=rng.random()**.35
  center=Vector((-15+145*rad*math.sqrt(1-zeta*zeta)*math.cos(theta),base_y-15+170*rad*math.sqrt(1-zeta*zeta)*math.sin(theta),970+84*rad*zeta))
  size=rng.uniform(4,8);normal=Vector((rng.uniform(-1,1),rng.uniform(-1,1),rng.uniform(-.7,.9))).normalized();side=normal.cross(Vector((0,0,1))).normalized();up=normal.cross(side).normalized();offset=len(verts)
  verts.extend([center-side*size,center+up*size*.65,center+side*size,center-up*size*.65]);faces.extend([(offset,offset+1,offset+2),(offset,offset+2,offset+3)])
 mesh=bpy.data.meshes.new('Complete inferred leaning-tree crown');mesh.from_pydata(verts,[],faces);mesh.update();mat=bpy.data.materials.new('Unknown off-map foliage');mat.diffuse_color=(.34,.34,.34,1);mesh.materials.append(mat)
 crown=bpy.data.objects.new('Tree00 inferred off-map crown',mesh);working.objects.link(crown);crown_node='foliage-tree00-inferred-crown'
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
 foot=min((objects[30].matrix_world@v.co for v in objects[30].data.vertices),key=lambda p:p.z)
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
  parts=[p for p in group['parts'] if p.get('obstacle') not in (30,) and (f"building-{p['obstacle']:03}" if 'obstacle' in p else p['node']) in nodes]
  if parts:groups.append(dict(group,parts=parts))
 groups.append(dict(id=asset,name=name,parts=[dict(obstacle=30,name='Complete trunk and branches'),dict(node=crown_node,name='Inferred complete crown')]))
 catalog.update(version=2,groups=groups,canonical_owners={f"building-{p['obstacle']:03}" if 'obstacle' in p else p['node']:g['id'] for g in groups for p in g['parts']})
 cat=dest/'catalog.json';cat.write_text(json.dumps(catalog,indent=2)+'\n')
 inventory(dest/'inventory',collection_name=working.name,map_name='Croisement01',source_path=OUT/'baseline/covered.png',patch_manifest=OUT/'source-states/layers.json')
 masks=dest/'masks.json';masks.write_text(json.dumps(native,indent=2)+'\n');source=dest/'source-masks.json';source.write_text(json.dumps(dict(version=1,mask_inventory=str(masks),projections=dict(exterior=dict(state='Initial western foreground tree wood only',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=asset,mask_indices=[200])]))),indent=2)+'\n')
 review=dest/'grouping-review.json';review.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(cat),inventory_sha256=sha(dest/'inventory/inventory.json'),evidence='Native mask0 and part30 show the tall west-border trunk. Rocks and lower foreground foliage are excluded from wood. Hidden west side has full rounded depth and the complete northern crown is inferred beyond the image border.'),indent=2)+'\n')
 worker=dest/'assets'/asset
 prepare(worker,asset_id=asset,scene_name='Croisement01 Refinement',collection_name=working.name,source_path=OUT/'baseline/covered.png',grouping_manifest=cat,inventory_path=dest/'inventory/inventory.json',review_path=review,source_mask_manifest=source,width=384,height=384,framing_padding=1.2,lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05))
 validate(worker);modified(worker);(worker/'inspection').mkdir(exist_ok=True)
 (dest/'construction.json').write_text(json.dumps(dict(status='Private candidate; self-review required',model_sha256=sha(worker/'model.blend'),native_parts=[30],wood_pixels=int(np.count_nonzero(np.asarray(wood))),deferred_pixels=int(np.count_nonzero(np.asarray(ImageChops.subtract(alpha,wood)))),trace=trace,crown_width=290,crown_depth=340,whole_map_duplicate=False,limitations=['Native ground and final foot seating need contact review.','All off-map wood and crown depth are inferred; hidden appearance is gray.','Foreground rocks, dense canopy and lower plants remain separate source owners.']),indent=2)+'\n')
 import render_candidate
 sys.argv=['render','--',str(worker)];render_candidate.main()
if __name__=='__main__':main()
