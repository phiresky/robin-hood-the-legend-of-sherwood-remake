"""Ground the reviewed western shrub bank in a new isolated revision workspace."""
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

def main():
    asset='croisement02-west-shrub-bank';previous=OUT/'understory-round-5/assets'/asset;worker=OUT/'understory-round-6/assets'/asset;folder=OUT/'understory-candidates/west-bank-v6'
    if worker.exists() or folder.exists():raise FileExistsError(worker)
    folder.mkdir(parents=True);oldhash=sha(previous/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(previous/'model.blend'));bpy.context.preferences.filepaths.save_version=0
    collection=bpy.data.collections['Croisement02 Working'];objects=[o for o in collection.all_objects if o.type=='MESH' and o.get('asset_group')==asset]
    if len(objects)!=2:raise ValueError('Expected two western bank foliage lobes')
    changes=[]
    for obj in objects:
        bounds=measure(obj);delta=RAY*((.5-bounds['bounds_min'][2])/SIN)
        for v in obj.data.vertices:v.co+=delta
        obj.data.update();after=measure(obj)
        if abs(after['bounds_min'][2]-.5)>.002 or abs(-delta.y*SIN-delta.z*COS)>1e-5:raise ValueError('Visible support or projection drift')
        changes.append(dict(object=obj.name,world_delta=list(delta),previous_opacity_bounds=bounds,current_opacity_bounds=after,source_pixel_displacement=[float(delta.x),float(-delta.y*SIN-delta.z*COS)]))
    catalog=json.loads((previous/'reference/grouping.json').read_text());group=next(g for g in catalog['groups'] if g['id']==asset);nodes={p['node'] for p in group['parts']}
    scoped=dict(catalog,groups=[group],canonical_owners={n:asset for n in nodes});write_json(folder/'catalog.json',scoped)
    manifest=json.loads((previous/'source-masks.json').read_text())
    for projection in manifest['projections'].values():
        projection['assignments']=[a for a in projection['assignments'] if a.get('source_node') in nodes or a.get('asset_group')==asset];projection['occluder_constraints']=[]
    write_json(folder/'source-masks.json',manifest)
    for old in list(bpy.data.objects):
        if old.type=='MESH' and old not in objects:bpy.data.objects.remove(old,do_unlink=True)
    bpy.data.orphans_purge(do_recursive=True)
    inventory(folder/'inventory',collection_name='Croisement02 Working',map_name='Croisement02',source_path=previous/'reference/source.png');validate_catalog(folder/'inventory/inventory.json',folder/'catalog.json')
    write_json(folder/'grouping-review.json',dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(folder/'catalog.json'),inventory_sha256=sha(folder/'inventory/inventory.json'),evidence='Exact registered-proposal source node and authored leaf domain, isolated for placement review. Foreign objects remain untouched in prior worker.'))
    prepare(worker,asset_id=asset,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',source_path=previous/'reference/source.png',grouping_manifest=folder/'catalog.json',inventory_path=folder/'inventory/inventory.json',review_path=folder/'grouping-review.json',source_mask_manifest=folder/'source-masks.json',width=384,height=384,framing_padding=1.25)
    modified(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'),compress=True);inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
    report=json.loads((previous/'inspection/refinement.json').read_text());report.update(model_sha256=sha(worker/'model.blend'),status='Opaque bank lower fringes grounded; fresh joint and visual review pending')
    for obj,row,change in zip(objects,report['crown']['lobes'],changes):
        row['opacity_bounds']=change['current_opacity_bounds'];row['minimum_z']=min(v.co.z for v in obj.data.vertices)
    write_json(inspection/'refinement.json',report);write_json(inspection/'support-evidence.json',dict(previous_worker=str(previous),previous_model_sha256=oldhash,model_sha256=sha(worker/'model.blend'),changes=changes,reason='New west joint exposed floating opaque lower fringes. Source-ray translation toZ0.5 preserves source projection; older selected bank remains immutable.'))
    write_json(folder/'revision.json',dict(previous_worker=str(previous),previous_model_sha256=oldhash,model_sha256=sha(worker/'model.blend'),reason='Source-preserving visible ground-support correction; new geometry approval required.'))
    audit(worker);render_workspace(worker,384,release_slot=False)
    if sha(previous/'model.blend')!=oldhash:raise ValueError('Previous candidate changed')
    print('SUPPORTED BANK',sha(worker/'model.blend'),flush=True)
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
