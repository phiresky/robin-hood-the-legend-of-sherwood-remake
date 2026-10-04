"""Resume saved private foliage candidates after an interrupted evidence stage."""
import argparse,json,sys,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha,write_json
from refinement_workspace import prepare,modified
from render_slots import acquire,release
from audit_candidates import audit
from render_tree import render_workspace
from opacity_bounds import measure

def finish(index):
    if index==54:
        asset='croisement02-northwest-boundary-shrub-54';destination=OUT/'understory-candidates/northwest54-v1';worker=OUT/'understory-round-7/assets'/asset;packet=destination/'partition.json';domain=417
    else:
        asset=f'croisement02-shrub-{index}';destination=OUT/'understory-candidates/west-complements-v1';worker=OUT/'understory-round-6/assets'/asset;packet=destination/f'shrub-{index}/partition.json';domain={57:481,60:482}[index]
    if any(r['asset_id']==asset and r['decision']=='approved' for r in json.loads((OUT/'user-feedback.json').read_text())['records']):raise ValueError('Cannot resume approved asset')
    bpy.context.preferences.filepaths.save_version=0
    if (worker/'model.blend').exists():bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
    else:
        if (destination/'input.blend').stat().st_size>256*1024**2:raise ValueError('Source scene exceeds bounded worker budget')
        if shutil.disk_usage(OUT).free<1024**3:raise ValueError('Insufficient disk reserve for bounded worker')
        bpy.ops.wm.open_mainfile(filepath=str(destination/'input.blend'))
        prepare(worker,asset_id=asset,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',source_path=OUT/'animation-references/composite-frame-0.png',grouping_manifest=destination/'catalog.json',inventory_path=destination/'inventory/inventory.json',review_path=destination/'grouping-review.json',source_mask_manifest=destination/'source-masks.json',width=384,height=384,framing_padding=1.25)
        modified(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'))
    before=sha(worker/'model.blend');objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==asset]
    if not objects:raise ValueError('No candidate meshes')
    lobes=[dict(geometry_version='native-shrub-leaf-volume-v2',object=o.name,vertices=len(o.data.vertices),faces=len(o.data.polygons),opacity_bounds=measure(o),minimum_z=min((o.matrix_world@v.co).z for v in o.data.vertices),source_projection_preserved=True,inferred_donor_alpha='irregular silhouette and luminance cut',source_fragment_layout='jittered Delaunay triangles') for o in objects]
    inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
    report=dict(asset_id=asset,mask=index,coverage_domain_mask=domain,crown=lobes[0] if len(lobes)==1 else dict(geometry_version='native-shrub-leaf-volume-v2',lobes=lobes),source_packet=str(packet),model_sha256=before,status='Saved candidate evidence resumed; manual self-review and neighbourhood review pending',limitations=['Hidden leaf arrangement and any off-map continuation are inferred.','Observed domains preserve original native RGB; excluded neighbours keep their source ownership.','No user geometry approval, texture approval or final terrain integration claimed.'])
    if index==54:
        previous=OUT/'understory-round-6/assets'/asset;old=json.loads((previous/'inspection/actual-materials/opacity-bounds.json').read_text())['crowns'];changes=[]
        for obj,bounds,row in zip(objects,old,lobes):
            now=row['opacity_bounds'];delta=[now['bounds_min'][j]-bounds['bounds_min'][j] for j in range(3)];changes.append(dict(object=obj.name,world_delta=delta,previous_opacity_bounds=bounds,current_opacity_bounds=now))
        write_json(inspection/'support-evidence.json',dict(previous_worker=str(previous),previous_model_sha256=sha(previous/'model.blend'),model_sha256=before,changes=changes,reason='Source-ray translation brings opaque lower fringe toZ0.5 after reviewed joint showed a ground gap.'))
    write_json(inspection/'refinement.json',report);audit(worker);render_workspace(worker,384,release_slot=False)
    if sha(worker/'model.blend')!=before:raise ValueError('Evidence generation changed saved geometry')
    print('RESUMED',worker,before,flush=True)
if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('indices',type=int,nargs='+',choices=[54,57,60]);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);acquire()
    try:
        for index in args.indices:finish(index)
    finally:release()
