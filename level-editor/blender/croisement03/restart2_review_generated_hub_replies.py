"""Bind Tree11 bark and corrected trio-ground responses to their approved inputs.

No synthesis or canonical writes. Ground appearance remains a separate review.
"""
from pathlib import Path
import json,hashlib,base64,io
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
B=ROOT/'level-editor/work/croisement03-refinement/restart2/approved-hub-textures-v1'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def rgba(p):return np.array(Image.open(p).convert('RGBA'))
for rel in ['croisement03-tree-11/wood-input-v1/packet-v1/experiment','trio-ground/correction-v2/experiment']:
 e=B/rel; g=e/'generation-short-no-mask-with-lighting-openrouter-with-auxiliary'; data=json.loads((g/'generation.json').read_text()); c=Path(data['cache']); req=json.loads((c/'request.json').read_text()); response=json.loads((c/'response.json').read_text()); inp=rgba(e/'input.png');raw=rgba(g/'generated-raw.png');out=rgba(g/'generated-preserved.png');mask=rgba(e/'mask.png')[:,:,3]==0
 assert inp.shape==raw.shape==out.shape
 assert data['status']==response['status']==200 and data['provider']=='openrouter' and data['model']=='openai/gpt-image-2.5-sunburst'
 assert req['parameters']['size']==f'{inp.shape[1]}x{inp.shape[0]}' and req['parameters']['prompt']==data['prompt']
 assert req['input_sha256']==sha(e/'input.png')==sha(c/'input.png'); assert req['lighting_sha256']==sha(e/'solid.png')==sha(c/'lighting.png'); assert sha(e/'mask.png')==sha(c/'mask.png')
 assert req['mask_sha256'] is None and data['maskSent'] is False
 assert np.array_equal(rgba(io.BytesIO(base64.b64decode(response['body']['data'][0]['b64_json']))),raw)
 assert np.array_equal(out[~mask],inp[~mask]) and np.array_equal(out[mask],raw[mask]); assert np.array_equal(out[:,:,3],inp[:,:,3])
 refs=data['auxiliary_references']; assert refs==req['auxiliary_references'] and refs['manifest_sha256']==sha(e/'auxiliary-references.json')
 for i,row in enumerate(refs['references']):assert sha(row['file'])==row['sha256']==sha(c/f'auxiliary-{i}.png')
 paths=[g/'generation.json',g/'generated-raw.png',g/'generated-preserved.png',c/'request.json',c/'response.json',e/'input.png',e/'mask.png',e/'solid.png',e/'auxiliary-references.json']
 if 'tree-11' in rel:
  assert {x['asset_id'] for x in refs['references']}=={'leicester-southeast-cottage-tree','leicester-moat-bank-tree'}
  review=dict(status='PASS dimensions/cache/source protection; generated sheet visually reviewed before bake',dimensions=[inp.shape[1],inp.shape[0]],filled_pixels=int(mask.sum()),protected_changes=0,request_inputs_exact=True,raw_response_exact=True,permitted_material_references_exact=True,visual_review='Eight gray-brown slender stems retain silhouettes and fine bark grain; no new roots or foliage. Private bake and saved-model native review required.',files={str(p):sha(p) for p in paths})
  assert not (e.parent/'generation-review.json').exists(), 'Never overwrite reviewed evidence'
  (e.parent/'generation-review.json').write_text(json.dumps(review,indent=2)+'\n')
 else:
  original=rgba(B/'trio-ground/experiment/input.png');assert np.array_equal(original,inp);assert mask.sum()==17447
  originalmask=rgba(B/'trio-ground/experiment/mask.png');assert np.array_equal(originalmask,rgba(e/'mask.png'))
  o=e.parent/'appearance-review-v2';o.mkdir(exist_ok=False)
  for name,p in [('input',e/'input.png'),('filled',g/'generated-preserved.png'),('raw',g/'generated-raw.png'),('old-filled',B/'trio-ground/experiment/generation-short-no-mask-with-lighting-openrouter-with-auxiliary/generated-preserved.png')]:Image.open(p).crop((908,0,1202,212)).resize((882,636),Image.Resampling.NEAREST).save(o/f'{name}-detail.png')
  review=dict(status='Technical guards PASS; visual review pending',dimensions=[1408,960],editable_pixels=17447,protected_changes=0,alpha_exact=True,original_editable_domain_exact=True,request_inputs_exact=True,raw_response_exact=True,reference_images_exact=True,files={str(p):sha(p) for p in paths})
  (o/'review.json').write_text(json.dumps(review,indent=2)+'\n')
 print(rel,review['status'],int(mask.sum()))
