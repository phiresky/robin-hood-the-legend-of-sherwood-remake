"""Bring visible northwest foliage to the reviewed ground datum without source drift."""
import json,sys
from pathlib import Path
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha,write_json
from refinement_workspace import prepare,modified
from audit_candidates import audit
from render_tree import render_workspace
from render_slots import acquire,release
from opacity_bounds import measure
from tree_geometry import RAY,SIN,COS

def main():
    asset='croisement02-northwest-boundary-shrub-54';previous=OUT/'understory-round-6/assets'/asset;worker=OUT/'understory-round-7/assets'/asset
    if worker.exists():raise FileExistsError(worker)
    if any(r['asset_id']==asset and r['decision']=='approved' for r in json.loads((OUT/'user-feedback.json').read_text())['records']):raise ValueError('Approved shrub is frozen')
    before=sha(previous/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(previous/'model.blend'));bpy.context.preferences.filepaths.save_version=0
    changes=[];report=json.loads((previous/'inspection/refinement.json').read_text())
    objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==asset]
    if len(objects)!=2:raise ValueError('Expected exactly two shrub lobes')
    for obj in objects:
        obj['asset_name']='Northwest Boundary Shrub54'
        bounds=measure(obj);delta=RAY*((.5-bounds['bounds_min'][2])/SIN)
        for v in obj.data.vertices:v.co+=delta
        obj.data.update();after=measure(obj)
        if abs(after['bounds_min'][2]-.5)>.002:raise ValueError('Opaque datum translation failed')
        if abs(-delta.y*SIN-delta.z*COS)>1e-5:raise ValueError('Source projection moved')
        changes.append(dict(object=obj.name,world_delta=list(delta),previous_opacity_bounds=bounds,current_opacity_bounds=after,source_pixel_displacement=[float(delta.x),float(-delta.y*SIN-delta.z*COS)]))
    prepare(worker,asset_id=asset,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',source_path=previous/'reference/source.png',grouping_manifest=previous/'reference/grouping.json',inventory_path=previous/'reference/inventory.json',review_path=previous/'reference/grouping-review.json',source_mask_manifest=previous/'source-masks.json',width=384,height=384,framing_padding=1.25)
    modified(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'))
    report['model_sha256']=sha(worker/'model.blend');report['status']='Opaque foliage grounded by source-ray translation; joint refresh pending'
    for row,change in zip(report['crown']['lobes'],changes):row['opacity_bounds']=change['current_opacity_bounds'];row['support_evidence']=change
    (worker/'inspection').mkdir(exist_ok=True)
    write_json(worker/'inspection/refinement.json',report)
    audit(worker);render_workspace(worker,384,release_slot=False)
    if sha(previous/'model.blend')!=before:raise ValueError('Previous candidate changed')
    write_json(worker/'inspection/support-evidence.json',dict(previous_worker=str(previous),previous_model_sha256=before,model_sha256=sha(worker/'model.blend'),changes=changes,reason='Northwest joint contact view showed visible foliage floating above diagnostic ground; opaque fringe now meetsZ0.5. Actual terrain integration remains separate.'))
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
