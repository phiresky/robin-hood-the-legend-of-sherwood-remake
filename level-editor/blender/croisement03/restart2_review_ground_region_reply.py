"""Validate the fresh aligned-guide ground response without changing its pixels."""
from pathlib import Path
import hashlib,json,base64,io
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
B=ROOT/'level-editor/work/croisement03-refinement/restart2/approved-hub-textures-v1/trio-ground'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def rgba(p):return np.array(Image.open(p).convert('RGBA'))
def main():
 e=B/'correction-v3-region-guide/experiment';g=e/'generation-short-no-mask-with-lighting-openrouter-with-auxiliary';o=e.parent/'appearance-review-v3';assert not o.exists();o.mkdir()
 gen=json.loads((g/'generation.json').read_text());c=Path(gen['cache']);req=json.loads((c/'request.json').read_text());res=json.loads((c/'response.json').read_text());inp=rgba(e/'input.png');raw=rgba(g/'generated-raw.png');fill=rgba(g/'generated-preserved.png');mask=rgba(e/'mask.png')[:,:,3]==0
 assert inp.shape==raw.shape==fill.shape==(960,1408,4) and mask.sum()==17447
 assert req['parameters']['size']=='1408x960' and req['parameters']['prompt']==gen['prompt'];assert gen['status']==res['status']==200 and gen['provider']=='openrouter' and gen['model']=='openai/gpt-image-2.5-sunburst'
 assert req['input_sha256']==sha(e/'input.png')==sha(c/'input.png');assert req['lighting_sha256']==sha(e/'solid.png')==sha(c/'lighting.png');assert sha(e/'mask.png')==sha(c/'mask.png')
 assert req['mask_sha256'] is None and gen['maskSent'] is False;assert np.array_equal(rgba(io.BytesIO(base64.b64decode(res['body']['data'][0]['b64_json']))),raw)
 assert np.array_equal(fill[~mask],inp[~mask]) and np.array_equal(fill[mask],raw[mask]) and np.array_equal(fill[:,:,3],inp[:,:,3]);assert np.array_equal(mask,rgba(B/'experiment/mask.png')[:,:,3]==0)
 refs=gen['auxiliary_references'];assert refs==req['auxiliary_references'] and refs['manifest_sha256']==sha(e/'auxiliary-references.json')
 for i,row in enumerate(refs['references']):assert sha(row['file'])==row['sha256']==sha(c/f'auxiliary-{i}.png')
 assert [r['source'] for r in refs['references']]==['input','material','material','region-guide']
 guide=rgba(e/'region-guide.png');assert np.array_equal(guide[~mask],inp[~mask]);assert np.any(guide!=inp,axis=2).sum()==17447
 paths=[g/'generation.json',g/'generated-raw.png',g/'generated-preserved.png',c/'request.json',c/'response.json',e/'input.png',e/'mask.png',e/'solid.png',e/'auxiliary-references.json',e/'region-guide.png']
 for name,p in [('filled',g/'generated-preserved.png'),('raw',g/'generated-raw.png'),('input',e/'input.png'),('guide',e/'region-guide.png'),('previous',B/'correction-v2/experiment/generation-short-no-mask-with-lighting-openrouter-with-auxiliary/generated-preserved.png')]:
  im=Image.open(p);im.crop((908,0,1202,212)).resize((882,636),Image.Resampling.NEAREST).save(o/f'{name}-detail.png');im.crop((966,105,1017,159)).resize((408,432),Image.Resampling.NEAREST).save(o/f'{name}-lower-strips.png')
 report=dict(status='Technical PASS; independent visual review pending',dimensions=[1408,960],editable_pixels=17447,protected_changes=0,alpha_exact=True,domain_exact=True,reference_images_exact=True,raw_response_exact=True,guide_sent_as_ordinary_reference=True,files={str(p):sha(p) for p in paths})
 (o/'review.json').write_text(json.dumps(report,indent=2)+'\n');print(o)
if __name__=='__main__':main()
