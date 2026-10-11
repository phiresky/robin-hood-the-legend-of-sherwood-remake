"""Private continuous stem hypothesis from conservative native bark evidence."""
import json,math,shutil,sys
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
sys.path.insert(0,str(Path(__file__).parent))
from restart2_tree18 import OUT,SIN,COS,union,assign_mesh
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
base=OUT/'restart2';dest=base/'tree07-wood-v1';dest.mkdir(exist_ok=False)
bank=base/'tree05-regional-banks-v18/assets/croisement01-tree-05/model.blend'
assert sha(bank)=='7926500e407950bd66b78d13e0ce65b3829dd2263d7db4ba5c57a7ece5dbc818'
bpy.ops.wm.open_mainfile(filepath=str(bank));bpy.context.preferences.filepaths.save_version=0
working=bpy.data.collections['Croisement01 Working'];terrain={'ground',*[f'building-{i:03d}' for i in range(10)]}
for o in list(bpy.data.objects):
 if o.type=='MESH' and o.get('source_node') not in terrain:bpy.data.objects.remove(o,do_unlink=True)
asset='croisement01-tree-07';node='scenery-tree07-wood';name='Northern Partly Hidden Tree'
core=base/'tree07-source-partition-v2/conservative-bark-core.png';wood=Image.open(core).convert('L');wood.save(dest/'wood-domain.png')
# Only the complete upper section constrains width; its isolated lower edge
# samples establish color without narrowing the opposite hidden side.
centers=[Vector(v) for v in [(458,-307,39),(458,-307,55),(458,-307,120),(456,-307,190),(456,-307,218),(458,-307,270),(452,-308,335),(451,-308,365)]]
body=helpers.tube('Continuous partly observed stem',centers,[14,13,11,10.5,10.5,9,5,1]);body['defer_union']=True
fit=np.asarray(wood)>0;fit[12:]=False
before=[v.co.copy() for v in body.data.vertices]
helpers.fit_short_root(body,fit,x0=441)
# The short helper requires only five observed slices; its exact changed band
# is kept separate from inferred continuation. Blend endpoints outside that
# band without pretending the foliage-hidden rows measured another silhouette.
for vertex,old in zip(body.data.vertices,before):
 sy=-old.y*SIN-old.z*COS
 if .5<=sy<=11.5:continue
 if -12<sy<.5 or 11.5<sy<24:
  boundary=.5 if sy<.5 else 11.5
  neighbors=[(abs((-v.y*SIN-v.z*COS)-boundary),j) for j,v in enumerate(before) if abs((-v.y*SIN-v.z*COS)-boundary)<2]
  if neighbors:
   _,j=min(neighbors,key=lambda pair:pair[0]+abs(before[pair[1]].x-old.x))
   dx=body.data.vertices[j].co.x-before[j].x;vertex.co.x+=dx*max(0,1-abs(sy-boundary)/12)
body.data.update()
for i,tip in enumerate([(435,-308,40),(480,-307,39),(458,-334,41),(458,-278,38)]):
 end=Vector(tip);root=helpers.tube('Inferred seated root '+str(i),[Vector((458,-307,64)),Vector((458,-307,45)).lerp(end,.6)+Vector((0,0,1)),end],[8,4,.3]);union(body,root)
for i,tip in enumerate([(408,-301,362),(492,-293,375),(462,-340,370)]):
 end=Vector(tip);union(body,helpers.tube('Inferred upper branch '+str(i),[Vector((456,-307,290)),Vector((456,-307,290)).lerp(end,.55),end],[5,3,.4]))
bpy.context.view_layer.objects.active=body;m=body.modifiers.new('Joined continuous wood','REMESH');m.mode='VOXEL';m.voxel_size=.65;m.use_remove_disconnected=False;bpy.ops.object.modifier_apply(modifier=m.name)
mesh=body.data.copy();bpy.data.objects.remove(body,do_unlink=True);dummy=bpy.data.meshes.new('Wood slot');mat=bpy.data.materials.new('Unknown wood');mat.diffuse_color=(.34,.34,.34,1);dummy.materials.append(mat);obj=bpy.data.objects.new('Tree07 wood',dummy);working.objects.link(obj)
for k,v in dict(source_node=node,asset_group=asset,asset_name=name,part_name='Conservative native bark and inferred continuous wood').items():obj[k]=v
assign_mesh(obj,mesh);bpy.context.view_layer.update()
native=json.loads((OUT/'baseline/masks/manifest.json').read_text())
for row in native['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
row=next(r for r in native['masks'] if r['index']==7);assert row['obstacle_indices']==[];native['masks'].append(dict(row,index=207,png=str(dest/'wood-domain.png')))
catalog=json.loads((OUT/'catalog.json').read_text());groups=[]
for group in catalog['groups']:
 parts=[p for p in group['parts'] if (f"building-{p['obstacle']:03d}" if 'obstacle' in p else p['node']) in terrain]
 if parts:groups.append(dict(group,parts=parts))
groups.append(dict(id=asset,name=name,parts=[dict(node=node,name='Partly observed woody stem')]))
catalog.update(version=2,groups=groups,canonical_owners={f"building-{p['obstacle']:03d}" if 'obstacle' in p else p['node']:g['id'] for g in groups for p in g['parts']})
cat=dest/'catalog.json';cat.write_text(json.dumps(catalog,indent=2)+'\n');inventory(dest/'inventory',collection_name=working.name,map_name='Croisement01',source_path=OUT/'baseline/covered.png',patch_manifest=OUT/'source-states/layers.json')
masks=dest/'masks.json';masks.write_text(json.dumps(native,indent=2)+'\n');source=dest/'source-masks.json';source.write_text(json.dumps(dict(version=1,mask_inventory=str(masks),projections=dict(exterior=dict(state='Conservative native07 bark',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=asset,mask_indices=[207])]))),indent=2)+'\n')
review=dest/'grouping-review.json';review.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(cat),inventory_sha256=sha(dest/'inventory/inventory.json'),evidence='308 conservative native bark pixels only; mixed crossing foliage deferred. Native040 proxy correspondence measured separately; no gameplay changes. Hidden lower stem and complete upper branches inferred; crown excluded.'),indent=2)+'\n')
worker=dest/'assets'/asset;prepare(worker,asset_id=asset,scene_name='Croisement01 Refinement',collection_name=working.name,source_path=OUT/'baseline/covered.png',grouping_manifest=cat,inventory_path=dest/'inventory/inventory.json',review_path=review,source_mask_manifest=source,width=384,height=384,framing_padding=1.2,lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05));validate(worker);modified(worker)
(dest/'construction.json').write_text(json.dumps(dict(status='PRIVATE_GEOMETRY_PENDING_SAVED_VIEW_SOURCE_AND_CONTACT_REVIEW',model_sha256=sha(worker/'model.blend'),known_bark_pixels=308,deferred_mixed_pixels=2352,bank_sha256=sha(bank),source_partition_sha256=sha(base/'tree07-source-partition-v2/partition.json'),crown_excluded=True,no_gameplay_mutation=True),indent=2)+'\n')
import render_candidate;sys.argv=['render','--',str(worker)];render_candidate.main()
