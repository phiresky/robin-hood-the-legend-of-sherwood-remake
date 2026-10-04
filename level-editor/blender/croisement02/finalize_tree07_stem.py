"""Project a private continuous lower stem while preserving the selected crown."""
import json,sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import prepare,modified,validate
from audit_candidates import audit
from render_tree import render_workspace

def main():
    old=tree_workspace(7);prototype=OUT/'tree07-root-research/continuous-stem-v5';worker=OUT/'root-stem-round-2/assets'/old.name
    if worker.exists():raise FileExistsError(worker)
    old_hash=sha(old/'model.blend');prototype_hash=sha(prototype/'model.blend');cfg=json.loads((old/'workspace.json').read_text())
    bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'));bpy.context.preferences.filepaths.save_version=0
    prepare(worker,asset_id=old.name,scene_name=cfg['scene_name'],collection_name=cfg['collection_name'],source_path=old/'reference/source.png',grouping_manifest=old/'reference/grouping.json',inventory_path=old/'reference/inventory.json',review_path=old/'reference/grouping-review.json',source_mask_manifest=old/'source-masks.json',width=384,height=384,framing_padding=cfg['framing_padding'],lighting=cfg['lighting'])
    bpy.ops.wm.open_mainfile(filepath=str(prototype/'model.blend'));bpy.context.preferences.filepaths.save_version=0
    objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==old.name]
    unchanged=[o for o in objects if not(o.get('source_node') in ('building-058','building-062') and o.get('projection_component')!='crown')]
    saved={o.name:o.data.copy() for o in unchanged}
    for mesh in saved.values():
        for i,material in enumerate(mesh.materials):mesh.materials[i]=material.copy()
    bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'));modified(worker)
    for obj in unchanged:obj.data=saved[obj.name]
    validate(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'));(worker/'inspection').mkdir(exist_ok=True)
    report=json.loads((old/'inspection/refinement.json').read_text());report.update(model_sha256=sha(worker/'model.blend'),lower_stem=json.loads((prototype/'evidence.json').read_text()),status='Private continuous lower-stem candidate; local source and contact audits pending')
    write_json(worker/'inspection/refinement.json',report);audit(worker);render_workspace(worker,384,release_slot=False)
    import audit_tree07_roots,inspect_tree07_base
    previous_args=sys.argv
    try:
        sys.argv=[previous_args[0],'--','--worker',str(worker),'--preservation-base',str(old)];audit_tree07_roots.main(release_slot=False)
        sys.argv=[previous_args[0],'--','--model',str(worker/'model.blend'),'--output-name','continuous-stem-v5-projected-review'];inspect_tree07_base.main()
    finally:sys.argv=previous_args
    if sha(old/'model.blend')!=old_hash or sha(prototype/'model.blend')!=prototype_hash:raise ValueError('Reviewed input changed')
    write_json(worker/'inspection/stem-candidate.json',dict(previous_worker=str(old),previous_model_sha256=old_hash,prototype=str(prototype),prototype_sha256=prototype_hash,model_sha256=sha(worker/'model.blend'),status='New geometry candidate; independent self-review required',approval='pending'))
    print(worker)

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
