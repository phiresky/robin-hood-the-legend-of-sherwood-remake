"""Export only the exact approved Tree01 wood, preserving excluded crown context."""
import copy,json,sys,shutil
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_slots import acquire,release
from texture_staging import validate_texture_handoff,verify_baked_geometry
from export_editor import export_asset_library
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2';asset='croisement01-tree-00';case=R/'approved-tree01-isolated-wood-fill-v1'/asset
assert shutil.disk_usage(R).free >= 10*1024**3+32*1024**2, 'Disk floor plus output reserve'
acquire()
try:
 bpy.context.scene.render.threads_mode='FIXED';bpy.context.scene.render.threads=2
 h=validate_texture_handoff(case/'review-manifest.json',asset,case/'texture-handoff-32-card-v1/decisions.json',case/'decisions.json');proof=verify_baked_geometry(h)
 approved=case/'baked-v1-luminance/worker.blend';before=sha(approved)
 grouping=R/'tree01-v2/assets'/asset/'reference/grouping.json';config=json.loads((R/'tree01-soil-joint-v10/assets'/asset/'workspace.json').read_text());assert sha(grouping)==config['grouping_manifest_sha256'];original=json.loads(grouping.read_text());catalog=copy.deepcopy(original)
 group=next(g for g in catalog['groups'] if g['id']==asset);excluded=[p for p in group['parts'] if p.get('obstacle')!=29];assert len(excluded)==1 and excluded[0]['node']=='foliage-tree01-inferred-crown';group['parts']=[p for p in group['parts'] if p.get('obstacle')==29];assert len(group['parts'])==1
 catalog['canonical_owners'].pop('foliage-tree01-inferred-crown')
 crown=next(o for o in bpy.data.objects if o.type=='MESH' and o.get('source_node')=='foliage-tree01-inferred-crown');crown.hide_render=True
 out=R/'tree01-wood-integration-v1';out.mkdir(exist_ok=False);report=export_asset_library('Croisement01',out/'assets',ROOT/'level-editor/work/croisement01-refinement/baseline/Croisement01.rhp.json',asset_ids=[asset],catalog=catalog)
 descriptor=json.loads((out/'assets'/asset/'asset.json').read_text());assert [p['node'] for p in descriptor['parts']]==['building-029'];assert sha(approved)==before
 assert sum(p.stat().st_size for p in out.rglob('*') if p.is_file()) <= 32*1024**2, 'Private export output cap'
 (out/'export-proof.json').write_text(json.dumps(dict(scope='WOOD ONLY: native029; excluded gray crown remains untouched in saved worker and is not published',geometry=proof,export=report,approved_user_decision_sha256=sha(case/'user-texture-decision-32-card-v1.json'),original_catalog_sha256=sha(grouping),excluded_context_parts=excluded,approved_worker_unchanged_sha256=before),indent=2)+'\n');print(out)
finally:release()
