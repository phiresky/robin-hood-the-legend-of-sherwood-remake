"""Adapt exact kindling and paired York receipts for a grouped scoped gallery."""
import hashlib,json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement02-refinement'
DEST=OUT/'restart3-review-batches/adapters-v1'
def sha(p):
 with Path(p).open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
def verify(d):
 for p,h in d['files'].items():assert sha(p)==h,p

def main():
 DEST.mkdir(exist_ok=False)
 p=OUT/'restart3-kindling/ready-candidate-v1.json';d=json.loads(p.read_text());verify(d);w=Path(d['worker'])
 revision=sha(p)
 images=[('Actual saved model, native camera top left',w/'inspection/actual-materials/sheet.png'),('Source solid geometry',w/'modified/solid.png'),('Original source and saved model',Path(d['source_trace'])),('Selected neighbor source comparison',Path(d['source_comparison'])),('Ground and neighbor contact views',Path(d['contact_sheet']))]
 def ims(rows):return [dict(label=l,file=str(p),source=str(p),sha256=sha(p)) for l,p in rows]
 member=dict(asset_id=d['asset_id'],model_sha256=d['model_sha256'],review_revision=revision,decision='pending',scope='New kindling geometry only; hidden texture fill remains pending.',notes=d['notes'],evidence={**d['files'],str(p):sha(p)},displayed_images=ims(images))
 (DEST/'kindling.json').write_text(json.dumps(dict(items=[member],cards=[dict(card_id=d['asset_id'],title='Southwest kindling bundle',asset_ids=[d['asset_id']])]),indent=2)+'\n')
 p=ROOT/'level-editor/work/york-refinement/restart2/pair-v16/ready-candidate-v1.json';d=json.loads(p.read_text());verify(d)
 evidence={str((ROOT/f).resolve()):h for f,h in d['files'].items()};evidence[str(p)]=sha(p)
 images=[(label.replace('_',' '),ROOT/path) for label,path in d['presentation'].items() if label!='first_view']
 # Individual sheets are also part of the paired geometry decision.
 images.extend((('Narrow house actual eight views' if '/bay/' in f else 'Adjoining house actual eight views'),ROOT/f) for f in d['files'] if ('/bay/' in f or '/house/' in f) and '/actual/' in f)
 members=[dict(asset_id=a,model_sha256=d['assembled_model_sha256'],review_revision=d['review_revision'],decision='pending',scope=d['scope'],notes=d['disclosures'],evidence=evidence,displayed_images=ims(images)) for a in d['asset_ids']]
 (DEST/'york-pair.json').write_text(json.dumps(dict(items=members,cards=[dict(card_id=d['id'],title=d['name'],asset_ids=d['asset_ids'])]),indent=2)+'\n')
 print(DEST)
if __name__=='__main__':main()
