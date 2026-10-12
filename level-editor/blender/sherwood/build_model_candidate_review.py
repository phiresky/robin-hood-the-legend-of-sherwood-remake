"""Publish inspected Sherwood model revisions without reopening grouping decisions."""
import argparse
import json
from pathlib import Path
import sys

HERE=Path(__file__).resolve().parent
EDITOR=HERE.parents[1]
sys.path[:0]=[str(HERE),str(EDITOR/'refinement/blender')]
from source_authority import sha
from build_review_gallery import build


def main(root):
    root=Path(root).resolve();folder=root/'packet'
    stage=json.loads((root/'stage.json').read_text());packet=json.loads((folder/'packet.json').read_text())
    assert sha(root/'candidate.blend')==stage['worker_sha256']==packet['worker_sha256']
    for name,expected in packet['files'].items():assert sha(folder/name)==expected
    assert (packet['views'][0]['azimuth_degrees'],packet['views'][0]['elevation_degrees'])==(0,35)
    inspection_path=root/'inspection.json'
    inspection=json.loads(inspection_path.read_text()) if inspection_path.exists() else {}
    checked=(inspection.get('packet_sha256')==sha(folder/'packet.json') and inspection.get('all_views_inspected') is True)
    verification=json.loads((root/'saved-worker-verification.json').read_text())
    checked=checked and verification.get('status')=='PASS' and verification.get('worker_sha256')==stage['worker_sha256']
    item=dict(id=packet['asset_id'],name='Northeast woodland oak — revised trunk and forks',
        status='ready-for-user' if checked else 'in-progress',technical_eligible=checked,user_approval='pending',
        solid=str(folder/'solid.png'),solid_label='Revised geometry — original camera first, then west, east and back',
        textured=str(folder/'textured.png'),textured_label='Source projection preview — hidden textures are still unfinished',
        context=str(folder/'original.png'),validation=str(root/'saved-worker-verification.json'),
        review=str(root/'stage.json'),ownership=str(folder/'packet.json'),
        artwork_references=[dict(id='original-day',label='Original Day artwork without animated foliage',
            path=str(folder/'original-day.png'),sha256=sha(folder/'original-day.png'))],
        notes=['Replaces the flat-topped trunk and thin central support with connected branching volume traced from native mask 11.',
               'Rounded cross-sections and rear depth are inferred; the original-camera silhouette follows the artwork.',
               'Approved grouping and the existing foliage are unchanged. This card reviews the revised wooden structure.',
               'The new bark projection uses reviewed native-mask ownership. This is not the final synthesized texture review.'])
    manifest=root/'gallery-manifest.json';manifest.write_text(json.dumps(dict(map='Sherwood models',review_kind='model',items=[item]),indent=2)+'\n')
    gallery=EDITOR/'work/needs-review/sherwood-refinement/model-review/gallery/revisions'
    build(manifest,gallery,map_name='Sherwood models',pending_only=True)
    evidence=json.loads((gallery/'evidence.json').read_text())
    decisions=json.loads((HERE/'model-decisions.json').read_text())['decisions']
    revision=evidence['items'][0]['review_revision']
    if any(d['id']==item['id'] and d['review_revision']==revision and d['decision']=='approved' for d in decisions):
        item['user_approval']='approved'
        manifest.write_text(json.dumps(dict(map='Sherwood models',review_kind='model',items=[item]),indent=2)+'\n')
        build(manifest,gallery,map_name='Sherwood models',pending_only=True)
    parent=gallery.parent/'index.html'
    if parent.exists():
        text=parent.read_text();notice='<p><a href="revisions/">Northeast woodland oak revision</a>: '+('approved' if item['user_approval']=='approved' else 'awaiting review')+'.</p>'
        if 'href="revisions/"' not in text:parent.write_text(text.replace('<nav>',notice+'<nav>',1))
    print(json.dumps(dict(gallery=str(gallery/'index.html'),ready=checked,asset=packet['asset_id'])))


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--root',required=True);a=p.parse_args();main(a.root)
