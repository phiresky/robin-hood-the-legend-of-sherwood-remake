"""Ground new western leaf complements in small isolated revision workspaces."""
import json,sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha,write_json
from refinement_inventory import inventory,validate_catalog
from refinement_workspace import prepare,modified
from audit_candidates import audit
from render_tree import render_workspace
from render_slots import acquire,release
from opacity_bounds import measure
from tree_geometry import RAY,SIN,COS

def main(index):
    asset=f'croisement02-shrub-{index}';previous=OUT/'understory-round-6/assets'/asset;worker=OUT/'understory-round-9/assets'/asset;folder=OUT/f'understory-candidates/west-support-v2/shrub-{index}'
    if worker.exists() or folder.exists():raise FileExistsError(worker)
    folder.mkdir(parents=True);oldhash=sha(previous/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(previous/'model.blend'));bpy.context.preferences.filepaths.save_version=0
    collection=bpy.data.collections['Croisement02 Working'];objects=[o for o in collection.all_objects if o.type=='MESH' and o.get('asset_group')==asset]
    if len(objects)!=1:raise ValueError('Expected one western foliage source')
    obj=objects[0];bounds=measure(obj);delta=RAY*((.5-bounds['bounds_min'][2])/SIN)
    for v in obj.data.vertices:v.co+=delta
    obj.data.update();after=measure(obj)
    if abs(after['bounds_min'][2]-.5)>.002 or abs(-delta.y*SIN-delta.z*COS)>1e-5:raise ValueError('Visible support or projection drift')
    catalog=json.loads((previous/'reference/grouping.json').read_text());group=next(g for g in catalog['groups'] if g['id']==asset);nodes={p['node'] for p in group['parts']}
    scoped=dict(catalog,groups=[group],canonical_owners={n:asset for n in nodes});write_json(folder/'catalog.json',scoped)
    manifest=json.loads((previous/'source-masks.json').read_text())
    for projection in manifest['projections'].values():
        projection['assignments']=[a for a in projection['assignments'] if a.get('source_node') in nodes or a.get('asset_group')==asset];projection['occluder_constraints']=[]
    write_json(folder/'source-masks.json',manifest)
    for old in list(bpy.data.objects):
        if old.type=='MESH' and old!=obj:bpy.data.objects.remove(old,do_unlink=True)
    bpy.data.orphans_purge(do_recursive=True)
    inventory(folder/'inventory',collection_name='Croisement02 Working',map_name='Croisement02',source_path=previous/'reference/source.png');validate_catalog(folder/'inventory/inventory.json',folder/'catalog.json')
    write_json(folder/'grouping-review.json',dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(folder/'catalog.json'),inventory_sha256=sha(folder/'inventory/inventory.json'),evidence='Exact registered-proposal source node and authored leaf domain, isolated for placement review. Foreign objects remain untouched in prior worker.'))
    prepare(worker,asset_id=asset,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',source_path=previous/'reference/source.png',grouping_manifest=folder/'catalog.json',inventory_path=folder/'inventory/inventory.json',review_path=folder/'grouping-review.json',source_mask_manifest=folder/'source-masks.json',width=384,height=384,framing_padding=1.25)
    modified(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'),compress=True);inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
    report=json.loads((previous/'inspection/refinement.json').read_text());report.update(model_sha256=sha(worker/'model.blend'),status='Opaque lower fringe grounded; fresh joint and visual review pending');report['crown']['opacity_bounds']=after;report['crown']['minimum_z']=min(v.co.z for v in obj.data.vertices)
    write_json(inspection/'refinement.json',report);write_json(inspection/'support-evidence.json',dict(previous_worker=str(previous),previous_model_sha256=oldhash,model_sha256=sha(worker/'model.blend'),world_delta=list(delta),previous_opacity_bounds=bounds,current_opacity_bounds=after,source_pixel_displacement=[float(delta.x),float(-delta.y*SIN-delta.z*COS)],reason='Joint low-angle views showed a floating lower fringe; actual opaque support moved toZ0.5 without changing source pixels.'))
    audit(worker);render_workspace(worker,384,release_slot=False)
    if sha(previous/'model.blend')!=oldhash:raise ValueError('Previous candidate changed')
    print('SUPPORTED',index,sha(worker/'model.blend'),flush=True)
if __name__=='__main__':
    for index in (57,60):
        acquire()
        try:main(index)
        finally:release()
