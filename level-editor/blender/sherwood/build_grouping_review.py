"""Build a review of Sherwood's actual regrouped candidate meshes."""
import argparse
import hashlib
import html
import json
from pathlib import Path
import sys

EDITOR=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(EDITOR/'refinement/blender'))
from build_review_gallery import build
from grouping_decisions import group_signature


def sha(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main(root):
    root=Path(root).resolve();plan=json.loads((root/'plan.json').read_text());stage=json.loads((root/'stage.json').read_text())
    assert stage['geometry_uv_materials_unchanged'] and stage['catalog_sha256']==sha(root/'catalog.json')
    assert stage['worker_sha256']==sha(root/'grouped-source-only.blend')
    catalog=json.loads((root/'catalog.json').read_text())
    decisions_path=Path(__file__).with_name('grouping-decisions.json')
    decisions=json.loads(decisions_path.read_text())['decisions'] if decisions_path.exists() else []
    latest={(d['id'],d['group_signature']):d for d in decisions}
    inspections_path=root/'inspections.json'
    inspections=json.loads(inspections_path.read_text()) if inspections_path.exists() else {}
    items=[];approved=[]
    for group in plan['groups']:
        if not group['changed']:continue
        decision=latest.get((group['id'],group_signature(group,catalog,stage)))
        if decision and decision['decision']=='approved':
            approved.append(dict(id=group['id'],name=group['name'],review_revision=decision['review_revision'],
                group_signature=decision['group_signature'],basis='Exact reviewed membership, canonical ownership and geometry fingerprint unchanged.'))
            continue
        folder=root/'packets'/group['id'];packet=json.loads((folder/'packet.json').read_text())
        assert packet['worker_sha256']==stage['worker_sha256']
        assert packet['catalog_sha256']==stage['catalog_sha256']
        for file,digest in packet['files'].items():assert sha(folder/file)==digest
        assert (packet['views'][0]['azimuth_degrees'],packet['views'][0]['elevation_degrees'])==(0,35)
        checked=inspections.get(group['id'])==sha(folder/'packet.json')
        items.append(dict(id=group['id'],name=group['name'],status='ready-for-user' if checked else 'in-progress',
            technical_eligible=checked,user_approval='pending',approval_scope='grouping-only',
            solid=str(folder/'solid.png'),solid_label='Combined geometry — original camera first, then west, east and back',
            east_solid=str(folder/'textured.png'),east_solid_label='Original-art projection — same four cameras; gray marks unknown texture',
            context=str(folder/'original.png'),ownership=str(folder/'packet.json'),validation=str(root/'stage.json'),
            notes=[group['reason'],'Grouping review only. Existing geometry and original texture projection are unchanged.',
                   'Previous component geometry approvals remain recorded; this card reviews their combined ownership.',
                   'Contains: '+', '.join(group['sources'])+'.']))
    manifest=root/'gallery-manifest.json';manifest.write_text(json.dumps(dict(map='Sherwood',review_kind='grouping',items=items,approved=approved,
        status_counts={'pending grouping reviews':len(items),'approved groups':len(approved),'unchanged groups':sum(not g['changed'] for g in plan['groups']),
                       'selectable assets after regrouping':plan['candidate_groups']}),indent=2)+'\n')
    gallery=EDITOR/'work/needs-review/sherwood-refinement/model-review/gallery/grouping'
    build(manifest,gallery,map_name='Sherwood',pending_only=True)
    page=gallery/'index.html'
    summary='<details><summary>Unchanged assets checked in this audit</summary><ul>'+''.join(
        '<li>'+html.escape(g['name']+': '+g['reason'])+'</li>' for g in plan['groups'] if not g['changed'])+'</ul></details>'
    summary+='<details><summary>Recorded grouping approvals</summary><ul>'+''.join(
        '<li>'+html.escape(g['name'])+'</li>' for g in approved)+'</ul></details>'
    if not items:
        summary='<p><strong>Grouping review complete.</strong> All changed groups are approved. '+str(plan['candidate_groups'])+' selectable assets; no pending grouping cards.</p>'+summary
    page.write_text(page.read_text().replace('<nav>',summary+'<nav>',1))
    parent=gallery.parent/'index.html';text=parent.read_text()
    notice=f'<p><strong>Grouping has been revised.</strong> <a href="grouping/index.html">Review the {len(items)} regrouped assets here</a>.</p>'
    if 'href="grouping/index.html"' not in text:parent.write_text(text.replace('<nav>',notice+'<nav>',1))
    print(json.dumps({'gallery':str(page),'groups':len(items),'ready':sum(i['technical_eligible'] for i in items)}))


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--root',required=True);a=p.parse_args();main(a.root)
