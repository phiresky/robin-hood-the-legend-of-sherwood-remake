"""Bake approved packets in isolated scenes, retaining each guarded handoff.

Blender CLI: --python bake_approved_packets.py -- jobs.json
Each job provides manifest, generated_image, output and optionally source_blend.
Existing outputs are refused by the single-asset staging implementation.
"""
import json
from pathlib import Path
import sys

sys.path.insert(0,str(Path(__file__).resolve().parent))
import bpy
from bake_reviewed_asset import stage


def run(jobs_path):
    jobs=json.loads(Path(jobs_path).read_text())
    reports=[]
    for job in jobs:
        manifest_path=Path(job['manifest']).resolve(strict=True)
        manifest=json.loads(manifest_path.read_text())
        blend=Path(job.get('source_blend',manifest['source_blend'])).resolve(strict=True)
        print('APPROVED PACKET START '+manifest['asset_id'],flush=True)
        bpy.ops.wm.open_mainfile(filepath=str(blend))
        aliases={}
        if job.get('review_asset_alias'):
            for name in manifest['object_names']:
                obj=bpy.data.objects[name]
                aliases[name]=obj['asset_group']
                obj['asset_group']=manifest['asset_id']
        report=stage(manifest_path,job['generated_image'],job['output'],
                     texels_per_unit=job.get('texels_per_unit',2))
        if aliases:
            for name,owner in aliases.items():
                bpy.data.objects[name]['asset_group']=owner
            bpy.ops.wm.save_as_mainfile(filepath=str(Path(job['output'])/'worker.blend'))
        reports.append({'asset_id':report['asset_id'],'output':str(Path(job['output']).resolve()),
                        'geometry_verified':report['geometry_verified'], 'counts':report['counts']})
        print('APPROVED PACKET COMPLETE '+json.dumps(reports[-1]),flush=True)
    return reports


if __name__=='__main__':
    print(json.dumps(run(sys.argv[sys.argv.index('--')+1])),flush=True)
