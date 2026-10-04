"""Compress a newly created private review scene and rebind its own evidence."""
import argparse
import json
import sys
from pathlib import Path
import bpy
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from approved_texture_stage import geometry,appearance
from evidence_io import sha,write_json


def fingerprints():
    return {o.name:dict(geometry=geometry(o),appearance=appearance(o),hide_render=o.hide_render) for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH'}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('stage',type=Path);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);stage=args.stage.resolve();model=stage/'scene.blend'
    if stage.name!='whole-scene-93-review':raise ValueError('Only newly created private93 stage is authorized')
    if not (stage/'contacts/evidence.json').exists() or not (stage/'first-hit/audit.json').exists():raise ValueError('Finish active evidence readers before compressing')
    report=json.loads((stage/'assembly.json').read_text());before_hash=sha(model);before_bytes=model.stat().st_size
    if report['model_sha256']!=before_hash:raise ValueError('Private scene changed')
    bpy.ops.wm.open_mainfile(filepath=str(model));before=fingerprints();bpy.context.preferences.filepaths.save_version=0
    bpy.ops.wm.save_as_mainfile(filepath=str(model),compress=True)
    bpy.ops.wm.open_mainfile(filepath=str(model));after=fingerprints()
    if before!=after:raise ValueError('Compression changed geometry or appearance')
    after_hash=sha(model)
    def rebound(value):
        if isinstance(value,dict):return {k:rebound(v) for k,v in value.items()}
        if isinstance(value,list):return [rebound(v) for v in value]
        return after_hash if value==before_hash else value
    updated=[]
    for path in sorted(stage.rglob('*.json')):
        record=json.loads(path.read_text());new=rebound(record)
        if record!=new:write_json(path,new);updated.append(str(path.relative_to(stage)))
    write_json(stage/'compression.json',dict(status='PASS',before_sha256=before_hash,after_sha256=after_hash,before_bytes=before_bytes,after_bytes=model.stat().st_size,mesh_objects_preserved=len(before),geometry_appearance_and_visibility_unchanged=True,updated_evidence=updated,snapshot_files_changed=False))
    print(before_bytes,'to',model.stat().st_size,'bytes;',len(before),'mesh fingerprints unchanged')


if __name__=='__main__':main()
