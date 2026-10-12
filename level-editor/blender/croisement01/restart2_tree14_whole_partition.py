"""Consolidated source-guided branch/foreground proposal with explicit inferred borders."""
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw
from scipy.ndimage import binary_dilation, binary_erosion, distance_transform_edt

ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/croisement01-refinement'
R=BASE/'restart2';OUT=R/'tree14-whole-source-role-proposal-v1';assert not OUT.exists();OUT.mkdir()
P=R/'tree14-layered-comparison-v1';CASE=R/'tree14-complete-material-casebook-v1'
read=lambda name:np.asarray(Image.open(P/(name+'.png')).convert('L'))>0
accepted=read('accepted-bark');foreground_accepted=read('foreground-candidate')
retained=read('retained-base-unreviewed');disputed=read('ambiguous-restored');edge=read('unresolved-edge')
H,W=accepted.shape
# Contours follow coherent visible branch runs reviewed against all37 source
# units. The one/two-pixel edge around each contour is an explicit inference.
contours={
 'main-upper':[(750,123),(756,133),(762,143),(772,152),(784,162),(796,174),(806,182),(809,191),(810,201),(809,211),(813,225),(821,241),(820,246),(809,230),(805,217),(804,205),(804,193),(802,185),(791,175),(779,164),(767,154),(757,143),(752,133)],
 'right-fork':[(815,247),(822,242),(833,236),(843,230),(849,224),(851,219),(850,209),(849,204),(851,198),(855,193),(861,190),(858,194),(854,199),(853,205),(855,215),(855,222),(851,229),(845,235),(835,241),(824,249),(817,253)],
 'right-tip':[(851,225),(861,221),(871,217),(880,216),(889,217),(896,220),(904,227),(906,230),(902,228),(894,222),(887,220),(880,219),(873,221),(861,225),(853,228)],
 'left-main':[(694,266),(700,273),(709,277),(720,280),(734,286),(750,293),(762,298),(767,299),(777,294),(787,288),(798,284),(801,289),(790,293),(780,297),(770,303),(764,305),(753,300),(736,291),(721,285),(708,281),(699,277),(693,269)],
 'left-upper':[(719,204),(726,209),(732,214),(738,224),(738,234),(740,244),(746,253),(759,262),(774,268),(790,272),(796,276),(793,280),(778,274),(763,270),(751,264),(743,257),(737,247),(734,237),(734,225),(728,215),(722,209)],
 'left-twig':[(695,208),(702,215),(708,224),(714,233),(721,242),(730,248),(743,252),(743,256),(728,252),(719,246),(712,237),(706,227),(700,218),(694,211)],
}
contour_mask=Image.new('L',(W,H));draw=ImageDraw.Draw(contour_mask)
for points in contours.values():draw.polygon([(x-645,y-39) for x,y in points],fill=255)
inside=np.asarray(contour_mask)>0
casebook=json.loads((CASE/'casebook.json').read_text())
# These narrow isolated units visibly continue the same striated twig axis.
# This is an explicit visual judgement, not a color or component-size rule.
wood_case_ids={7,9,11,12,15,19,23,25,26,32,34}
case_wood=np.zeros((H,W),bool)
for case in casebook['cases']:
 if case['case'] in wood_case_ids:
  for sample in case['samples']:
   x,y=sample['pixel'];case_wood[y-39,x-645]=True
