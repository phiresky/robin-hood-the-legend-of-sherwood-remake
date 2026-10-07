"""Small native-ray comparison with explicit neighboring source exclusions."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from restart2_tree02_firsthit_v1 import rows_for,sample
from render_slots import acquire,release
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2'
def main():
 ver=sys.argv[sys.argv.index('--')+1];out=B/f'tree02-isolated-prototype-{ver}';guard=json.loads((out/'firsthit.json').read_text());assert guard['status'].startswith('PASS');assert not (out/'native-comparison.png').exists();acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(out/'worker.blend'));s=bpy.data.scenes['Tree02 isolated'];rows=rows_for([o for o in s.objects if o.type=='MESH'],False);box=(130,-100,295,250);w,h=165,350;actual=np.zeros((h,w,4),np.uint8);expected=np.zeros_like(actual);src=np.zeros_like(actual);covered=Image.open(B.parent/'baseline/covered.png').convert('RGBA');src[100:]=np.array(covered.crop((130,0,295,250)));bark=np.array(Image.open(B/'tree02-bark-proposal-v1/proposed-bark.png').crop((130,0,295,250)))>0;expected[100:][bark]=src[100:][bark];ownleaf=np.array(Image.open(out/'native-leaves.png').convert('RGBA'));sub=expected[100:275,45:95];sub[ownleaf[:,:,3]>0]=ownleaf[ownleaf[:,:,3]>0]
  excluded=guard['retained13Tree03NativeLeafOcclusions']
  for x,y,*_ in excluded:expected[y+100,x-130]=0
  for yy in range(h):
   for xx in range(w):actual[yy,xx]=sample(rows,xx+130,yy-100)[3]
  known=expected[:,:,3]>0;changed=known&np.any(expected!=actual,axis=2);assert not changed.any(),int(changed.sum())
  # Native context is raw map plus only the named shared canopy; it is not a global state compositor.
  canopy=np.array(Image.open(B.parent/'animation-references/animation-07/000.png').convert('RGBA').crop((130,0,295,250)));valid=canopy[:,:,3]>0;src[100:][valid]=canopy[valid]
  sheet=Image.new('RGBA',(w*3,h),(45,45,45,255))
  for i,a in enumerate((src,expected,actual)):
   im=Image.fromarray(a);base=Image.new('RGBA',im.size,(45,45,45,255));base.alpha_composite(im);sheet.paste(base,(i*w,0))
  sheet.resize((w*6,h*2),Image.Resampling.NEAREST).save(out/'native-comparison.png');write_json(out/'native-comparison.json',dict(model_sha256=sha(out/'worker.blend'),firsthit_sha256=sha(out/'firsthit.json'),image_sha256=sha(out/'native-comparison.png'),columns=['Raw map and named Arbre08 frame0 context only','Accepted own known pixels;13 neighbor-owned leaf occlusions excluded','Isolated native ray result including inferred continuation and gray unknown wood'],source_box=box,known_samples=int(known.sum()),known_changes=0,excluded_neighbor_occlusion_samples=13,limits=['Context is a spatial construction diagnostic, not complete native runtime/global state ordering.','Isolated image omits retained Tree03 foliage; its13 exact leaf occlusions are proven by the separate joint first-hit guard.','Gray lower closure remains pending ledge/terrain contact and bark appearance.']))
  print('PASS native comparison',int(known.sum()))
 finally:release()
if __name__=='__main__':main()
