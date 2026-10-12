"""Export approved Tree08 wood privately without inventing gameplay ownership."""
import hashlib,json,shutil,sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from export_editor import export_asset_library
from review_evidence import sha
R=ROOT/'level-editor/work/active/croisement01/restart2'
asset='croisement01-tree-08'
receipt=ROOT/'level-editor/work/active/croisement02/restart3-review-batches/six-scoped-reviews-oct11-v1/user-approval.json'
assert sha(receipt)=='d435a76b08eb3ac616f1ae2ffcdf19a7875969e515dd01f65de1871ca886ef1d'
approval=json.loads(receipt.read_text());assert approval['status']=='APPROVED'
decision=next(d for d in approval['decisions'] if d['asset_id']==asset)
assert decision['decision']=='approved' and decision['scope']=='texture'
model=Path(decision['model']);assert sha(model)==decision['model_sha256']=='01dedc44df9d06962a76b6bbf6d4fc96715f219ee0af7eb782ba22677892767a'
geometry_receipt=ROOT/'level-editor/work/active/croisement02/restart3-review-batches/three-tree-geometry-oct11-v1/user-approval.json'
assert sha(geometry_receipt)=='a30e6aa6559e71347d7e1ee4b37a51defa2d4d880504ba7eeb334feafcef7d1e'
def geometry(o):
 return hashlib.sha256(json.dumps(dict(matrix=[list(r) for r in o.matrix_world],points=[list(v.co) for v in o.data.vertices],faces=[list(f.vertices) for f in o.data.polygons]),separators=(',',':')).encode()).hexdigest()
acquire(slots=2)
try:
 assert shutil.disk_usage(R).free>=10*1024**3
 assert next(int(s.split()[1]) for s in Path('/proc/meminfo').read_text().splitlines() if s.startswith('MemAvailable:'))>=6*1024**2
 bpy.ops.wm.open_mainfile(filepath=str(model));objects=[o for o in bpy.context.scene.objects if o.type=='MESH'];assert len(objects)==1
 obj=objects[0];assert obj.get('asset_group')==asset and obj.get('source_node')=='scenery-tree08-wood-prototype'
 before=geometry(obj);node=obj['source_node']
 working=bpy.data.collections.get('Croisement01 Working')
 if working is None:working=bpy.data.collections.new('Croisement01 Working');bpy.context.scene.collection.children.link(working)
 if obj.name not in working.objects:working.objects.link(obj)
 obj.hide_render=False;obj['asset_name']='Tree08 branching wood';obj['part_name']='Approved trunk, branches and roots'
 catalog=dict(version=1,map='Croisement01',groups=[dict(id=asset,name='Tree08 branching wood',parts=[dict(node=node,name='Approved trunk, branches and roots')])],canonical_owners={node:asset},review_notes=['Wood appearance and shape approved; canopy and native obstacle association excluded.'])
 out=R/'tree08-approved-wood-export-v2';out.mkdir()
 (out/'grouping.json').write_text(json.dumps(catalog,indent=2)+'\n')
 report=export_asset_library('Croisement01',out/'assets',ROOT/'level-editor/work/active/croisement01/baseline/Croisement01.rhp.json',asset_ids=[asset],catalog=catalog)
 descriptor=json.loads((out/'assets'/asset/'asset.json').read_text());assert [p['node'] for p in descriptor['parts']]==[node]
 assert all('source_obstacle' not in p and 'obstacle_local_game' not in p for p in descriptor['parts'])
 assert geometry(obj)==before and sha(model)==decision['model_sha256']
 files={str(p.relative_to(out)):sha(p) for p in out.rglob('*') if p.is_file()}
 (out/'export-proof.json').write_text(json.dumps(dict(status='EXPORTED_REQUIRES_IMAGE_AND_RUNTIME_REVIEW',scope='Approved decorative wood only; no native obstacle or canopy reassignment, no canonical writes',model_sha256=sha(model),approval_sha256=sha(receipt),geometry_approval_sha256=sha(geometry_receipt),geometry_unchanged=before,export=report,files=files),indent=2)+'\n')
 print(out)
finally:release()