new_wood=disputed&(inside|case_wood)
# One discrete gold crossing lies across the striated right-fork run. Its
# boundary excludes the already accepted bark core by construction.
crossings={
 'right-fork-crossing':[(841,231),(844,228),(848,230),(848,233),(845,237),(842,236),(840,234)],
 'left-upper-crossing':[(755,264),(761,263),(767,267),(765,272),(759,271),(755,268)],
 'left-junction-crossing':[(751,283),(758,281),(764,284),(764,288),(759,291),(754,289)],
}
leaf_shapes=Image.new('L',(W,H));draw=ImageDraw.Draw(leaf_shapes)
for points in crossings.values():draw.polygon([(x-645,y-39) for x,y in points],fill=255)
new_wood &= ~(np.asarray(leaf_shapes)>0)
wood=accepted|retained|new_wood
foreground=foreground_accepted|(disputed&~new_wood)|edge
assert not (wood&foreground).any()
assert np.array_equal(wood|foreground,accepted|retained|foreground_accepted|disputed|edge)
assert np.all(wood[accepted]) and np.all(foreground[foreground_accepted])
# Every border decision remains inspectable as an inference. Source color is
# always observed even when the physical receiving material is inferred.
line=inside&~binary_erosion(inside)
inferred=(binary_dilation(line,iterations=2)&disputed)|edge
inferred |= case_wood&disputed
source_path=BASE/'baseline/covered.png';source=Image.open(source_path).convert('RGBA')
rgba=np.asarray(source.crop((645,39,985,442)))
for name,m in [('wood',wood),('foreground',foreground),('inferred-role-border',inferred)]:
 Image.fromarray(m.astype(np.uint8)*255).save(OUT/(name+'-full.png'))
 local=m[122-39:439-39,689-645:916-645]
 assert local.shape==(317,227)
 Image.fromarray(local.astype(np.uint8)*255).save(OUT/(name+'-domain.png'))
 pixels=rgba.copy();pixels[:,:,3]=m*255;pixels[~m]=0
 Image.fromarray(pixels).save(OUT/(name+'-source.png'))
colors=rgba.copy();colors[~(wood|foreground)]=0
colors[wood,:3]=(65,185,245);colors[foreground,:3]=(230,95,210);colors[inferred,:3]=(245,195,70)
Image.fromarray(colors).save(OUT/'role-overlay.png')
plain=source.crop((680,110,925,315)).convert('RGB').resize((980,820),Image.Resampling.NEAREST)
marked=plain.copy();draw=ImageDraw.Draw(marked)
for points in contours.values():
 pts=[((x-680)*4,(y-110)*4) for x,y in points];draw.line(pts+[pts[0]],fill='#54c8ef',width=1)
for points in crossings.values():
 pts=[((x-680)*4,(y-110)*4) for x,y in points];draw.line(pts+[pts[0]],fill='#ee6ed9',width=2)
board=Image.new('RGB',(1960,850),'#292930');board.paste(plain,(0,30));board.paste(marked,(980,30));draw=ImageDraw.Draw(board)
draw.text((5,8),'Untouched source',fill='white');draw.text((985,8),'Coherent branch contours / discrete foreground crossings; borders inferred',fill='white');board.save(OUT/'whole-branch-role-comparison.png')
hits=np.load(P/'receiver-depths.npz')['wood_hit'];missing=wood&~hits
report=dict(status='CONSOLIDATED_PRIVATE_ROLE_PROPOSAL_REQUIRES_COMPLETE_TREE_GEOMETRY_REVIEW',
 source_sha256=hashlib.sha256(source_path.read_bytes()).hexdigest(),native_origin=[689,122],
 counts=dict(wood=int(wood.sum()),foreground=int(foreground.sum()),new_disputed_bark=int(new_wood.sum()),new_disputed_foreground=int((disputed&~new_wood).sum()),inferred_role_borders=int(inferred.sum()),unassigned=0,retained_accepted_bark=int(accepted.sum()),retained_foreground_candidates=int(foreground_accepted.sum())),
 contours=contours,foreground_crossings=crossings,wood_case_ids=sorted(wood_case_ids),casebook_sha256=hashlib.sha256((CASE/'casebook.json').read_bytes()).hexdigest(),
 existing_wood_geometry_misses=[[int(x+645),int(y+39)] for y,x in zip(*np.nonzero(missing))],
 decisions=['All37 units have a proposed role. No source pixels discarded or synthesized.',
 'Continuous branch contours and selected narrow twig units are proposed bark; crossing/off-axis fragments are proposed foreground.',
 '40 detached right-edge fragments are inferred foreground; their native colors remain protected.',
 'Inferred border roles are decisions to review in the complete tree, not additional observed evidence.',
 'Crown94 and neighboring source owners still require joint geometry and first-hit validation.'],
 no_model_mutation=True,no_texture_synthesis=True)
(OUT/'proposal.json').write_text(json.dumps(report,indent=2)+'\n');print(report['counts'])
