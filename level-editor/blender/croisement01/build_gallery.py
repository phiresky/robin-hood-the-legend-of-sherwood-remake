"""Build an honest progress gallery; no readiness or approvals are inferred."""
import html
import json
from pathlib import Path
from catalog import OUT


def main():
    gallery=OUT/'gallery';gallery.mkdir(exist_ok=True)
    inventory=json.loads((OUT/'source-survey/inventory.json').read_text())
    groups=json.loads((OUT/'catalog.json').read_text())['groups']
    fragments=[]
    names={g['id']:g['name'] for g in groups}
    for report in sorted(OUT.glob('*/assets/*/inspection/self-review.json')):
        worker=report.parent.parent
        record=json.loads(report.read_text())
        group={'id':worker.name,'name':names.get(worker.name,worker.name)}
        revision=worker.parent.parent.name
        images=[]
        for label,relative in [('Solid','modified/solid.png'),('Source projection','modified/textured.png'),
                               ('Saved materials','inspection/actual-materials/sheet.png'),
                               ('Native comparison','inspection/native-source/comparison.png')]:
            path=worker/relative
            if path.exists():images.append(f'<figure><figcaption>{label}</figcaption><a href="../{path.relative_to(OUT)}"><img src="../{path.relative_to(OUT)}"></a></figure>')
        fragments.append(f'<article id="{group["id"]}-{revision}"><h2>{html.escape(group["name"])} — {html.escape(revision)}</h2><p>{html.escape(record["status"])}</p>'+''.join(images)+'<ul>'+''.join('<li>'+html.escape(t)+'</li>' for t in record.get('findings',record.get('limitations',[record.get('finding','')])))+'</ul></article>')
    sheets=sorted((OUT/'source-survey').glob('masks-*.jpg'))
    survey=''.join(f'<a href="../source-survey/{p.name}">{p.stem}</a> ' for p in sheets)
    body=''.join(fragments) or '<p>Geometry workers are being prepared. No candidate has passed self-review yet.</p>'
    document='''<!doctype html><meta charset="utf-8"><title>Croisement01 refinement</title>
<style>body{background:#202020;color:#eee;font:16px system-ui;max-width:1400px;margin:32px auto;padding:0 24px}a{color:#9bd6ff}img{max-width:100%}article{border-top:1px solid #666;margin-top:32px;padding-top:16px}figure{margin:16px 0}.hold{color:#ffcc70}</style>
<h1>Croisement01 refinement</h1><p class="hold">Work in progress — no geometry or texture approval requested yet.</p>
<p>85 native obstacle parts, 103 masks, 13 animation records, 6 native patches and 108 mission patch records inventoried. Existing library unchanged.</p>
<p>Source grouping remains incomplete. Mask-only vegetation, complete crowns, terrain contacts, hidden surfaces and state integration remain unfinished; the native-part count is not a completion measure.</p>
<p><a href="../source-survey/visual-classification.json">Source classification</a> · <a href="../state-review/inventory.json">State inventory</a></p>
<details><summary>Native mask/context evidence</summary><p>'''+survey+'''</p></details>'''+body
    (gallery/'index.html').write_text(document)
    (gallery/'status.json').write_text(json.dumps(dict(map='Croisement01',status='in progress',
        ready_for_approval=0,geometry_approvals=0,texture_approvals=0,
        native_counts=inventory['counts'],candidate_cards=len(fragments)),indent=2)+'\n')
    print(gallery/'index.html')


if __name__=='__main__':main()
