"""Stage an unapproved western shrub microfragment revision without changing round4."""
import json
import shutil
import sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from shrub_geometry import build
from refinement_workspace import modified
from audit_candidates import audit
from render_tree import render_workspace
ASSET='croisement02-west-shrub-bank'


def main():
    source=OUT/'understory-round-4/assets'/ASSET;worker=OUT/'understory-round-5/assets'/ASSET
    if any(r['asset_id']==ASSET and r['decision']=='approved' for r in json.loads((OUT/'user-feedback.json').read_text())['records']):
        raise ValueError('Approved bank geometry requires a separately authorized revision')
    if worker.exists():raise ValueError('Existing revision must be reviewed, not overwritten')
    previous=OUT/'understory-candidates/west-bank-v4';directory=OUT/'understory-candidates/west-bank-v5';directory.mkdir(exist_ok=False)
    shutil.copytree(source,worker)
    # A copied previous receipt must never select or approve a fresh revision.
    for name in ['shrub-candidate.json','visual-review.json','joint-neighbourhood.json']:
        target=worker/'inspection'/name
        if target.exists():target.rename(target.with_name('previous-'+name))
    shutil.copy2(previous/'complete-source.png',directory/'complete-source.png')
    packet=json.loads((previous/'partition.json').read_text());packet['directory']=str(directory);write_json(directory/'partition.json',packet)
    packets=[]
    for label in ['west','east']:
        shutil.copytree(previous/label,directory/label)
        packet=json.loads((directory/label/'partition.json').read_text());packet.update(directory=str(directory/label),irregular_source_fragments=True)
        write_json(directory/label/'partition.json',packet);packets.append(packet)
    bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));bpy.context.preferences.filepaths.save_version=0
    collection=bpy.data.collections['Croisement02 Working'];reports=[]
    for label,packet in zip(['west','east'],packets):
        obj=next(o for o in collection.all_objects if o.type=='MESH' and o.get('asset_group')==ASSET and o.name.endswith(label))
        reports.append(build(obj,packet))
    modified(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'))
    report=json.loads((worker/'inspection/refinement.json').read_text());report.update(crown=dict(geometry_version='native-shrub-leaf-volume-v2',lobes=reports),source_packet=str(directory/'partition.json'),model_sha256=sha(worker/'model.blend'),status='isolated microfragment revision; renewed self/joint review pending')
    report['limitations']=[n for n in report['limitations'] if 'curved source patches' not in n]
    report['limitations'].append('Observed twigs remain small source-facing fragments; off-map twig appearance and hidden foliage are inferred.')
    write_json(worker/'inspection/refinement.json',report)
    audit(worker);render_workspace(worker,384,release_slot=False)
    write_json(directory/'revision.json',dict(previous_worker=str(source),previous_model_sha256=sha(source/'model.blend'),worker=str(worker),model_sha256=sha(worker/'model.blend'),status='isolated unapproved microfragment revision; source domain413 unchanged'))
    print('WEST BANK MICROFRAGMENT REVISION',worker,flush=True)

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
