"""Freeze mixed, explicitly scoped review cards without changing source revisions."""
from __future__ import annotations
import argparse
import hashlib
import html
import json
import os
from pathlib import Path
import shutil


def sha(path):
    with Path(path).open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('config', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    config = json.loads(args.config.read_text())
    out = args.output.resolve()
    if out.exists():
        raise ValueError(f'Frozen output must be new: {out}')
    # Validate all upstream bindings before creating the output directory.
    cards, resources, sources = [], {}, []
    def resource(path, expected=None):
        p = Path(path).resolve(strict=True)
        digest = sha(p)
        if expected is not None and digest != expected:
            raise ValueError(f'Hash mismatch: {p}')
        target = f'resources/{digest}{p.suffix.lower()}'
        resources[target] = dict(source=str(p), sha256=digest)
        return dict(file=target, sha256=digest, source=str(p))
    seen = set()
    for spec in config['sources']:
        path = Path(spec['evidence']).resolve(strict=True)
        packet = json.loads(path.read_text())
        sources.append(dict(kind=spec['kind'], evidence=resource(path)))
        if spec['kind'] == 'source-artifact':
            asset, scope = packet['card_id'], packet['review_scope']
            if (asset, scope) in seen:
                raise ValueError(f'Duplicate decision scope: {(asset, scope)}')
            seen.add((asset, scope))
            reports = [dict(label='Exact contract manifest', **resource(packet['artifact'], packet['artifact_sha256']))]
            for label in ('root_review', 'parity'):
                reports.append(dict(label=label.replace('_', ' '), **resource(packet[label], packet[label+'_sha256'])))
            root_evidence = json.loads(Path(packet['root_review']).read_text())['evidence']
            for filename, digest in root_evidence.items():
                reports.append(dict(label=Path(filename).name, **resource(filename, digest)))
            contract_ids = set()
            for contract in packet['members']:
                if contract['id'] in contract_ids:
                    raise ValueError('Duplicate source contract: '+contract['id'])
                contract_ids.add(contract['id'])
                reports.append(dict(label=contract['id'], **resource(contract['contract'], contract['contract_sha256'])))
            images = [dict(label=packet['source_comparison_label'], **resource(packet['source_comparison'], root_evidence[packet['source_comparison']]))]
            member = dict(asset_id=asset, title=packet['name'], scope=scope,
                scope_description=scope, artifact=packet['artifact'], artifact_sha256=packet['artifact_sha256'],
                contract_members=packet['members'], review_revision=packet['review_revision'],
                source_evidence=str(path), source_evidence_sha256=sha(path), images=images,
                reports=reports, notes=packet['notes'], decision='pending')
            cards.append(dict(card_id='source-artifact-'+asset, title=packet['name'], scope=scope, members=[member]))
            continue
        members = {}
        selected = set(spec.get('asset_ids', []))
        for item in packet['items']:
            asset = item.get('id', item.get('asset_id'))
            if selected and asset not in selected:
                continue
            scope = spec.get('scope', 'geometry' if spec['kind'] == 'geometry-gallery' else 'texture')
            key = (asset, scope)
            if key in seen:
                raise ValueError(f'Duplicate decision scope: {key}')
            seen.add(key)
            images, reports = [], []
            if spec['kind'] in ('geometry-gallery','texture-gallery','scoped-gallery'):
                if item['status'] != 'ready-for-user' or not item.get('technical_eligible'):
                    raise ValueError(f'Not ready: {asset}')
                model = Path(item['model'])
                model_hash = sha(model)
                binding = {kind: {k:v['sha256'] for k,v in item[kind].items()} for kind in ('images','reports')}
                binding['model'] = model_hash
                revision = hashlib.sha256(json.dumps(binding,sort_keys=True).encode()).hexdigest()
                if revision != item['review_revision']:
                    raise ValueError(f'Revision mismatch: {asset}')
                # Actual stored material, preferably full bounds, is shown first.
                order = sorted(item['images'], key=lambda k: (-1 if k==spec.get('primary_image') else 0 if 'full-crown_textured' in k else 1 if k=='stored_material_textured' else 2 if k=='context' else 3, k))
                for label in order:
                    entry = item['images'][label]
                    images.append(dict(label=item.get(label+'_label',label.replace('_',' ')), **resource(path.parent / entry['file'],entry['sha256'])))
                for label,entry in item['reports'].items():
                    reports.append(dict(label=label, **resource(path.parent / entry['file'],entry['sha256'])))
                title = item['name']
                detail = spec.get('scope_description') or ('Geometry only. Texture completion and mission-state behavior are separate decisions.' if scope=='geometry' else 'Texture appearance only on previously approved geometry; no new geometry or mission-state approval.')
            elif spec['kind'] in ('endpoint-textures','bound-members'):
                if item.get('texture_approval',item.get('decision')) != 'pending':
                    raise ValueError(f'Already decided: {asset}')
                model_hash = item['model_sha256']
                model_paths = []
                for filename,digest in item['evidence'].items():
                    p = Path(filename)
                    if p.suffix in ('.blend', '.glb'):
                        if sha(p) != digest:
                            raise ValueError(f'Model binding mismatch: {p}')
                        if digest == model_hash: model_paths.append(p)
                    else:
                        reports.append(dict(label=p.name, **resource(p,digest)))
                if len(model_paths) != 1:
                    raise ValueError(f'Expected one exact model: {asset}')
                model = model_paths[0]
                for entry in item['displayed_images']:
                    images.append(dict(label=entry['label'], **resource(path.parent/entry['file'],entry['sha256'])))
                title, detail = item.get('name',asset), item['scope']
            else:
                raise ValueError(f'Unsupported packet kind: {spec["kind"]}')
            notes = item.get('notes', [])
            if isinstance(notes,str): notes=[notes]
            members[asset] = dict(asset_id=asset,title=title,scope=scope,scope_description=detail,
                model=str(model.resolve()),model_sha256=model_hash,review_revision=item['review_revision'],
                source_evidence=str(path),source_evidence_sha256=sha(path),images=images,reports=reports,notes=notes,decision='pending',
                **{key:item[key] for key in ('canonical_asset_id','state') if key in item})
        if selected and selected != set(members):
            raise ValueError(f'Missing requested assets: {selected-set(members)}')
        if spec['kind'] in ('geometry-gallery','texture-gallery','scoped-gallery'):
            cards.extend(dict(card_id=m['scope'].replace(' ','-')+'-'+m['asset_id'],title=m['title'],scope=m['scope'],members=[m]) for m in members.values())
        else:
            used=set()
            for card in packet['cards']:
                included=set(card['asset_ids']) & set(members)
                if not included: continue
                if included != set(card['asset_ids']):
                    raise ValueError('Cannot silently split a paired endpoint card')
                used |= included
                cards.append(dict(card_id=scope.replace(' ','-')+'-'+card['card_id'],title=card['title'],scope=scope,members=[members[a] for a in card['asset_ids']]))
            if used!=set(members): raise ValueError('Uncarded texture member')
    for entry in config.get('supplementary_evidence',[]):
        sources.append(dict(kind='supplementary provenance',evidence=resource(entry['file'],entry['sha256'])))
    resource_bytes = sum(Path(entry['source']).stat().st_size for entry in resources.values())
    if resource_bytes > config.get('max_resource_bytes', float('inf')):
        raise ValueError(f'Review resources exceed write budget: {resource_bytes}')
    if shutil.disk_usage(out.parent).free - resource_bytes < config.get('minimum_free_bytes', 0):
        raise ValueError('Review resources would cross the reserved disk floor')
    out.mkdir(parents=True)
    (out/'resources').mkdir()
    for rel,entry in resources.items():
        reuse = (Path(config['reuse_frozen_resources']) / Path(rel).name
                 if config.get('reuse_frozen_resources') else None)
        if reuse is not None and reuse.is_file() and sha(reuse)==entry['sha256']:
            os.link(reuse, out/rel)
        else:
            shutil.copyfile(entry['source'],out/rel)
        if sha(out/rel)!=entry['sha256']: raise ValueError(f'Copy mismatch: {rel}')
    evidence=dict(status='Frozen pending user review',title=config['title'],cards=cards,sources=sources,
        resources=resources,resource_bytes=resource_bytes,card_count=len(cards),decision_count=sum(len(c['members']) for c in cards),
        approval_policy='Only explicit decisions for these exact member revisions apply; Each geometry, texture and source-state application scope remains separate.')
    (out/'evidence.json').write_text(json.dumps(evidence,indent=2)+'\n')
    escaped=lambda s:html.escape(str(s),quote=True)
    sections=[]
    for n,card in enumerate(cards,1):
        body=[]
        for member in card['members']:
            pictures=''.join(f'<figure><a href="{escaped(im["file"])}" target="_blank"><img loading="lazy" src="{escaped(im["file"])}" alt="{escaped(im["label"])}"></a><figcaption>{escaped(im["label"])}</figcaption></figure>' for im in member['images'])
            notes=''.join(f'<li>{escaped(note)}</li>' for note in member['notes'])
            reports=''.join(f'<li><a href="{escaped(r["file"])}">{escaped(r["label"])}</a></li>' for r in member['reports'])
            digest_label = 'Artifact' if 'artifact_sha256' in member else 'Model'
            digest = member.get('artifact_sha256', member.get('model_sha256'))
            body.append(f'<section class="member"><h3>{escaped(member["title"])}</h3><p>{escaped(member["scope_description"])}</p><p class="revision">{escaped(member["asset_id"])} · review {member["review_revision"][:16]}</p>{pictures}<ul>{notes}</ul><details><summary>Bound reports and evidence</summary><ul>{reports}</ul><p>{digest_label} SHA256: {digest}</p></details></section>')
        sections.append(f'<article id="{escaped(card["card_id"])}"><h2>{n}. {escaped(card["title"])} <span>{card["scope"].upper()}</span></h2>{"".join(body)}<label>Decision <select data-card="{n-1}"><option value="">Not reviewed</option><option>approved</option><option>needs refinement</option><option>feedback</option></select></label><label> Notes <textarea data-note="{n-1}" rows="2"></textarea></label></article>')
    script="""const cards=DATA;const storageKey='scoped-review-'+cards.flatMap(c=>c.members.map(m=>m.review_revision)).join('-');
const read=()=>cards.map((c,n)=>({decision:document.querySelector(`[data-card="${n}"]`).value,note:document.querySelector(`[data-note="${n}"]`).value}));
try{const saved=JSON.parse(localStorage.getItem(storageKey)||'null');if(saved)cards.forEach((c,n)=>{document.querySelector(`[data-card="${n}"]`).value=saved[n]?.decision||'';document.querySelector(`[data-note="${n}"]`).value=saved[n]?.note||''})}catch(e){}
document.querySelectorAll('select,textarea[data-note]').forEach(e=>e.addEventListener('input',()=>{try{localStorage.setItem(storageKey,JSON.stringify(read()))}catch(e){}}));
document.querySelector('#export').onclick=()=>{const lines=[];cards.forEach((c,n)=>{const d=document.querySelector(`[data-card="${n}"]`).value;if(!d)return;const note=document.querySelector(`[data-note="${n}"]`).value;for(const m of c.members)lines.push(`${m.asset_id}: ${d} (${m.scope})${note?' — '+note:''} [review ${m.review_revision}]`)});document.querySelector('#feedback').value=lines.join('\\n');};""".replace('DATA',json.dumps(cards).replace('</','<\\/'))
    navigation=''.join(f'<li><a href="#{escaped(c["card_id"])}">{escaped(c["title"])} — {c["scope"]}</a></li>' for c in cards)
    page=f'''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{escaped(config['title'])}</title><style>body{{font:16px system-ui;background:#15181c;color:#eef0f4;margin:24px auto;max-width:1450px;padding:0 20px}}a{{color:#a7d4ff}}article{{border:1px solid #555;border-radius:10px;margin:28px 0;padding:20px}}h2 span{{font-size:14px;background:#384861;padding:5px}}img{{width:100%;height:auto;background:#242831}}figure{{margin:16px 0}}figcaption,.revision{{color:#b5c1d0}}details{{overflow-wrap:anywhere}}textarea{{display:block;width:98%;background:#222;color:white}}select,button{{padding:10px}}li{{margin:6px 0}}</style><h1>{escaped(config['title'])}</h1><p>{len(cards)} cards, {evidence['decision_count']} exact scoped decisions. Each card states its exact review scope. Grouped cards apply one decision to every displayed member. Native camera is top left in eight-view sheets. Click any image for full resolution.</p><p><a href="evidence.json">Frozen evidence manifest</a>. Decisions below prepare feedback; they do not modify any model or approval record.</p><details><summary>Jump to a review card</summary><ul>{navigation}</ul></details>{''.join(sections)}<button id="export">Prepare review feedback</button><textarea id="feedback" rows="12" readonly></textarea><script>{script}</script></html>'''
    (out/'index.html').write_text(page)
    print(json.dumps(dict(output=str(out),cards=len(cards),decisions=evidence['decision_count'],resources=len(resources))))

if __name__=='__main__': main()
