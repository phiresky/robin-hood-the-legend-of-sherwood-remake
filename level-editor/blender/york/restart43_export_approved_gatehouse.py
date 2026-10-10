"""Privately export approved gatehouse channels and jamb while retaining original parts."""
import copy,hashlib,json,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement';B=W/'restart2';OUT=B/'restart43-approved-gatehouse-export-v1';assert not OUT.exists();sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
RECEIPTS={
'next-tree02-channels-mounds-v1/user-approval.json':'fbe71bc79abb873cc1c6937b11229fe265040824cdb6fa8f6521f89ac6900dd4',
'next-six-textures-channels-v1/user-approval.json':'7f6abc55fa21eee6ebd3b10c4f079e91e7f8159127a30a23fb9a6955d52e3f9d',
'next-climbing-shed-v2/user-approval.json':'830263fe013c8a0157c158290e529e91188bf8bad6f984de090c53569795d088'}
for p,digest in RECEIPTS.items():assert sha(ROOT/'level-editor/work/croisement02-refinement/restart3-review-batches'/p)==digest
source=B/'restart42-channel-assembly-v1/model.blend';assert sha(source)=='9e3d3c48c7ce207b2e6c3dcfd9066d4c54fa5d76de729d310a02f19d9451c6aa'
aid='york-castle-west-gatehouse';live=ROOT/'level-editor/library/3d-assets/york'/aid;descriptor=json.loads((live/'asset.json').read_text());live_pins={str(p):sha(p)for p in [live/'asset.json',live/'model.glb']}
prior=json.loads((B/'approved-shed-jamb-motion-integration-v1/integration.json').read_text());assert all(live_pins[p]==digest for p,digest in prior['protected_live_inputs'].items()if p in live_pins)
assert shutil.disk_usage(ROOT).free>10*1024**3;assert int(next(l.split()[1]for l in Path('/proc/meminfo').read_text().splitlines()if l.startswith('MemAvailable:')))*1024>6*1024**3
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire
acquire(slots=2)
import bpy
from export_editor import export_asset_library
from refinement_workspace import _geometry
bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene;scene.name='york Refinement';scene.render.threads_mode='FIXED';scene.render.threads=2
jamb=bpy.data.objects['building-778-portcullis-jamb-return'];objects=[o for o in scene.objects if o.type=='MESH'and o.get('source_node')in {f'building-{n}'for n in [763,764,765,774,776,777,778,779]}];assert len(objects)==9 and jamb in objects
geometry={o.name:_geometry(o,protect_appearance=True)for o in scene.objects};original_metadata={o.name:{k:copy.deepcopy(o[k])for k in ('asset_group','asset_name','part_name','projection_component')if k in o}for o in objects}
working=bpy.data.collections.new('york Working');scene.collection.children.link(working)
catalog=json.loads((ROOT/'level-editor/refinement/catalogs/york.json').read_text());group=next(g for g in catalog['groups']if g['id']==aid)
for part in group['parts']:
 if part['obstacle']==778:part['components']=['structural-body','portcullis-jamb-return']
for o in objects:
 working.objects.link(o);o['asset_group']=aid;o['asset_name']=group['name'];o['part_name']=o.name
 if o.get('source_node')=='building-778':o['projection_component']='portcullis-jamb-return'if o==jamb else'structural-body'
 else:
  if 'projection_component'in o:del o['projection_component']
external={}
for o in objects:
 for mat in o.data.materials:
  if mat and mat.use_nodes:
   for node in mat.node_tree.nodes:
    if node.type=='TEX_IMAGE'and node.image and not node.image.packed_file:
     p=Path(bpy.path.abspath(node.image.filepath));assert p.is_file();external[str(p)]=sha(p)
report=export_asset_library('york',OUT/'3d-assets',W/'baseline/york.rhp.json',standalone_pivots={aid:descriptor['source_origin_scene']},asset_ids=[aid],catalog=catalog)
for o in objects:
 for k in ('asset_group','asset_name','part_name','projection_component'):
  if k in original_metadata[o.name]:o[k]=original_metadata[o.name][k]
  elif k in o:del o[k]
assert geometry=={o.name:_geometry(o,protect_appearance=True)for o in scene.objects};assert sha(source)=='9e3d3c48c7ce207b2e6c3dcfd9066d4c54fa5d76de729d310a02f19d9451c6aa';assert all(sha(p)==d for p,d in live_pins.items())
p=OUT/'3d-assets'/aid/'asset.json';d=json.loads(p.read_text());assert d['source_origin_scene']==descriptor['source_origin_scene'];d['gameplay']=copy.deepcopy(descriptor['gameplay']);p.write_text(json.dumps(d,separators=(',',':'))+'\n');index=OUT/'3d-assets/index.json';idx=json.loads(index.read_text());entry=next(r for r in idx['assets']if r['id']==aid);entry['editor']=d;entry['descriptor_sha256']=sha(p);index.write_text(json.dumps(idx,separators=(',',':'))+'\n');(OUT/'private-catalog.json').write_text(json.dumps(catalog,indent=2)+'\n')
result=dict(status='PRIVATE_EXPORT_SURFACE_VERIFICATION_REQUIRED',asset_id=aid,approved_model_sha256=sha(source),source_model=str(source),receipts=RECEIPTS,source_origin_scene=descriptor['source_origin_scene'],model_sha256=sha(p.with_name('model.glb')),descriptor_sha256=sha(p),source_geometry_uv_materials_exact=True,gameplay_exact=True,live_pins=live_pins,external_image_pins=external,exported_objects=[o.name for o in objects],excluded_context=['scenery-york-castle-portcullis'],scope='Static gatehouse only: eight original parts with approved778/779channel corrections and separately approved jamb addition. No moving gate/runtime integration.',export_report=report,canonical_writes=False)
(OUT/'export-report.json').write_text(json.dumps(result,indent=2)+'\n');print(OUT)
