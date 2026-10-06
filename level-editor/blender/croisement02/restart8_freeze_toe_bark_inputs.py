"""Freeze exact approved-model bark input scope for grouped review."""
from pathlib import Path
import json,hashlib
O=Path.cwd()/'level-editor/work/croisement02-refinement';B=O/'restart8-toe-bark-fill-v1';h=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();read=lambda p:json.loads(p.read_text());items=[];cards=[]
for n in (19,25):
 D=B/f'tree-{n}-input-v1';proof=read(D/'input-review.json');model=Path(proof['model']);assert h(model)==proof['model_sha256'];refs=read(D/'auxiliary-references.json')['references'];assert len(refs)==2
 root=dict(status='PASS concrete texture input scope for grouped V16',authority='Root personally reviewed both native-first input sheets and provenance self-review',model_sha256=proof['model_sha256'],input_sha256=h(D/'input.png'),scope='Exact approved geometry unchanged; prior inferred material replacement inside gray input only. No generated appearance approval.',user_input_approval='pending grouped V16')
 (D/'root-review.json').write_text(json.dumps(root,indent=2)+'\n')
 files=[p for p in D.rglob('*')if p.is_file()]+[model,B/'source-authority.json',B/'approved-geometry.json',Path(proof['geometry_approval']['geometry_approval_file'])]
 for ref in refs:
  p=Path(ref['file']);assert h(p)==ref['sha256'];files.append(p)
 evidence={str(p):h(p)for p in files};revision=h(D/'input.png');identity=dict(asset_id=proof['asset_id'],model_sha256=proof['model_sha256'],evidence=evidence);revision=hashlib.sha256(json.dumps(identity,sort_keys=True,separators=(',',':')).encode()).hexdigest()
 images=[dict(label=label,file=str(D/f),sha256=h(D/f))for label,f in [('Proposed unknown bark input — native camera first','input.png'),('Current approved saved appearance','actual.png'),('Unchanged approved solid geometry','solid.png')]]+[dict(label=ref['asset_id']+' — permitted supplementary bark reference',file=ref['file'],sha256=ref['sha256'])for ref in refs]
 notes=['Texture input only; model geometry already approved in Batch15.','Gray guide includes unknown and previously inferred bark that may be replaced; protected native green, olive, ivy and grazing pixels remain exact.','Crown and all upper/support geometry remain unchanged.','Only the two linked Leicester examples supplement this asset own native colors.','API waits grouped input approval; generated appearance will receive saved-model review.']
 item=dict(asset_id=proof['asset_id'],name=f'Tree {n} — bark fill input',model_sha256=proof['model_sha256'],review_revision=revision,decision='pending',scope='texture-input',evidence=evidence,displayed_images=images,notes=notes);items.append(item);cards.append(dict(card_id=f'croisement02-tree-{n}-bark-input-v1',title=item['name'],asset_ids=[proof['asset_id']]))
 ready=dict(version=1,id=cards[-1]['card_id'],name=item['name'],asset_ids=item and[proof['asset_id']],scope='texture-input',status='Root PASS; awaiting grouped input approval',candidate_model=str(model),candidate_model_sha256=proof['model_sha256'],review_revision=revision,root_review=root,presentation=dict(main_actual=str(D/'input.png'),before_actual=str(D/'actual.png'),main_solid=str(D/'solid.png'),supplementary_references=images[3:]),disclosures=notes,files={p:dict(path=p,sha256=sha)for p,sha in evidence.items()})
 (D/'ready-candidate-v1.json').write_text(json.dumps(ready,indent=2)+'\n')
 dest=B/'input-ready-v16';dest.mkdir(exist_ok=True)
(dest/'bound-members.json').write_text(json.dumps(dict(items=items,cards=cards),indent=2)+'\n');print(dest/'bound-members.json',h(dest/'bound-members.json'))
