"""Adapt hash-bound multi-asset ready receipts while retaining their exact revision."""
import argparse,hashlib,json
from pathlib import Path

def sha(p):
 with Path(p).open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()

def main():
 parser=argparse.ArgumentParser();parser.add_argument('receipt',type=Path);parser.add_argument('output',type=Path);args=parser.parse_args()
 if args.output.exists():raise FileExistsError(args.output)
 receipt=args.receipt.resolve();data=json.loads(receipt.read_text())
 evidence={str(Path(p).resolve()):h for p,h in data['files'].items()}
 for filename,digest in evidence.items():
  if sha(filename)!=digest:raise ValueError('Changed ready resource: '+filename)
 evidence[str(receipt)]=sha(receipt)
 model=data.get('assembled_model_sha256',data['candidate_model_sha256'])
 if not any(p.endswith('.blend') and h==model for p,h in evidence.items()):raise ValueError('Missing exact reviewed model')
 images=[]
 for label,path in data['presentation'].items():
  if label=='first_view':continue
  p=Path(path).resolve()
  if str(p) not in evidence:raise ValueError('Unbound presentation image')
  images.append(dict(label=label.replace('_',' '),file=str(p),source=str(p),sha256=evidence[str(p)]))
 members=[dict(asset_id=a,model_sha256=model,review_revision=data['review_revision'],decision='pending',scope=data['scope'],notes=data['disclosures'],evidence=evidence,displayed_images=images) for a in data['asset_ids']]
 args.output.parent.mkdir(parents=True,exist_ok=True)
 args.output.write_text(json.dumps(dict(items=members,cards=[dict(card_id=data['id'],title=data['name'],asset_ids=data['asset_ids'])]),indent=2)+'\n')
 print(args.output)
if __name__=='__main__':main()
