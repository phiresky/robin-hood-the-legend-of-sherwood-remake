"""Conservative exposed-rock source seeds; not a final appearance assignment."""
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw

R=Path(__file__).resolve().parents[3]
B=R/'level-editor/work/croisement03-refinement'
O=B/'restart2/bank-exposed-rock-proposal-v1'
# Source-inspected interior patches avoid the large central gold crown and ivy wall.
PATCHES={
 'left-upper-rock':[(159,209),(175,189),(190,180),(209,181),(233,186),(245,189),(238,201),(220,213),(204,229),(184,240),(167,232),(154,225)],
 'left-lower-shelf':[(166,237),(183,246),(215,237),(221,244),(210,253),(195,261),(178,262),(160,252)],
 'upper-rock-band':[(249,175),(291,175),(340,176),(383,178),(417,179),(464,175),(497,174),(528,178),(535,188),(526,195),(507,189),(480,184),(447,185),(405,189),(367,181),(320,181),(278,181),(252,185)],
}


def main():
 O.mkdir(exist_ok=True);assert not (O/'proposal.json').exists()
 lp=B/'baseline/Croisement03.rhp.json';sp=B/'baseline/covered.png';level=json.loads(lp.read_text());src=Image.open(sp).convert('RGB');masks={}
 for i,r in enumerate(level['masks']):
  im=Image.new('L',src.size);im.paste(Image.open(B/f'baseline/masks/{i:06}.png'),tuple(r['box_top_left']));masks[i]=np.array(im)>0
 native=masks[96]|masks[108]|masks[109];foreign=np.zeros(native.shape,bool)
 for i,m in masks.items():
  if i not in (96,108,109):foreign|=m
 final=np.zeros(native.shape,bool);records=[]
 for name,poly in PATCHES.items():
  im=Image.new('L',src.size);ImageDraw.Draw(im).polygon(poly,fill=255);trace=np.array(im)>0;accepted=trace&native&~foreign;final|=accepted
  records.append(dict(name=name,polygon=poly,traced_pixels=int(trace.sum()),candidate_pixels=int(accepted.sum()),foreign_mask_pixels=int((trace&foreign).sum()),outside_bank_mask_pixels=int((trace&~native).sum())))
 Image.fromarray(final.astype('uint8')*255).save(O/'candidate-rock-seeds.png')
 marked=np.array(src);marked[final]=(marked[final]*.45+np.array([0,230,255])*.55).astype('uint8');marked=Image.fromarray(marked);d=ImageDraw.Draw(marked)
 for name,poly in PATCHES.items():d.line(poly+[poly[0]],fill='white',width=1)
 crop=(130,155,630,370);sheet=Image.new('RGB',(1000,920),'#222222');d=ImageDraw.Draw(sheet)
 for i,(label,im) in enumerate([('Native source; foliage stays separate',src),('White source trace / cyan candidate rock pixels after all foreign-mask reservations',marked)]):
  sheet.paste(im.crop(crop).resize((1000,430),Image.Resampling.NEAREST),(0,i*460+30));d.text((4,i*460+8),label,fill='white')
 sheet.save(O/'source-seed-comparison.png')
 assert not np.any(final&foreign)
 record=dict(status='Conservative CPU source proposal; independent review and actual face ownership pending',
  source_hashes={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in (lp,sp)},patches=records,candidate_pixels=int(final.sum()),
  policy='Hand-traced exposed rock interiors intersect bank domains and reserve every overlapping foreign mask. This conservative reservation does not transfer ownership of the excluded pixels.',
  face_ownership_plan=['Only candidate source-facing bank/ramp first hits can receive these proposed rock samples; face normal alone is insufficient.',
   'All other bank source pixels remain unclassified/unknown until explicit rock, ivy, tree and ground partitions exist.',
   'Hidden rear, bottom and closure faces remain inferred and neutral.',
   'Separate thin source fragments from solid support if known foliage spans the bank; do not stretch canopy RGB across rock faces.',
   'Frozen Tree02/Tree03 shared crest and accepted source first hits remain mandatory blockers against any new face.'],
  limits=['Seed patches are intentionally incomplete; unmasked vegetation and temporal overlap still require source/frame checks.',
   'Native mask exclusivity is a reservation aid, not proof of material type or runtime ordering.',
   'No Blender object, gameplay obstacle, source RGBA, frozen review card or live asset was modified.'])
 (O/'proposal.json').write_text(json.dumps(record,indent=2)+'\n');assert sum(p.stat().st_size for p in O.iterdir())<2*1024**2;print('candidate rock seed pixels',int(final.sum()))


if __name__=='__main__':main()
