"""Native mask03 decorative tree, without inventing a gameplay obstacle association."""
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
 parser=argparse.ArgumentParser();parser.add_argument('--revision',type=int,default=2);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else []);dest=OUT/f'restart2/tree03-v{args.revision}';dest.mkdir(exist_ok=False);acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement01-grouped.blend'));bpy.context.preferences.filepaths.save_version=0;working=bpy.data.collections['Croisement01 Working'];asset='croisement01-tree-03';wood_node='scenery-tree03-wood';crown_node='foliage-tree03-inferred-crown';name='Northwest Slender Decorative Tree'
 native=json.loads((OUT/'baseline/masks/manifest.json').read_text())
 for row in native['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
 row=next(r for r in native['masks'] if r['index']==3);assert row['obstacle_indices']==[];alpha=Image.open(row['png']).convert('L');wood=alpha.copy();draw=ImageDraw.Draw(wood);draw.polygon([(0,377),(10,377),(10,419),(0,419)],fill=0)
 if args.revision>=3:
  foreground=next(r for r in native['masks'] if r['index']==4);foreign=Image.open(foreground['png']).convert('L');cut=Image.new('L',alpha.size);cut.paste(foreign,(foreground['box_top_left'][0]-row['box_top_left'][0],foreground['box_top_left'][1]-row['box_top_left'][1]));wood=ImageChops.subtract(wood,cut)
  (dest/'foreground-exclusion.json').write_text(json.dumps(dict(native_mask=3,excluded_mask=4,reason='Foreground tree04 branch crosses the rear thin tree03 and has a larger native character threshold; preserve its pixels for tree04 instead of painting them on tree03.',foreground_sha256=sha(Path(foreground['png']))),indent=2)+'\n')
 wood.save(dest/'wood-domain.png');ImageChops.subtract(alpha,wood).save(dest/'deferred-domain.png');native['masks'].append(dict(row,index=203,png=str(dest/'wood-domain.png')))
 base_y=-419/SIN
 def point(x,y):return Vector((x,base_y,(419-y)/COS))
 trace=[(170,418,3),(171,407,8),(173,380,8),(173,340,7),(172,300,6),(171,260,5),(170,220,4),(170,180,4),(172,140,4),(174,100,2.5),(174,60,1.4),(174,20,1.3),(174,-25,1.4)]
 if args.revision>=4:trace[0]=(170,418,8.5)
 body=tube('Decorative thin trunk',[point(x,y) for x,y,r in trace]+[Vector((174,base_y,585)),Vector((174,base_y,625))],[r for x,y,r in trace]+[2,1]);body['defer_union']=True;rng=random.Random(103)
 if args.revision>=4:
  base=point(170,418)
  for angle in [.4,2.5,4.6]:
   tip=base+Vector((math.cos(angle)*15,math.sin(angle)*17,0));tip.z=.1
   union(body,tube('Inferred low root buttress',[point(171,409),base.lerp(tip,.55)+Vector((0,0,2)),tip],[3.5,2,.35]))
 for i in range(7):
  angle=math.tau*i/7;start=Vector((174,base_y,570+i*4));tip=Vector((174+math.cos(angle)*45,base_y+math.sin(angle)*55,645+rng.uniform(-15,20)));union(body,tube('Inferred small bough',[start,start.lerp(tip,.55)+Vector((0,0,8)),tip],[2.5,1.3,.35]))
 bpy.context.view_layer.objects.active=body;modifier=body.modifiers.new('Continuous slender wood','REMESH');modifier.mode='VOXEL';modifier.voxel_size=.65;modifier.use_remove_disconnected=False;bpy.ops.object.modifier_apply(modifier=modifier.name)
 contour=np.asarray(alpha)>0;contour[377:]=False;fit=fit_native_width(body,contour,'west-cut',x0=155,y0=0);(dest/'native-contour-fit.json').write_text(json.dumps(fit,indent=2)+'\n')
 mesh=body.data.copy();bpy.data.objects.remove(body,do_unlink=True);dummy=bpy.data.meshes.new('Decorative wood slot');fallback=bpy.data.materials.new('Unknown decorative wood');fallback.diffuse_color=(.34,.34,.34,1);dummy.materials.append(fallback);obj=bpy.data.objects.new('Tree03 decorative wood',dummy);working.objects.link(obj);obj['source_node']=wood_node;obj['asset_group']=asset;obj['asset_name']=name;obj['part_name']='Scenery wood; no gameplay obstacle';assign_mesh(obj,mesh)
 verts=[];faces=[]
 for i in range(900):
  theta=rng.uniform(0,math.tau);zeta=rng.uniform(-1,1);rad=rng.random()**.35;center=Vector((174+60*rad*math.sqrt(1-zeta*zeta)*math.cos(theta),base_y+75*rad*math.sqrt(1-zeta*zeta)*math.sin(theta),650+55*rad*zeta));size=rng.uniform(3,6);normal=Vector((rng.uniform(-1,1),rng.uniform(-1,1),rng.uniform(-.7,.9))).normalized();side=normal.cross(Vector((0,0,1))).normalized();up=normal.cross(side).normalized();offset=len(verts);verts.extend([center-side*size,center+up*size*.65,center+side*size,center-up*size*.65]);faces.extend([(offset,offset+1,offset+2),(offset,offset+2,offset+3)])
 mesh=bpy.data.meshes.new('Inferred complete crown');mesh.from_pydata(verts,[],faces);mesh.update();mat=bpy.data.materials.new('Unknown off-map foliage');mat.diffuse_color=(.34,.34,.34,1);mesh.materials.append(mat);crown=bpy.data.objects.new('Tree03 inferred crown',mesh);working.objects.link(crown)
 for k,v in dict(source_node=crown_node,asset_group=asset,asset_name=name,part_name='Inferred crown',projection_component='crown').items():crown[k]=v
 uv=mesh.uv_layers.new(name='Source UV');known=mesh.color_attributes.new(name='Source ownership',type='FLOAT_COLOR',domain='CORNER');mesh.color_attributes.active_color=known
 for loop in mesh.loops:
  p=mesh.vertices[loop.vertex_index].co;uv.data[loop.index].uv=(p.x/1408,1-(-p.y*SIN-p.z*COS)/960);known.data[loop.index].color=(0,1,1,1)
 terrain_nodes={'ground'}|{f'building-{i:03d}' for i in [*range(10),*range(76,81)]};vertices=[];polygons=[];owners=[]
 for support in list(working.all_objects):
  if support.type!='MESH' or support.get('source_node') not in terrain_nodes:continue
  offset=len(vertices);vertices.extend(support.matrix_world@v.co for v in support.data.vertices);polygons.extend(tuple(offset+i for i in face.vertices) for face in support.data.polygons);owners.extend([support.get('source_node')]*len(support.data.polygons))
 tree=BVHTree.FromPolygons(vertices,polygons);ray=Vector((0,-COS,SIN));anchor=point(170,418);origin=anchor+ray*5000;skipped=[]
 while True:
  hit,normal,index,distance=tree.ray_cast(origin,-ray,10000)
  if hit is None:raise ValueError('No upward terrain support')
  if normal.z>.35:break
  skipped.append(dict(point=list(hit),node=owners[index],normal=list(normal)));origin=hit-ray*.01
 shift=ray*((hit-anchor).dot(ray)+.2)
 for target in [obj,crown]:
  for vertex in target.data.vertices:vertex.co+=shift
  target.data.update()
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
 masks=dest/'masks.json';masks.write_text(json.dumps(native,indent=2)+'\n');source=dest/'source-masks.json';source.write_text(json.dumps(dict(version=1,mask_inventory=str(masks),projections=dict(exterior=dict(state='Native mask03 decorative tree wood',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=asset,mask_indices=[203])]))),indent=2)+'\n');review=dest/'grouping-review.json';review.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(cat),inventory_sha256=sha(dest/'inventory/inventory.json'),evidence='Mask03 is a slender independent scenery tree with no explicit mask-to-obstacle association; left basal mixed pixels and foreground tree04 branch explicitly deferred. Crown complete beyond source border; only permitted reference tree construction patterns used.'),indent=2)+'\n')
 worker=dest/'assets'/asset;prepare(worker,asset_id=asset,scene_name='Croisement01 Refinement',collection_name=working.name,source_path=OUT/'baseline/covered.png',grouping_manifest=cat,inventory_path=dest/'inventory/inventory.json',review_path=review,source_mask_manifest=source,width=384,height=384,framing_padding=1.2,lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05));validate(worker);modified(worker);(worker/'inspection').mkdir(exist_ok=True)
 (dest/'construction.json').write_text(json.dumps(dict(status='Private unapproved geometry candidate',model_sha256=sha(worker/'model.blend'),native_mask=3,native_obstacles=[],no_gameplay_mutation=True,wood_pixels=int(np.count_nonzero(np.asarray(wood))),deferred_pixels=int(np.count_nonzero(np.asarray(ImageChops.subtract(alpha,wood)))),crown_width=120,crown_depth=150,trace=trace),indent=2)+'\n');import render_candidate;sys.argv=['render','--',str(worker)];render_candidate.main()
if __name__=='__main__':main()
