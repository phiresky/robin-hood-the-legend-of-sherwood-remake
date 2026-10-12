"""Prepare a scoped native gold-crown fragment for private geometry comparison."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement';O=B/'restart2/tree01-native-fragment-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 inv=B/'restart2/tree01-canopy-source-v1/canopy-inputs.json';data=json.loads(inv.read_text());polygon=[(0,0),(174,0),(174,145),(125,170),(86,142),(46,123),(0,111)];mask=Image.new('L',(175,175));ImageDraw.Draw(mask).polygon(polygon,fill=255);selected=np.array(mask)>0;O.mkdir(exist_ok=False);mask.save(O/'spatial-proposal.png');rows=[];union=np.zeros_like(selected);sheet=Image.new('RGBA',(700,350),'#333333')
 for i,row in enumerate(data['frames']):
  p=Path(row['path']);assert sha(p)==row['sha256'];original=np.array(Image.open(p).convert('RGBA'))[:175,:175];fragment=original.copy();fragment[~selected,3]=0;assert np.array_equal(fragment[selected],original[selected]);alpha=fragment[:,:,3]>0;union|=alpha;dest=O/f'{i:03}.png';Image.fromarray(fragment).save(dest);rows.append({'index':i,'source_sha256':row['sha256'],'fragment_sha256':sha(dest),'alpha_pixels':int(alpha.sum()),'delay':row['delay'],'known_selected_rgba_exact':True})
 for col,idx in enumerate([0,3,7,10]):
  im=Image.open(O/f'{idx:03}.png').convert('RGBA');sheet.alpha_composite(im,(col*175,0))
 source=Image.open(B/'baseline/covered.png').convert('RGBA').crop((0,0,175,175));sheet.alpha_composite(source,(0,175));ImageDraw.Draw(sheet).text((182,195),'Top: exact native frame fragments 0/3/7/10\nBelow: painted source context (not reassigned)',fill='white');sheet.convert('RGB').save(O/'source-contact-sheet.png');Image.fromarray(union.astype('uint8')*255).save(O/'alpha-union.png');report={'status':'PRIVATE_SPATIAL_FRAGMENT_NOT_GLOBAL_OWNERSHIP','source_inventory_sha256':sha(inv),'native_box':[0,0,175,175],'polygon':polygon,'frames':rows,'alpha_union_pixels':int(union.sum()),'tree02_boundary':'Existing reviewed Tree02 native fragment begins x175; this private fragment ends x174.','source_role':'Separate provisional observed shared-Arbre08 component, never embedded into inferred static crown.','limits':['Spatial crop is a geometry-review scope, not physical animated ownership. Full original shared animation remains canonical authority.','Lower green Tree00 crown and neighboring red/gold crowns excluded; unclaimed fringe retained for later shared-source audit.','Native occupancy mask is not used as alpha.','Painted static crown/background source not reassigned by this packet.']};(O/'scope.json').write_text(json.dumps(report,indent=2)+'\n');print({'frames':len(rows),'frame0':rows[0]['alpha_pixels'],'union':int(union.sum())})
if __name__=='__main__':main()
