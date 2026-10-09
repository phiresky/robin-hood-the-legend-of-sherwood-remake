"""Prepare a private approved seven-component well export with current gameplay intact."""
import copy,hashlib,json,shutil,sys,re
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement';O=W/'restart2/restart42-approved-well-export-v2';assert not O.exists();sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest();R=ROOT/'level-editor/work/croisement02-refinement/restart3-review-batches/next-six-textures-channels-v1/user-approval.json';assert sha(R)=='7f6abc55fa21eee6ebd3b10c4f079e91e7f8159127a30a23fb9a6955d52e3f9d';asset='york-market-roofed-stone-well';member=next(m for m in json.loads(R.read_text())['decisions']if m['asset_id']==asset);source=Path(member['model']);assert sha(source)==member['model_sha256'];live=ROOT/'level-editor/library/3d-assets/york'/asset/'asset.json';live_sha=sha(live);descriptor=json.loads(live.read_text());pivot=descriptor['source_origin_scene'];assert shutil.disk_usage(ROOT).free>10*1024**3;assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire
acquire(slots=2)
import bpy
from export_editor import export_asset_library
from refinement_workspace import _geometry
bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene;scene.name='york Refinement';scene.render.threads_mode='FIXED';scene.render.threads=2;objects=[o for o in scene.objects if o.type=='MESH'];assert len(objects)==7 and all(o.get('asset_group')==asset for o in objects);before={o.name:_geometry(o,protect_appearance=True)for o in objects};working=bpy.data.collections.get('york Working')
if working is None:working=bpy.data.collections.new('york Working');scene.collection.children.link(working)
for o in objects:
 if o.name not in working.objects:working.objects.link(o)
catalog=json.loads((ROOT/'level-editor/refinement/catalogs/york.json').read_text());group=next(g for g in catalog['groups']if g['id']==asset);components={node:sorted(o.get('projection_component')for o in objects if o.get('source_node')==node)for node in ('building-035','building-036','building-037')};assert [len(components[n])for n in sorted(components)]==[5,1,1]
original_components={o.name:o.get('projection_component')for o in objects};slug=lambda name:re.sub('[^a-z0-9]+','-',name.lower()).strip('-')
for o in objects:o['projection_component']=slug(original_components[o.name])
source_component_labels=copy.deepcopy(components);components={node:[slug(value)for value in values]for node,values in components.items()}
assert all(len(values)==len(set(values))and all(values)for values in components.values())
for part in group['parts']:part['components']=components[f"building-{part['obstacle']:03d}"]
for o in objects:
 if not o.get('part_name'):o['part_name']=o.get('projection_component')or o.name
 if not o.get('asset_name'):o['asset_name']=group['name']
report=export_asset_library('york',O/'3d-assets',W/'baseline/york.rhp.json',standalone_pivots={asset:pivot},asset_ids=[asset],catalog=catalog)
for o in objects:o['projection_component']=original_components[o.name]
assert before=={o.name:_geometry(o,protect_appearance=True)for o in objects};assert sha(source)==member['model_sha256']and sha(live)==live_sha;p=O/'3d-assets'/asset/'asset.json';d=json.loads(p.read_text());assert d['source_origin_scene']==pivot;d['gameplay']=copy.deepcopy(descriptor['gameplay']);p.write_text(json.dumps(d,separators=(',',':'))+'\n');index=O/'3d-assets/index.json';j=json.loads(index.read_text());e=next(x for x in j['assets']if x['id']==asset);e['editor']=d;e['descriptor_sha256']=sha(p);index.write_text(json.dumps(j,separators=(',',':'))+'\n');(O/'private-catalog.json').write_text(json.dumps(catalog,indent=2)+'\n');(O/'export-report.json').write_text(json.dumps({'status':'PRIVATE_EXPORT_ROUNDTRIP_VERIFICATION_REQUIRED','asset_id':asset,'approval_receipt_sha256':sha(R),'approved_model_sha256':sha(source),'source_geometry_uv_materials_exact':True,'source_origin_scene':pivot,'gameplay_exact':True,'source_components':components,'source_component_display_labels':source_component_labels,'temporary_component_identity_mapping_restored':True,'live_descriptor_sha256':live_sha,'model_sha256':sha(p.with_name('model.glb')),'descriptor_sha256':sha(p),'export_report':report,'canonical_writes':False},indent=2)+'\n');print(O)
