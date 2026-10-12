"""Emit exact pixel constraints and a scoped physical reconstruction handoff."""
import json
import numpy as np
import qualify as q
SUPPORTS={1:['tree-00','tree-03','shrub-64','shrub-55','north-woodland-bank'],2:['tree-23','tree-02','tree-03','tree-04','ground-plant-117','ground-plant-118','ground-plant-119','ground-plant-120','ground-plant-121','ground-plant-122','ground-plant-123','north-woodland-bank'],3:['tree-16','tree-18','north-firewood-stack','north-kindling-bundle'],4:['tree-24','tree-25','shrub-79','shrub-80']}
def exact_runs(frame):
 a=np.array(q.rgba(frame))[:,:,3]>0;x0,y0=map(int,frame['top_left']);columns=[]
 for x in range(a.shape[1]):
  yy=np.where(a[:,x])[0]
  if not len(yy):continue
  groups=np.split(yy,np.where(np.diff(yy)>1)[0]+1)
  columns.append(dict(x=x0+x,runs=[[y0+int(g[0]),y0+int(g[-1])+1] for g in groups]))
 return columns

def main():
 qualification=json.loads((q.OUT/'qualification.json').read_text());rows=[]
 for n,row in enumerate(qualification['rows'],1):
  frames=row['art']['states']['initial']['frames']+row['art']['states']['transition']['frames'];phases=[]
  for i,f in enumerate(frames):
   columns=exact_runs(f);a=np.array(q.rgba(f))[:,:,3]>0;assert sum(hi-lo for col in columns for lo,hi in col['runs'])==int(a.sum())
   phases.append(dict(id=['initial','transition-0','transition-1','integrated-terminal'][i],source=f['image'],top_left=f['top_left'],size=f['size'],duration_eligible_updates=f['duration'],observed_vertical_runs=columns,pixel_count=int(a.sum())))
  chosen=['croisement02-'+s for s in SUPPORTS[n]];candidates={c['id']:c for c in row['bounds_candidates']};assert all(c in candidates for c in chosen)
  rows.append(dict(profile=row['profile'],asset_id=f'croisement02-hidden-archer-{n:02}-foliage-screen',occurrences=row['occurrences'],phases=phases,retained_contact_context=[candidates[c] for c in chosen],reconstruction={
   'observed':'Foliage sheet/screen spanning an opening, lowering into a curved hanging strip. Source pixels do not establish woven fabric, a trunk, root system, person, rope, or metal support.',
   'endpoint_requirement':'New independently owned raised and lowered screen geometry. Existing static receivers remain unchanged unless a separate source/contact review proves a specific conflict.',
   'control_cage':'Fit separate endpoint silhouette envelopes to the exact column runs. Preserve disconnected leaf tips and interior gaps; do not fill the bounding rectangle. Use three source transition rows as measured intermediate pose constraints, not interpolated timing guesses.',
   'depth':'Unresolved. Choose a shallow continuous physical foliage mass only after ray/triangle first-hit and attachment/contact checks against pinned receiver models. Any thickness, back surface, tether or attachment is inference; never project the patch wholesale onto a tree crown.',
   'camera':'Original source camera first in all review sheets. Source coordinates stay exact; native image origin is authored sprite XY plus frame offset, not minus profile center.',
   'materials':'Use only this variant own four image frames for observed appearance. Preserve source palette and transparent holes. Hidden/back texture remains unknown until geometry approval; no synthesis performed.',
   'map_edge':'Variant02 upper source boundary is a display crop, not a reusable-object boundary. Infer missing continuation and label it; keep native display clipping separate.' if n==2 else 'Do not invent missing roots, branches, stump or actor anatomy from screen silhouettes.',
   'reuse':'Surrounding approved geometry is pinned context. No equivalent existing screen endpoint found; variant05 climbing geometry approval is not transferable.',
   'native_state':'Initial visible; three transition frames hold two eligible updates each; terminal third frame is integrated into background and transient FX deactivates. Reset restores initial/background only, not callback actors or latches.',
   'acceptance':['Exact native-camera source silhouette and protected outside-domain comparison for all four poses.','Independent alpha-aware current receiver/contact check; broad projected boxes and cyan triangle sheets are only diagnostics.','Full reusable-object eight-view review, including inferred back/depth and variant02 off-map continuation.','No invented actor/capture state. Keep source coupling and material/geometry approval separate.']}))
 masks=[q.pin(q.ROOT/f'level-editor/work/needs-review/map-compile/occlusion-depth-all-layers/croisement02.layer-{layer}.occlusion-depth.mask-ids.png')for layer in [0,1]]
 report=dict(status='RECONSTRUCTION_REQUIRED_NOT_APPROVED',qualification=q.pin(q.OUT/'qualification.json'),model_evidence=q.pin(q.OUT/'model-evidence.json'),mask_context=masks,rows=rows,tac19='Automatic PC crossing trigger remains separate. This packet adds no Tac19 capture or occupied-net state.',review='Native sheet, all four source composites, all four current geometry projection sheets, and both mask-layer contexts inspected. Current projected geometry is conservative, not final first-hit or attachment proof.')
 (q.OUT/'reconstruction-recipe.json').write_text(json.dumps(report,indent=2)+'\n')
 print('Four exact source-run recipes; no geometry approval or canonical change.')
if __name__=='__main__':main()
