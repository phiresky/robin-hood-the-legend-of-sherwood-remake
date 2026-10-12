"""Private continuous stem hypothesis from conservative native bark evidence."""
import json,math,shutil,sys
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
sys.path.insert(0,str(Path(__file__).parent))
from restart2_tree18 import OUT,SIN,COS,union,assign_mesh,fit_native_width
import restart2_tree05 as helpers
from refinement_workspace import prepare,modified,validate
from refinement_inventory import inventory
from evidence_io import sha
from render_slots import acquire
from PIL import Image
helpers.DENSE=True
acquire(slots=2)
assert shutil.disk_usage(OUT).free>10*1024**3+512*1024**2
available=int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024
assert available>6*1024**3
base=OUT/'restart2';dest=base/'tree14-wood-v6';dest.mkdir(exist_ok=False)
bank=base/'tree05-regional-banks-v18/assets/croisement01-tree-05/model.blend'
assert sha(bank)=='7926500e407950bd66b78d13e0ce65b3829dd2263d7db4ba5c57a7ece5dbc818'
bpy.ops.wm.open_mainfile(filepath=str(bank));bpy.context.preferences.filepaths.save_version=0
working=bpy.data.collections['Croisement01 Working'];terrain={'ground',*[f'building-{i:03d}' for i in range(10)]}
for o in list(bpy.data.objects):
 if o.type=='MESH' and o.get('source_node') not in terrain:bpy.data.objects.remove(o,do_unlink=True)
asset='croisement01-tree-14';node='scenery-tree14-wood';asset_label='Northern Twisted Tree'
core=base/'tree14-upper-ownership-v1/extended-known-wood-domain.png';wood=Image.open(core).convert('L');wood.save(dest/'wood-domain.png')
# The source traces define branch centerlines; unseen depth remains inferred.
trace=json.loads((base/'tree14-source-preparation-v1/preparation.json').read_text())['trace_screen_coordinates']
trace['root_left']=[[849,395,18],[821,395,10],[807,402,6],[794,413,.8]]
trace['root_front']=[[850,405,20],[821,411,12],[807,423,8],[794,436,.8]]
trace['root_forward_base']=[[852,401,18],[849,416,14],[849,424,4],[851,432,.5]]
trace['root_forward_west']=[[846,410,10],[838,418,5],[835,425,2],[837,429,.5]]
trace['root_right']=[[851,405,18],[873,398,9],[892,391,7],[905,397,5],[914,402,.4]]
base_y=-412/SIN
point=lambda x,y:Vector((x,base_y,(412-y)/COS))
main=trace['main_stem'];body=helpers.tube('Continuous twisted trunk',[point(x,y) for x,y,r in main],[r for x,y,r in main]);body['defer_union']=True
# Full lower stem mask sections establish its silhouette, unlike an interior
# conservative color patch. Root and left-branch zones are fitted separately.
fit=np.asarray(wood)>0;yy,xx=np.indices(fit.shape);fit&=(yy+122>=270)&(yy+122<=385)&(xx+689>=794)
fit_native_width(body,fit,'west-cut',x0=689,y0=122)
profiles=json.loads((base/'tree14-upper-branch-profiles-v1/profiles.json').read_text())['profiles']
for branch,profile in profiles.items():
 original=trace[branch];xy=np.array([row[:2] for row in original],float);lengths=np.linalg.norm(np.diff(xy,axis=0),axis=1);distance=np.r_[0,np.cumsum(lengths)];samples=profile['samples'];new=[]
 for row in samples:
  along=row['along'];center=np.array([np.interp(along,distance,xy[:,axis]) for axis in range(2)]);measured=np.array([row['x'],row['y']]);shift=measured-center;norm=np.linalg.norm(shift)
  if norm>2:shift*=2/norm
  radius=min(row['smoothed_radius']+.15,float(np.interp(along,distance,[v[2] for v in original])))
  new.append([*(center+shift),max(.4,radius)])
 for axis in range(3):
  values=np.array([row[axis] for row in new]);smoothed=np.convolve(np.pad(values,(2,2),mode='edge'),[.1,.2,.4,.2,.1],mode='valid')
  for row,value in zip(new,smoothed):row[axis]=float(value)
 # Keep connected parent attachment fixed; native cross-sections constrain the
 # thinner distal branch, while the taper tip is retained from the source trace.
 new[0]=original[0];new.append(profile['tip']);trace[branch]=new
for name,rows in trace.items():
 if name=='main_stem':continue
 if name.startswith('root_'):
  centers=[]
  for j,(x,y,radius) in enumerate(rows):
   z=(412-y)/COS if j==0 else max(.4,radius*.7)
   centers.append(Vector((x,-(y+z*COS)/SIN,z)))
 else:
  offsets={'upper_branch':-70,'right_fork':90,'right_tip':130,'left_main':-90,'left_upper':60,'left_twig':90}
  # Native camera rays leave branch depth unconstrained. Offset progressively
  # from the connected base while preserving every traced screen centerline.
  ray=Vector((0,-COS,SIN));start_depth={'right_tip':26,'left_twig':17}.get(name,0);centers=[point(x,y)+ray*(start_depth+(offsets[name]-start_depth)*(j/(len(rows)-1))**1.5) for j,(x,y,radius) in enumerate(rows)]
 union(body,helpers.tube(name,centers,[radius for x,y,radius in rows]))
