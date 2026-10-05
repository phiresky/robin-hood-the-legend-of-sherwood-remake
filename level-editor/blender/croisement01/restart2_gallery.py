"""Isolated continuation gallery; previous review and catalogs stay untouched."""
import hashlib
import html
import json
from pathlib import Path

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement01-refinement/restart2'
SELECTION=[('branch-source-fit-v3/branch-round-10/assets/croisement01-east-fallen-branch','East Fallen Branch'),
           ('grass75-volume-v9/assets/croisement01-grass-75','East Branch Foreground Grass'),
           ('grass74-volume-v12/assets/croisement01-grass-74','Southwest Field Grass'),
           ('grass82-volume-v12/assets/croisement01-grass-82','Southeast Field Grass'),
           ('grass76-volume-v13/assets/croisement01-grass-76','Central Field Grass'),
           ('tree18-v4/assets/croisement01-tree-18','Northeast Forked Forest Tree')]


def main():
    destination=OUT/'gallery';destination.mkdir(exist_ok=True)
    cards=[];records=[]
    for relative,name in SELECTION:
        worker=OUT/relative
        if not (worker/'model.blend').exists():continue
        report=worker/'inspection/self-review.json'
        review=json.loads(report.read_text()) if report.exists() else dict(status='private candidate; review pending')
        status=review['status'];items=[]
        for label,path in [('Solid','modified/solid.png'),('Actual saved materials','inspection/actual-materials/sheet.png'),('Original source camera','inspection/native-source/comparison.png'),('Archival terrain contacts','inspection/terrain-contact/sheet.png')]:
            source=worker/path
            if not source.exists():continue
            digest=hashlib.sha256(source.read_bytes()).hexdigest()
            url='../'+str(source.relative_to(OUT))+'?sha256='+digest
            items.append(f'<figure><figcaption>{label}</figcaption><a href="{url}"><img src="{url}"></a></figure>')
        notes=''.join('<li>'+html.escape(finding)+'</li>' for finding in review.get('findings',[]))
        cards.append(f'<article id="{worker.name}"><h2>{html.escape(name)}</h2><p>{html.escape(status)}</p><ul>{notes}</ul>'+''.join(items)+'</article>')
        records.append(dict(asset_id=worker.name,name=name,worker=str(worker),model_sha256=hashlib.sha256((worker/'model.blend').read_bytes()).hexdigest(),review=review))
    document='''<!doctype html><meta charset="utf-8"><title>Crossings01 continuation review</title>
<style>body{background:#202020;color:#eee;font:16px system-ui;max-width:1400px;margin:32px auto;padding:0 24px}a{color:#9bd6ff}img{max-width:100%}article{border-top:1px solid #666;margin-top:32px;padding-top:16px}figure{margin:16px 0}</style>
<h1>Crossings01 continuation review</h1><p>Private continuation candidates. No user geometry or texture approval has been recorded. Original gallery, catalog and library remain preserved.</p>
<p>Earlier 21 private experiments remain archived. Most map assets, all scene integration and mission-state geometry are unfinished. Source-fit measurements do not establish whole-map completion.</p>
'''+''.join(cards)
    (destination/'index.html').write_text(document)
    (destination/'evidence.json').write_text(json.dumps(dict(status='private coordinator review',user_approvals=0,items=records),indent=2)+'\n')
    print(destination/'index.html')


if __name__=='__main__':main()
