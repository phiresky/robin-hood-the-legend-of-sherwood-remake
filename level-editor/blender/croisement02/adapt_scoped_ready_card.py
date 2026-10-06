"""Adapt hash-bound multi-asset ready receipts while retaining their exact revision."""
import argparse,hashlib,json
from pathlib import Path

def sha(p):
 with Path(p).open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()

def main():
 parser=argparse.ArgumentParser();parser.add_argument('receipt',type=Path);parser.add_argument('output',type=Path);args=parser.parse_args()
 if args.output.exists():raise FileExistsError(args.output)
 receipt=args.receipt.resolve();data=json.loads(receipt.read_text())
 evidence={}
 for p,value in data['files'].items():
  if isinstance(value,dict):
   if Path(value['path']).resolve()!=Path(p).resolve():raise ValueError('Receipt file key/path mismatch')
   digest=value['sha256']
  else:digest=value
  evidence[str(Path(p).resolve())]=digest
 for filename,digest in evidence.items():
  if sha(filename)!=digest:raise ValueError('Changed ready resource: '+filename)
 evidence[str(receipt)]=sha(receipt)
 if data.get('worker') and data.get('asset_id'):
  worker=Path(data['worker']);asset=data['asset_id'];model=data['model_sha256']
  if evidence.get(str((worker/'model.blend').resolve()))!=model:raise ValueError('Unbound worker model')
  rows=[('Actual saved model, original camera top left',Path(data.get('actual_sheet',worker/'inspection/actual-materials/sheet.png'))),('Solid geometry, original camera top left',Path(data.get('solid_sheet',worker/'modified/solid.png'))),('Original source and saved model',Path(data['source_trace'])),('Native ground and neighbor comparison',Path(data['source_comparison'])),('Ground and neighbor contact views',Path(data['contact_sheet']))]
  images=[]
  for label,path in rows:
   p=str(path.resolve())
   if p not in evidence:raise ValueError('Unbound worker presentation image: '+p)
   images.append(dict(label=label,file=p,source=p,sha256=evidence[p]))
  title=data.get('name',asset.removeprefix('croisement02-').replace('-',' ').capitalize())
  member=dict(asset_id=asset,name=title,model_sha256=model,review_revision=data.get('review_revision',sha(receipt)),decision='pending',scope='New geometry only; inferred hidden texture completion remains separate.',notes=data['notes'],evidence=evidence,displayed_images=images)
  args.output.parent.mkdir(parents=True,exist_ok=True)
  args.output.write_text(json.dumps(dict(items=[member],cards=[dict(card_id=asset,title=title,asset_ids=[asset])]),indent=2)+'\n');print(args.output);return
 if data.get('states'):
  canonical,=data['asset_ids'];members=[]
  for index,state in enumerate(data['states']):
   state_id=state['patch001']+'-'+state['patch002'];images=[]
   for filename,digest in state['files'].items():
    if filename.endswith('.blend'):continue
    if 'actual/' in filename:label='Actual eight views: '+state_id
    elif 'complete-object/' in filename:label='Solid eight views: '+state_id
    elif 'original-native' in filename:label='Original artwork and native model comparison: '+state_id
    else:label='Native joint: '+state_id
    path=str(Path(filename).resolve());images.append(dict(label=label,file=path,source=path,sha256=digest))
   if index==len(data['states'])-1:
    path=str(Path(data['presentation']['terrain']).resolve());images.append(dict(label='Foundation and original terrain contact',file=path,source=path,sha256=evidence[path]))
   members.append(dict(asset_id=canonical+'--'+state_id,canonical_asset_id=canonical,state=state_id,name='York castle great hall — '+state_id,model_sha256=state['model_sha256'],review_revision=data['review_revision'],decision='pending',scope=data['scope'],notes=data['disclosures']+[data['roof_semantics'][index]],evidence=evidence,displayed_images=images))
  args.output.parent.mkdir(parents=True,exist_ok=True)
  args.output.write_text(json.dumps(dict(items=members,cards=[dict(card_id=data['id'],title=data['name'],asset_ids=[m['asset_id'] for m in members])]),indent=2)+'\n')
  print(args.output);return
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
