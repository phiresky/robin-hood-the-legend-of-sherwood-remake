"""Place an inferred elevated fringe behind adjacent crowns without source drift."""
import json,sys
from pathlib import Path
import bpy
from mathutils import Vector
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
    asset='croisement02-canopy-fringe-22';previous=OUT/'understory-round-23/assets'/asset;worker=OUT/'understory-round-25/assets'/asset
    if worker.exists():raise FileExistsError(worker)
    proof=OUT/'understory-candidates/north-fringe22-visibility-v3/evidence.json';visibility=json.loads(proof.read_text())
    if next(r for r in visibility['results'] if r['ray_offset']==-80)['visible_owned_pixels']!=387:raise ValueError('Observed fringe visibility not proven')
    before=sha(previous/'model.blend')
    if visibility['workers'][0]['model_sha256']!=before:raise ValueError('Visibility candidate changed')
    bpy.ops.wm.open_mainfile(filepath=str(previous/'model.blend'));bpy.context.preferences.filepaths.save_version=0
    objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==asset]
    if len(objects)!=1:raise ValueError('Expected one elevated fringe')
    changes=[];delta=RAY*-80
    for obj in objects:
        old=measure(obj);local=obj.matrix_world.inverted().to_3x3()@delta
        for vertex in obj.data.vertices:vertex.co+=local
        obj.data.update();new=measure(obj)
        if abs(-delta.y*SIN-delta.z*COS)>1e-5:raise ValueError('Source projection moved')
        changes.append(dict(object=obj.name,world_delta=list(delta),previous_opacity_bounds=old,current_opacity_bounds=new,source_pixel_displacement=[float(delta.x),float(-delta.y*SIN-delta.z*COS)]))
    prepare(worker,asset_id=asset,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',source_path=previous/'reference/source.png',grouping_manifest=previous/'reference/grouping.json',inventory_path=previous/'reference/inventory.json',review_path=previous/'reference/grouping-review.json',source_mask_manifest=previous/'source-masks.json',width=384,height=384,framing_padding=1.25)
    modified(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'));mh=sha(worker/'model.blend');inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
    report=json.loads((previous/'inspection/refinement.json').read_text());report.update(model_sha256=mh,status='Elevated source-ray placement revision; exact joint review pending',inferred_crown_center_z=250+delta.z)
    report['crown'].update(opacity_bounds=changes[0]['current_opacity_bounds'],minimum_z=min((objects[0].matrix_world@v.co).z for v in objects[0].data.vertices),inferred_crown_center_z=250+delta.z,support_evidence=str(inspection/'support-evidence.json'))
    write_json(inspection/'refinement.json',report)
    write_json(inspection/'support-evidence.json',dict(model_sha256=mh,previous_worker=str(previous),previous_model_sha256=before,visibility_evidence=str(proof),visibility_evidence_sha256=sha(proof),changes=changes,reason='Native22 shallow occlusion threshold indicates a background fringe. Small source-ray retreat preserves all387 observed pixels in alpha-aware neighbour probe. Elevated crown association and unknown continuation remain hypotheses; no trunk or ground anchor is asserted.'))
    fill=previous/'inspection/inferred-fill-evidence.json'
    write_json(inspection/'inferred-fill-evidence.json',dict(model_sha256=mh,previous_evidence=str(fill),previous_evidence_sha256=sha(fill),materials_uv_alpha_unchanged=True,geometry_change='Rigid source-ray translation only'))
    audit(worker);render_workspace(worker,384,release_slot=False,transparent_bounces=256)
    if sha(previous/'model.blend')!=before:raise ValueError('Prior worker changed')
    print(worker,flush=True)
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
