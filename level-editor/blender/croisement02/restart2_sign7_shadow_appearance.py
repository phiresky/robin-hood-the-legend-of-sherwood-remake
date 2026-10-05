"""Keep painted-shadow appearance separate from opaque sign-body visibility."""
import sys,json
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json

def main():
 base=OUT/'restart2-fence/shrub57-sign-bend-v9';proposal=OUT/'state-sign-candidate/painted-shadow-v2/proposal.json';data=json.loads(proposal.read_text());assert data['coordinates']=={'canvas_size':[64,72],'anchor':[32,55]};records=[]
 for row in data['records']:
  phase=row['frame'];mask=Path(row['mask']);assert sha(mask)==row['mask_sha256'];native=np.array(Image.open(mask).convert('RGBA'))[:,:,3]>127;expected=np.zeros((96,96),bool);expected[9:81,16:80]=native
  actual_path=base/f'joint-proof/pose-{phase:02}-actual.png';actual=np.array(Image.open(actual_path).convert('RGBA'))[1::3,1::3];black=(actual[:,:,:3].max(2)<12)&(actual[:,:,3]>127)
  records.append(dict(phase=phase,expected_painted_shadow_pixels=int(expected.sum()),physical_matching_black_pixels=int((black&expected).sum()),not_matching_black_pixels=int((expected&~black).sum()),physical_render_sha256=sha(actual_path)))
 p=base/'painted-shadow-appearance.json';assert not p.exists();write_json(p,dict(status='Physical/native painted-shadow appearance divergence explicitly retained',model_sha256=sha(base/'model.blend'),proposal_sha256=sha(proposal),records=records,limitations=['Screen-space near-black comparison is an appearance metric, not first-hit object attribution.','Opaque sign-body visibility statistics do not include painted-ground shadow masks.','Native sprite composition draws black paint above static map foliage, while the physical horizontal receiver can be obscured by foliage.','This is a separate native presentation/physical depth contract, not a claim of exact full-scene pixel parity or completed mission integration.']))
 print(p)
if __name__=='__main__':main()
