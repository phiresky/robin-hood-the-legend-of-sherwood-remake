"""Check the cluster atlas against original static foliage ownership and RGB."""
import hashlib,json
from pathlib import Path
import numpy as np
from PIL import Image

ROOT=Path(__file__).resolve().parents[3]


def main():
 run=ROOT/'level-editor/work/croisement03-refinement'
 out=run/'restart2/texture-batch-v7/croisement03-tree-25/experiment/cluster-geometry-v4'
 target=out/'source-proof-verified.json';assert not target.exists()
 construction=json.loads((out/'construction.json').read_text());x0,y0,x1,y1=construction['native_bbox']
 samples=np.load(out/'native-samples.npz');known=samples['observed'];rgba=samples['rgba']
 level=json.loads((run/'baseline/Croisement03.rhp.json').read_text());mx,my=level['masks'][116]['box_top_left']
 mask_path=run/'baseline/masks/000116.png';mask=np.array(Image.open(mask_path))>127
 expected=np.zeros(known.shape,bool);height=min(mask.shape[0],y1-my)
 expected[my-y0:my-y0+height,mx-x0:mx-x0+mask.shape[1]]=mask[:height]
 assert np.array_equal(known,expected)
 rgb_path=run/'baseline/covered.png';original=np.array(Image.open(rgb_path).convert('RGB').crop((x0,y0,x1,y1)))
 reconstructed=np.rint(np.clip(rgba[...,:3],0,1)*255).astype(np.uint8)
 assert np.array_equal(reconstructed[known],original[known])
 audit=json.loads((out/'native-appearance.json').read_text())
 assert audit['model_sha256']==construction['model_sha256']
 assert all(audit[key]==0 for key in ('missing_pixels','extra_pixels','rgb_changes_over_one_8bit_step','observed_ownership_changes','ray_depth_limits'))
 sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
 target.write_text(json.dumps(dict(status='PASS original native mask116 and source RGB preserved',
  model_sha256=construction['model_sha256'],original_mask_sha256=sha(mask_path),original_rgb_sha256=sha(rgb_path),
  observed_source_pixels=int(known.sum()),source_ownership_mask_exact=True,original_rgb_bytes_exact=True,
  full_native_audit_sha256=sha(out/'native-appearance.json'),
  limitations=['Static view-mask ownership only; wind animation remains unfinished.',
              'Pixel-centre source appearance is exact; renderer antialiasing/lighting differences are reported separately.']),indent=2)+'\n')
 print(target)


if __name__=='__main__':main()
