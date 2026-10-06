"""Reuse rigidly translated gate detail while restoring exact raised native pixels."""
import json,math,hashlib,shutil
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/york-refinement/restart2/gate-texture-inputs-v1';a=BASE/'covered/experiment';b=BASE/'raised/experiment';source=a/'generation-short-no-mask-with-lighting-openrouter-with-auxiliary';out=b/'reused-covered-generation-v1'
if out.exists():raise FileExistsError(out)
fa=json.loads((a/'views.json').read_text());fb=json.loads((b/'views.json').read_text());assert fa['layout']==fb['layout'];delta=np.zeros((4,4));delta[2,3]=57/math.cos(math.radians(35));max_error=0
for x,y in zip(fa['views'],fb['views']):
 assert x['crop']==y['crop'];assert abs(x['ortho_scale']-y['ortho_scale'])<.0001;e=float(np.max(np.abs(np.array(y['camera_matrix_world'])-np.array(x['camera_matrix_world'])-delta)));assert e<.003;e and None;max_error=max(max_error,e)
original=np.array(Image.open(b/'input.png').convert('RGBA'));mask=np.array(Image.open(b/'mask.png').convert('RGBA'));raw=np.array(Image.open(source/'generated-raw.png').convert('RGBA'));assert original.shape==raw.shape;editable=mask[:,:,3]==0;result=original.copy();result[editable,:3]=raw[editable,:3];assert np.array_equal(result[~editable],original[~editable]);assert np.array_equal(result[:,:,3],original[:,:,3]);out.mkdir();shutil.copyfile(source/'generated-raw.png',out/'generated-raw.png');Image.fromarray(result).save(out/'generated-preserved.png');h=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();(out/'reuse-receipt.json').write_text(json.dumps({'method':'Rigidly translated same geometry and native-first camera layout; generated RGB reused only in raised editable pixels. Known raised source and full input alpha exact.','covered_generation_sha256':h(source/'generated-raw.png'),'covered_views_sha256':h(a/'views.json'),'raised_views_sha256':h(b/'views.json'),'maximum_camera_roundoff':max_error,'protected_changed':0,'alpha_changed':0,'provider_calls':0},indent=2)+'\n');print(out)
