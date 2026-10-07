"""Compare adjacent source frames without pretending periodic links have unique identities."""
import hashlib,json,math
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';SOURCE=WORK/'geometry-pass-01/native-state-source-v1';OUT=WORK/'restart2/winch-strand-motion-v2.json'
if OUT.exists():raise FileExistsError(OUT)
record=next(r for r in json.loads((SOURCE/'manifest.json').read_text())['records'] if r['id']=='patch-004');frames=next(r['frames'] for r in record['rows'] if r['action']=='PatchTransition');poses=json.loads((WORK/'restart2/winch-motion-physical-v2/motion.json').read_text())['rows'];ys=np.arange(878,947,dtype=float);profiles=[]
for frame in frames:
    im=np.asarray(Image.open(SOURCE/frame['image']).convert('RGBA'));entry={}
    for side,lo,hi in [('left',2394,2405),('right',2406,2415)]:
        values=[]
        for y in ys:
            yy=int(y)-frame['bbox'][1]
            values.append(sum(float(im[yy,x-frame['bbox'][0],3])/255 for x in range(lo,hi) if 0<=yy<im.shape[0] and 0<=x-frame['bbox'][0]<im.shape[1]))
        entry[side]=np.asarray(values)
    profiles.append(entry)
def correlation(index,side,delta):
    y=np.arange(885,937,dtype=float);previous_y=y-delta
    valid=(previous_y>=882)&(previous_y<=940)
    if side=='left':valid&=(abs(y+.5-poses[index]['screen_center_y'])>=12)&(abs(previous_y+.5-poses[index-1]['screen_center_y'])>=12)
    current=np.interp(y[valid],ys,profiles[index][side]);previous=np.interp(previous_y[valid],ys,profiles[index-1][side]);a=current-current.mean();b=previous-previous.mean();den=float(np.linalg.norm(a)*np.linalg.norm(b))
    return float(np.dot(a,b)/den) if den>1e-8 else 0.,int(valid.sum())
steps=np.arange(-28,29,dtype=float)/8;rows=[]
for i in range(1,45):
    delta=poses[i]['screen_center_y']-poses[i-1]['screen_center_y'];row={'from_frame':i-1,'to_frame':i,'traveller_down_pixels':delta,'sides':{}}
    for side in ('left','right'):
        candidates=[{'down_pixels_modulo7':float(d),'correlation':correlation(i,side,float(d))[0]} for d in steps];candidates.sort(key=lambda r:-r['correlation']);same,count=correlation(i,side,delta);opposite,_=correlation(i,side,-delta);row['sides'][side]={'best':candidates[0],'top5':candidates[:5],'same_as_traveller_correlation':same,'opposite_to_traveller_correlation':opposite,'valid_rows_for_rigid_comparison':count,'all_scores':candidates}
    rows.append(row)
summaries={}
for side in ('left','right'):
    entries=[r['sides'][side] for r in rows];summaries[side]={'best_mean_correlation':float(np.mean([e['best']['correlation'] for e in entries])),'rigid_same_mean_correlation':float(np.mean([e['same_as_traveller_correlation'] for e in entries])),'opposite_mean_correlation':float(np.mean([e['opposite_to_traveller_correlation'] for e in entries])),'early_best_displacements':[rows[i]['sides'][side]['best']['down_pixels_modulo7'] for i in range(14)]}
# Select only a smooth representative of the modulo-seven equivalence class.
# It is a numerical preference, not a claim of tracked material link identity.
for side in ('left','right'):
    values=np.arange(-112,113,dtype=float)/8;previous_cost=values**2*.2;backpointers=[]
    for row in rows:
        samples=row['sides'][side]['all_scores'];lookup={round(r['down_pixels_modulo7'],3):r['correlation'] for r in samples};mod=(values+3.5)%7-3.5;scores=np.array([lookup[round(float(v),3)] for v in mod]);matrix=previous_cost[:,None]+.15*(values[:,None]-values[None,:])**2;backpointer=matrix.argmin(axis=0);previous_cost=matrix[backpointer,np.arange(len(values))]+(1-scores)+.002*values**2;backpointers.append(backpointer)
    state=int(previous_cost.argmin());chosen=[]
    for pointer in reversed(backpointers):chosen.append(float(values[state]));state=int(pointer[state])
    chosen.reverse();summaries[side]['smooth_representative_down_pixels']=chosen;summaries[side]['representative_total_displacement']=sum(chosen)
result={'status':'Source-only temporal diagnostic; periodic aliasing remains explicit','source_sha256':[hashlib.sha256((SOURCE/f['image']).read_bytes()).hexdigest() for f in frames],'method':'Adjacent-frame alpha-row width correlation; native rows885..936, travelling-part rows excluded on both images. Positive displacement is down. Modulo7 scores retain equivalent link identities. Smooth DP penalizes acceleration and speed; not physical gearing evidence.','summary':summaries,'rows':rows,'limitations':['Repeated links can differ by any whole7pixel period; initial hidden traveller positions are inferred.','Whole-strand shading, occlusion and rasterization can change alpha profiles.','A rigid lug requires a consistent material-link trajectory, not merely the smoothest periodic phase.']};OUT.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(summaries,indent=2))