# Existing native forks carry inferred depth; no additional crown is inferred
# from neighboring foliage layers whose source ownership remains separate.
bpy.context.view_layer.objects.active=body;m=body.modifiers.new('Joined continuous wood','REMESH');m.mode='VOXEL';m.voxel_size=.65;m.use_remove_disconnected=False;bpy.ops.object.modifier_apply(modifier=m.name)
# Remove only tiny voxel remnants, retaining the complete connected wood.
# Fresh source-ray coverage below independently checks the resulting model.
bm=bmesh.new();bm.from_mesh(body.data);remaining=set(bm.verts);components=[]
while remaining:
 component={remaining.pop()};stack=list(component)
 while stack:
  vertex=stack.pop()
  for edge in vertex.link_edges:
   other=edge.other_vert(vertex)
   if other in remaining:remaining.remove(other);component.add(other);stack.append(other)
 components.append(component)
components.sort(key=len,reverse=True);assert len(components[0])>100000 and all(len(c)<=20 for c in components[1:])
bmesh.ops.delete(bm,geom=[v for c in components[1:] for v in c],context='VERTS');bm.to_mesh(body.data);bm.free();body.data.update()
mesh=body.data.copy();bpy.data.objects.remove(body,do_unlink=True);dummy=bpy.data.meshes.new('Wood slot');mat=bpy.data.materials.new('Unknown wood');mat.diffuse_color=(.34,.34,.34,1);dummy.materials.append(mat);obj=bpy.data.objects.new('Tree14 wood',dummy);working.objects.link(obj)
for k,v in dict(source_node=node,asset_group=asset,asset_name=asset_label,part_name='Conservative native bark and inferred continuous wood').items():obj[k]=v
assign_mesh(obj,mesh);bpy.context.view_layer.update()
native=json.loads((OUT/'baseline/masks/manifest.json').read_text())
for row in native['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
row=next(r for r in native['masks'] if r['index']==14);assert row['obstacle_indices']==[];native['masks'].append(dict(row,index=214,png=str(dest/'wood-domain.png')))
catalog=json.loads((OUT/'catalog.json').read_text());groups=[]
for group in catalog['groups']:
 parts=[p for p in group['parts'] if (f"building-{p['obstacle']:03d}" if 'obstacle' in p else p['node']) in terrain]
 if parts:groups.append(dict(group,parts=parts))
groups.append(dict(id=asset,name=asset_label,parts=[dict(node=node,name='Partly observed woody stem')]))
catalog.update(version=2,groups=groups,canonical_owners={f"building-{p['obstacle']:03d}" if 'obstacle' in p else p['node']:g['id'] for g in groups for p in g['parts']})
cat=dest/'catalog.json';cat.write_text(json.dumps(catalog,indent=2)+'\n');inventory(dest/'inventory',collection_name=working.name,map_name='Croisement01',source_path=OUT/'baseline/covered.png',patch_manifest=OUT/'source-states/layers.json')
masks=dest/'masks.json';masks.write_text(json.dumps(native,indent=2)+'\n');source=dest/'source-masks.json';source.write_text(json.dumps(dict(version=1,mask_inventory=str(masks),projections=dict(exterior=dict(state='Conservative native14 bark',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=asset,mask_indices=[214])]))),indent=2)+'\n')
review=dest/'grouping-review.json';review.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(cat),inventory_sha256=sha(dest/'inventory/inventory.json'),evidence='9151 native lower trunk/root pixels only; 3181 overlapping upper foliage pixels deferred. Native043/044/045 proxy correspondence measured separately; no gameplay changes. Root depth and hidden wood inferred; crown excluded.'),indent=2)+'\n')
worker=dest/'assets'/asset
if '--geometry-only' in sys.argv:
 worker.mkdir(parents=True);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'baseline.blend'))
 (dest/'geometry-checkpoint.json').write_text(json.dumps(dict(status='PRIVATE_MEASURED_UPPER_BRANCH_CORRECTION_PENDING_AUDIT',baseline_sha256=sha(worker/'baseline.blend'),trace=trace,scope='Correct lower native root contour; preserve prior nonplanar limbs and private bank. No appearance approval.'),indent=2)+'\n')
 raise SystemExit(0)
prepare(worker,asset_id=asset,scene_name='Croisement01 Refinement',collection_name=working.name,source_path=OUT/'baseline/covered.png',grouping_manifest=cat,inventory_path=dest/'inventory/inventory.json',review_path=review,source_mask_manifest=source,width=384,height=384,framing_padding=1.2,lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05));validate(worker);modified(worker)
(dest/'construction.json').write_text(json.dumps(dict(status='PRIVATE_GEOMETRY_PENDING_SAVED_VIEW_SOURCE_AND_CONTACT_REVIEW',model_sha256=sha(worker/'model.blend'),known_bark_pixels=9151,deferred_mixed_pixels=3181,bank_sha256=sha(bank),source_partition_sha256=sha(base/'tree14-source-preparation-v1/preparation.json'),crown_excluded=True,inferred_nonplanar_branches=True,no_gameplay_mutation=True),indent=2)+'\n')
(worker/'inspection').mkdir(exist_ok=True)
import render_candidate;sys.argv=['render','--',str(worker)];render_candidate.main()
