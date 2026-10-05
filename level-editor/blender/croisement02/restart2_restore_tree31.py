"""Restore exact frozen outside meshes while retaining the newly baked wood data."""
import argparse,json,sys,shutil
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json
from refinement_workspace import validate,_geometry
from audit_candidates import audit
from render_tree import render_workspace
from render_slots import acquire,release


def shape(obj):return ([tuple(v.co) for v in obj.data.vertices],[tuple(f.vertices) for f in obj.data.polygons])

def main():
    parser=argparse.ArgumentParser();parser.add_argument('index',type=int,choices=[31]);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);index=args.index;old=tree_workspace(index)
    prototype=OUT/'restart2-wood/tree31-sdf-v4';worker=OUT/'restart2-wood/projected/assets'/old.name;domain=old;proto=json.loads((prototype/'evidence.json').read_text());base=Path(proto['previous_worker']);prototype_hash=sha(prototype/'model.blend');baked_hash=sha(worker/'model.blend')
    snapshot=prototype/'projected-before-restore.blend'
    if snapshot.exists():
        if sha(snapshot)!=baked_hash:raise ValueError('Earlier bake snapshot differs; preserve it')
    else:shutil.copyfile(worker/'model.blend',snapshot)
    bpy.ops.wm.open_mainfile(filepath=str(prototype/'model.blend'));bpy.context.preferences.filepaths.save_version=0;objects=list(bpy.data.collections['Croisement02 Working'].all_objects);wood=[o for o in objects if o.type=='MESH' and o.get('asset_group')==old.name and o.get('projection_component')!='crown'];outside=[o for o in objects if o.type=='MESH' and o not in wood];before={o.name:_geometry(o,protect_appearance=True) for o in outside};names=[o.name for o in wood]
    with bpy.data.libraries.load(str(snapshot),link=False) as (_,loaded):loaded.objects=names
    for obj,source in zip(wood,loaded.objects):
        if source is None or shape(obj)!=shape(source):raise ValueError('Baked wood geometry changed')
        obj.data=source.data
    for obj in loaded.objects:bpy.data.objects.remove(obj,do_unlink=True)
    if before!={o.name:_geometry(o,protect_appearance=True) for o in outside}:raise ValueError('Frozen outside appearance changed')
    validate(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'));(worker/'inspection').mkdir(exist_ok=True)
    report=json.loads((old/'inspection/refinement.json').read_text());report.update(model_sha256=sha(worker/'model.blend'),boundary_geometry=proto,source_domain_review={'status':'Unchanged native31 ownership'},status='Private bounded boundary candidate; independent actual/source review pending');write_json(worker/'inspection/refinement.json',report)
    write_json(worker/'inspection/boundary-preservation.json',dict(model_sha256=sha(worker/'model.blend'),previous_worker=str(base),previous_model_sha256=proto['previous_model_sha256'],protected_appearances=before,preserved=True,geometry_and_material_outside_wood_unchanged=True,projection_staging_hash=baked_hash,restoration='Exact original scene reopened; imported only new wood UV/material data after checking identical wood vertices/faces.'))
    audit(worker);render_workspace(worker,384,release_slot=False)
    if sha(base/'model.blend')!=proto['previous_model_sha256'] or sha(prototype/'model.blend')!=prototype_hash:raise ValueError('Frozen input changed')
    write_json(worker/'inspection/boundary-candidate.json',dict(model_sha256=sha(worker/'model.blend'),geometry_prototype=str(prototype),geometry_sha256=prototype_hash,source_domain=str(domain),status='Awaiting self and root actual/source review',approval='pending'))
    print(worker)

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
