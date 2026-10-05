"""Freeze the user-approved initial floor request for the repository texture API."""
from pathlib import Path
import json,hashlib,shutil
from PIL import Image
R=Path(__file__).resolve().parents[3]; O=R/'level-editor/work/croisement02-refinement'; P=O/'restart3-initial-fence/floor-proposal-v2'; I=P/'inputs-v1'; E=O/'restart3-initial-fence/floor-fill-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def write(p,v):p.write_text(json.dumps(v,indent=2)+'\n')
receipt=O/'restart3-review-batches/batch-v7/user-approval.json'
assert sha(receipt)=='262e46f66799913d51cd0e34404cb58333772f6f11ea6897f57bc828d5f5a13c'
request=json.loads((I/'request.json').read_text()); review=json.loads(receipt.read_text())
E.mkdir(exist_ok=False)
for name in ['input.png','mask.png','prompt.txt']:shutil.copyfile(I/name,E/name)
Image.new('RGBA',(1792,1152),(127,127,127,255)).save(E/'solid.png')
a={'status':'approved','approved_by':'user','input_sha256':sha(E/'input.png'),'geometry_revision':'16c638be71eeb76e86439a0fdb14bac1e7bb9562afe20d175b58d0df96fb4ec2','mask_sha256':sha(E/'mask.png'),'receipt':str(receipt),'receipt_sha256':sha(receipt),'answer':review['answer'],'scope':'Exact 5419 inferred initial floor input pixels only; no native returns; generated appearance pending review','proposal_sha256':sha(P/'proposal.json'),'request_sha256':sha(I/'request.json'),'revision':'b2df62a8caa2cfccb156e9edefcda81b953ea011f2a789cb18cf4fdc53af4911','transport':'OpenRouter no-mask; authoritative mask applied locally; ordinary region guide plus four unchanged approved native references; uniform gray flat-plane lighting transport reference'}
write(E/'approval.json',a);write(P/'user-input-approval.json',a)
write(E/'views.json',{'projection_kind':'planar-atlas','layout':{'width':1792,'height':1152},'views':[{'input':'input.png','mask':'mask.png','crop':{'left':0,'top':0,'width':1792,'height':1152}}],'source_image':str(I/'input.png')})
refs=[]
for ref in request['references']:
 x,y,w,h=ref['source_bbox'];refs.append({'source':'input','file':ref['path'],'sha256':ref['sha256'],'crop':{'left':x,'top':y,'width':w,'height':h},'scale':1})
refs.append({'source':'region-guide','file':str(I/'region-guide.png'),'sha256':request['region_guide_sha256'],'role':'Only orange pixels indicate editable initial floor. Every other region, including every other gray gap, is protected. Never copy orange guide color.'})
write(E/'auxiliary-references.json',{'version':1,'input_sha256':sha(E/'input.png'),'lighting_sha256':sha(E/'solid.png'),'references':refs})
print(E)
