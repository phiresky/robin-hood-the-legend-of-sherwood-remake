"""Record explicit pasted gallery feedback as grouping-only decisions.

Usage: python3 record_grouping_feedback.py path/to/user-feedback.txt
No grouping approval authorizes geometry completion, texture synthesis or publication.
"""
import argparse
import json
import re
from pathlib import Path

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/active/york'


def preview_only_update(before, after):
    """Carry grouping approval across a solid-preview fix, never an asset edit."""
    for key in ('id','name','approval_scope','technical_eligible','notes'):
        if before.get(key) != after.get(key):return False
    if before.get('approval_scope') != 'grouping-only':return False
    # Ownership binds the model, descriptor, geometry and source assignments.
    for key in ('ownership','validation'):
        a=before.get('reports',{}).get(key,{}).get('sha256')
        b=after.get('reports',{}).get(key,{}).get('sha256')
        if not a or a != b:return False
    def source_images(item):
        return {k:v['sha256'] for k,v in item.get('images',{}).items()
                if k not in ('solid','east_solid')}
    sources=source_images(before)
    return bool(sources) and sources == source_images(after)


def record(text, gallery, destination):
    gallery,destination=Path(gallery),Path(destination)
    evidence=json.loads((gallery/'evidence.json').read_text())
    current={r['id']:r for r in evidence['items']}
    known={}
    for folder in [gallery,*sorted((gallery/'history').glob('*'))]:
        path=folder/'evidence.json'
        if not path.is_file():continue
        for item in json.loads(path.read_text())['items']:
            known[(item['id'],item['review_revision'][:16])]=(item,folder)
    additions=[]
    for line in text.splitlines():
        if not line.strip() or line.strip()=='York grouping review':continue
        match=re.fullmatch(r'(york-[a-z0-9-]+): (approved|needs refinement|feedback)(?: — (.*))? \[review ([0-9a-f]{16})\]',line)
        if not match:raise ValueError('Unrecognized feedback line: '+line)
        identity,decision,note,revision=match.groups()
        if (identity,revision) not in known:raise ValueError('Unknown asset revision: '+identity)
        item,folder=known[(identity,revision)]
        if item.get('approval_scope')!='grouping-only':raise ValueError('Not grouping evidence: '+identity)
        if decision=='approved' and (identity not in current or current[identity]['review_revision']!=item['review_revision']):
            raise ValueError('Approval refers to an outdated grouping revision: '+identity)
        additions.append({'asset_id':identity,'review_revision':item['review_revision'],
            'decision':decision,'scope':'grouping-only','exact_text':line,'note':note or '',
            'evidence':str(folder/'evidence.json')})
    if not additions:raise ValueError('No explicit gallery feedback')
    records=json.loads(destination.read_text()) if destination.exists() else {'version':1,'decisions':[]}
    if records.get('version')!=1:raise ValueError('Unsupported grouping decision format')
    records['decisions'].extend(additions)
    destination.write_text(json.dumps(records,indent=2)+'\n')
    return additions


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('feedback',type=Path);args=parser.parse_args()
    result=record(args.feedback.read_text(),OUT/'review',OUT/'grouping-decisions.json')
    print(json.dumps({'recorded':len(result),'scope':'grouping-only'}))
