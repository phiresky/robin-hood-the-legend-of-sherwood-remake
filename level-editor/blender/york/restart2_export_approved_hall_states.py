"""Export approved hall state surfaces privately with unchanged gameplay and pivots."""
import argparse,copy,hashlib,json,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/york-refinement'
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('state',choices=['initial-initial','initial-applied','applied-initial','applied-applied'])
args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
archive=BASE/'restart2/hall-textures-v2/approval-batch-v10'
record=json.loads((archive/'archive.json').read_text())
assert sha(archive/'user-approval.json')==record['receipt_sha256']=='534d552590823cbb5221e1ff52a082657360eb16f80ff5c42acfdb4980a2e3e8'
member=next(x for x in record['members']if x['state']==args.state)
source=Path(member['model']);assert sha(source)==member['model_sha256']
out=BASE/'restart2/hall-textures-v2/exports-v3'/args.state
if out.exists():raise FileExistsError(out)
if shutil.disk_usage(ROOT).free<25*1024**3:raise RuntimeError('Disk below25GiB')
asset='york-castle-great-hall';live=ROOT/'level-editor/library/3d-assets/york'/asset/'asset.json';live_sha=sha(live);descriptor=json.loads(live.read_text());pivot=descriptor['source_origin_scene']
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
import bpy
from export_editor import export_asset_library
from refinement_workspace import _geometry
bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.window.scene=bpy.data.scenes['york Refinement'];bpy.context.view_layer.update()
working=bpy.data.collections['york Working'];members=[o for o in working.objects if o.type=='MESH' and not o.hide_render and o.get('asset_group')==asset]
before={o.name:_geometry(o,protect_appearance=True)for o in members}
catalog=json.loads((ROOT/'level-editor/refinement/catalogs/york.json').read_text());group=next(x for x in catalog['groups']if x['id']==asset)
allowed={'scenery-york-great-hall-upper-front-wall','scenery-york-great-hall-northwest-arch','scenery-york-great-hall-candle-stand'}
extras={o['source_node']:o for o in members if o['source_node'].startswith('scenery-')}
for part in group['parts']:
 if part.get('obstacle')==769:
  selected={o.get('projection_component') for o in members if o['source_node']=='building-769'}
  if selected!={'hall','hall-foundation'}:raise ValueError('Unexpected approved769 receivers: '+repr(selected))
  part['components']=['hall','hall-foundation']
if set(extras)-allowed:raise ValueError('Unexpected authored receiver: '+repr(set(extras)-allowed))
for obj in members:
 if not obj.get('part_name'):obj['part_name']=obj.name.rsplit(' / ',1)[-1]
 if not obj.get('asset_name'):obj['asset_name']=group['name']
for node,obj in extras.items():
 group['parts'].append({'node':node,'name':obj['part_name']})
 catalog['canonical_owners'][node]=asset
report=export_asset_library('york',out/'3d-assets',BASE/'baseline/york.rhp.json',standalone_pivots={asset:pivot},asset_ids=[asset],catalog=catalog)
assert before=={o.name:_geometry(o,protect_appearance=True)for o in members}
assert sha(source)==member['model_sha256'] and sha(live)==live_sha
p=out/'3d-assets'/asset/'asset.json';generated=json.loads(p.read_text());assert generated['source_origin_scene']==pivot
generated['gameplay']=copy.deepcopy(descriptor['gameplay']);p.write_text(json.dumps(generated,separators=(',',':'))+'\n')
idx=out/'3d-assets/index.json';index=json.loads(idx.read_text());entry=next(x for x in index['assets']if x['id']==asset);entry['editor']=generated;entry['descriptor_sha256']=sha(p);idx.write_text(json.dumps(index,separators=(',',':'))+'\n')
(out/'private-catalog.json').write_text(json.dumps(catalog,indent=2)+'\n')
(out/'export-report.json').write_text(json.dumps({'state':args.state,'asset_id':asset,'approved_model':str(source),'approved_model_sha256':sha(source),'approval_receipt_sha256':record['receipt_sha256'],'preserved_source_pivot':pivot,'source_geometry_uv_materials_unchanged':True,'gameplay_preserved':True,'live_descriptor':str(live),'live_descriptor_sha256':live_sha,'exported_parts':[x['node']for x in generated['parts']],'authored_receivers':sorted(extras),'scope':'Private static state derivative; no runtime/candle animation or neighbor-context publication','report':report,'model_sha256':sha(p.with_name('model.glb')),'descriptor_sha256':sha(p),'live_writes':False},indent=2)+'\n')
print(out)
