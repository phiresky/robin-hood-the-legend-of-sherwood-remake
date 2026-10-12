"""Isolate saved screen visibility behind unchanged static alpha/geometry."""
import sys,json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.ndimage import binary_erosion
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
import joined02_review as original
from render_slots import acquire,release

def main():
 source=Path(original.__file__).read_text()
 needle="receivers.extend(o for o in added if o.type=='MESH')"
 assert source.count(needle)==1
 source=source.replace(needle,needle+"\n  for obj in added:\n   if obj.type=='MESH':obj.is_holdout=True")
 scope={'__name__':'joined02_holdout_private','__file__':original.__file__}
 exec(compile(source,original.__file__,'exec'),scope)
 dest=original.q.OUT/'joined02-v3-holdout-coverage';scope['DEST']=dest;scope['VIEW_REPORT']=original.q.OUT/'joined02-camera-input-v2/report.json';scope['main']()
 report=json.loads((dest/'report.json').read_text());crop=report['crop'];qualification=json.loads((original.q.OUT/'qualification.json').read_text());row=qualification['rows'][1];checks=[]
 for phase in ['initial','applied']:
  frame=row['art']['states']['initial']['frames'][0]if phase=='initial'else row['art']['states']['transition']['frames'][-1]
  expected=np.zeros((crop[3]-crop[1],crop[2]-crop[0],4),np.uint8);rgba=np.array(original.q.rgba(frame));x=int(frame['top_left'][0])-crop[0];y=int(frame['top_left'][1])-crop[1];expected[y:y+rgba.shape[0],x:x+rgba.shape[1]]=rgba
  actual=np.array(Image.open(dest/f'{phase}-joined.png').convert('RGBA'));mask=expected[:,:,3]>0;interior=binary_erosion(mask,iterations=1);lost=mask&(actual[:,:,3]<128);interior_lost=interior&(actual[:,:,3]<128)
  error=np.max(abs(actual[:,:,:3].astype(int)-expected[:,:,:3].astype(int)),axis=2)
  overlay=np.zeros_like(expected);overlay[mask]=[40,140,40,255];overlay[lost]=[255,0,0,255];overlay[interior_lost]=[255,0,255,255];Image.fromarray(overlay).save(dest/f'{phase}-coverage.png')
  checks.append(dict(phase=phase,expected_pixels=int(mask.sum()),interior_pixels=int(interior.sum()),alpha_below_half=int(lost.sum()),interior_alpha_below_half=int(interior_lost.sum()),interior_rgb_difference_over_8=int((interior&(error>8)).sum()),actual=original.q.pin(dest/f'{phase}-joined.png'),coverage=original.q.pin(dest/f'{phase}-coverage.png')))
 (dest/'coverage.json').write_text(json.dumps(dict(status='DIAGNOSTIC_REQUIRES_VISUAL_REVIEW',render_report=original.q.pin(dest/'report.json'),base_recipe=original.q.pin(Path(original.__file__)),checks=checks,limits=['Static neighbor meshes use Cycles holdout without modifying topology, transforms or source alpha.','Threshold half-alpha and RGB differences include antialiasing/render effects; interior and boundary disagreement are separated.','The native source domain is independent of the model acceptance mask.','No geometry or publication changed.']),indent=2)+'\n')
if __name__=='__main__':
 acquire(slots=2)
 try:main()
 finally:release()
