"""Independently bind manual ground generation and prepare a private appearance review."""
import base64
import hashlib
import io
import json
from pathlib import Path
import numpy as np
from PIL import Image
R=Path(__file__).resolve().parents[3]
B=R/'level-editor/work/croisement03-refinement/restart2'
E=B/'approved-hub-textures-v1/trio-ground/experiment'
O=E.parent/'appearance-review-v1'
SUFFIX='Fill only the neutral-gray receiver cutouts in the northeast corner with continuous shaded olive-brown forest floor, moss and fine fallen-leaf litter at the native pixel scale. These are ground beneath removed trees: do not reconstruct tree silhouettes, trunks, roots, crowns, standing shrubs or large leaves. Preserve all surrounding painted scenery exactly. Use the two supplied same-map floor crops only for ground material character. The gray shapes are edit regions, not silhouettes of objects to redraw.'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def rgba(p):return np.array(Image.open(p).convert('RGBA'))
def main():
 O.mkdir(exist_ok=True)
 prep=json.loads((E/'preparation.json').read_text())
 for name,digest in prep['files'].items():assert sha(E/name)==digest
 genpath=E/'generation-short-no-mask-with-lighting-openrouter-with-auxiliary';gen=json.loads((genpath/'generation.json').read_text());cache=Path(gen['cache']);assert cache.parent==E/'api-cache'
 request=json.loads((cache/'request.json').read_text());assert gen['prompt']==request['parameters']['prompt'] and SUFFIX in gen['prompt'];assert gen['status']==200 and gen['provider']=='openrouter' and gen['model']=='openai/gpt-image-2.5-sunburst';assert request['parameters']['size']=='1408x960'
 assert request['input_sha256']==sha(E/'input.png')==sha(cache/'input.png');assert request['lighting_sha256']==sha(E/'solid.png')==sha(cache/'lighting.png');assert sha(E/'mask.png')==sha(cache/'mask.png');assert request['mask_sha256'] is None and gen['maskSent'] is False
 refs=json.loads((E/'auxiliary-references.json').read_text());assert request['auxiliary_references']==gen['auxiliary_references'];assert request['auxiliary_references']['manifest_sha256']==sha(E/'auxiliary-references.json')
 for i,row in enumerate(request['auxiliary_references']['references']):assert sha(row['file'])==row['sha256']==sha(cache/f'auxiliary-{i}.png')
 response=json.loads((cache/'response.json').read_text());assert response['status']==200
 decoded=rgba(io.BytesIO(base64.b64decode(response['body']['data'][0]['b64_json'])));raw=rgba(genpath/'generated-raw.png');assert np.array_equal(decoded,raw)
 original=rgba(B/'trio-tree-integration-v1/terrain-input-proposal-v5/decoded-ground-original.png');input_image=rgba(E/'input.png');filled=rgba(genpath/'generated-preserved.png');mask=rgba(E/'mask.png')[:,:,3]==0
 assert filled.shape==input_image.shape==original.shape==raw.shape==(960,1408,4);assert mask.sum()==17447
 assert np.array_equal(original[~mask],input_image[~mask]);assert np.array_equal(filled[~mask],original[~mask]);assert np.array_equal(filled[mask],raw[mask]);assert np.array_equal(filled[:,:,3],original[:,:,3])
 changed=np.any(filled!=original,axis=2);assert not (changed&~mask).any()
 yy,xx=np.nonzero(mask);box=(max(0,int(xx.min())-24),max(0,int(yy.min())-24),min(1408,int(xx.max())+25),min(960,int(yy.max())+25))
 for name,data in [('original',original),('input',input_image),('filled',filled),('raw',raw)]:Image.fromarray(data).crop(box).save(O/f'{name}-detail.png')
 Image.fromarray(filled).save(O/'filled-atlas.png');overlay=filled.copy();overlay[mask]=(.5*overlay[mask]+.5*np.array([255,0,180,255])).astype(np.uint8);Image.fromarray(overlay).crop(box).save(O/'editable-domain-detail.png')
 report=dict(status='HOLD appearance: source protection passes, visible silhouette-shaped low-detail patches remain',api_cache=str(cache),corrected_prompt_suffix_present=True,original_no_suffix_cache_not_used='888880f48c7607cefb04f6e29094b1a6d0e3dd374a9e32cb9c689c4198d76949',dimensions=[1408,960],editable_pixels=int(mask.sum()),changed_protected_pixels=int((changed&~mask).sum()),raw_response_matches_saved_raw=True,filled_pixels_exactly_raw_in_domain=True,alpha_exact=True,approved_input_files_rehashed=True,reference_images_exact=True,review_crop=list(box),visual_findings=['At native scale the left lower removed trunk remains readable as a smooth dark olive strip with a sharp right edge.','The right lower removed trunk remains a broad smooth green patch with an abrupt boundary against detailed native litter.','The filled upper receiver includes flattened low-detail bands that follow original tree cutouts rather than continuous forest-floor grain.'],next_step='Revise ground fill context/prompt using exact same protected domain; no automatic retry. New appearance approval still required.',files={str(p):sha(p) for p in [genpath/'generation.json',genpath/'generated-raw.png',genpath/'generated-preserved.png',cache/'request.json',cache/'response.json',E/'input.png',E/'mask.png',E/'solid.png',E/'auxiliary-references.json']})
 (O/'review.json').write_text(json.dumps(report,indent=2)+'\n')
 (O/'index.html').write_text('''<!doctype html><meta charset="utf-8"><title>Croisement03 ground fill review</title><style>body{background:#202323;color:#eee;font:16px system-ui;margin:24px}figure{display:inline-block;margin:8px;vertical-align:top}img{image-rendering:pixelated;max-width:100%}figcaption{margin:8px 0}.detail{width:588px}a{color:#9cf}</style><h1>Croisement03 trio ground fill — HOLD</h1><p>The corrected request is verified. All 17,447 editable pixels use the response; every protected pixel and alpha value is unchanged.</p><p>Appearance needs correction: smooth olive patches and sharp boundaries still reveal removed trunk silhouettes. This is a private review, not a request for approval.</p><figure><figcaption>Approved input</figcaption><img class="detail" src="input-detail.png"></figure><figure><figcaption>Protected final fill</figcaption><img class="detail" src="filled-detail.png"></figure><figure><figcaption>Original ground atlas</figcaption><img class="detail" src="original-detail.png"></figure><figure><figcaption>Editable domain highlighted</figcaption><img class="detail" src="editable-domain-detail.png"></figure><p><a href="review.json">Exact request, pixel guards and visual findings</a></p><img src="filled-atlas.png">''')
 print(json.dumps({k:report[k] for k in ['status','editable_pixels','changed_protected_pixels','corrected_prompt_suffix_present','reference_images_exact']}))
if __name__=='__main__':main()
