"""Bind every source frame and native profile center used by the sign order proof."""
import json,sys
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json

def main():
 base=OUT/'state-sign-candidate';proof=base/'native-order-reference-v3/manifest.json';records=json.loads(proof.read_text())['records'];manifest=OUT/'state-target-evidence/manifest.json';target=json.loads(manifest.read_text());profile=next(p for p in target['profiles']if p['id']=='TG_Panel-12');frames=[]
 for row in profile['rows']:
  for f in row['frames']:
   for pathkey,hashkey in [('image','image_sha256'),('raw','raw_sha256')]:assert sha(Path(f[pathkey]))==f[hashkey]
   frames.append(dict(action=row['action_id'],frame=f['index'],image_sha256=f['image_sha256'],raw_sha256=f['raw_sha256']))
 animated=json.loads((OUT/'animation-references/manifest.json').read_text())['animations'];indices=sorted({r['index']for record in records for r in record['overlapping_animations']});animations=[]
 for index in indices:
  a=next(a for a in animated if a['index']==index);bound=[]
  for f in a['frames']:
   assert sha(Path(f['source']))==f['sha256'];decoded=np.asarray(Image.open(f['image']).convert('RGBA'));raw=np.asarray(Image.open(f['source']).convert('RGBA'));visible=decoded[:,:,3]>127;assert decoded.shape==raw.shape;assert np.array_equal(decoded[visible,:3],raw[visible,:3]);bound.append(dict(image=f['image'],image_sha256=sha(Path(f['image'])),raw_sha256=f['sha256'],bbox=f['bbox']))
  native=Path(a['frames'][0]['source']).parents[2]/'manifest.json';p=next(p for p in json.loads(native.read_text())['profiles']if p['name']==a['sprite']['profile_name']);animations.append(dict(index=index,frames=bound,native_profile_manifest=str(native),native_profile_manifest_sha256=sha(native),profile_center=[p['center_x'],p['center_y']]))
 write_json(base/'native-order-reference-v3/input-audit.json',dict(status='PASS',proof_sha256=sha(proof),target_manifest_sha256=sha(manifest),target_frames=frames,animations=animations,known_visible_rgb_changed=0,limitations=['This binds decoded source inputs; it does not claim a physical scene render.']))
 print('Bound96 target frames and',sum(len(a['frames'])for a in animations),'overlay frames')

if __name__=='__main__':main()
