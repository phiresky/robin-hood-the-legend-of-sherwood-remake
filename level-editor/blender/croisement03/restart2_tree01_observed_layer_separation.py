"""Separate complementary observed leaf layers along the native ray to retain both RGBA masks."""
import sys,json,hashlib,math,shutil
from pathlib import Path
import numpy as np
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement/restart2';I=B/'tree01-native-transition-cpu-v1';O=B/'tree01-native-transition-cpu-v2'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 assert not O.exists();O.mkdir();pins={str(I/n):sha(I/n)for n in ['geometry.json','native-donor-atlas.png','cells-native.json','cells-static.json']}
 for name in ['geometry.json','native-donor-atlas.png','cells-native.json']:shutil.copyfile(I/name,O/name)
 data=json.loads((I/'cells-static.json').read_text());v=np.array(data['vertices']);s,c=math.sin(math.radians(35)),math.cos(math.radians(35));ray=np.array([0,-c,s]);changed=v-ray*.05;before=np.column_stack((v[:,0],-v[:,1]*s-v[:,2]*c));after=np.column_stack((changed[:,0],-changed[:,1]*s-changed[:,2]*c));error=float(np.max(np.abs(after-before)));assert error<1e-10;data['vertices']=changed.tolist();data['observed_layer_separation']={'world_recession':.05,'direction':(-ray).tolist(),'reason':'Complementary static and native RGBA quads previously coincided, losing 5030 observed pixels in actual native render.','prior_cells_sha256':sha(I/'cells-static.json'),'source_projection_max_error':error,'source_RGBA_UVs_unchanged':True};(O/'cells-static.json').write_text(json.dumps(data,separators=(',',':'))+'\n');(O/'receipt.json').write_text(json.dumps({'status':'PRIVATE_OBSERVED_LAYER_VISIBILITY_CORRECTION','pins':pins,'projection_error':error,'preserved':['inferred crown geometry','wood geometry','native cells','all source image bytes','all UVs'],'probe':str(B/'tree01-native-transition-saved-v1/coplanar-native-probe-v1/receipt.json')},indent=2)+'\n')
if __name__=='__main__':main()
