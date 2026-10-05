"""Freeze the reviewed arch-ring versus surrounding-wall source correction."""
import hashlib,json,shutil
from pathlib import Path
from PIL import Image,ImageChops,ImageFilter
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/york-refinement/restart2';SRC=BASE/'hall-source-authority-v1';OUT=BASE/'hall-source-authority-v2';PROPOSAL=BASE/'hall-crosspiece-study-v1/arch-authority-proposal-v2'
if not OUT.exists():shutil.copytree(SRC,OUT)
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
for state in ['initial-applied','applied-applied']:
 for role,new in [('scenery-york-great-hall-northwest-arch','arch'),('shell','shell')]:
  full=Image.open(PROPOSAL/f'{state}-{new}-full-trace.png').convert('L');known=full.filter(ImageFilter.MinFilter(3))
  full.save(OUT/f'{state}-{role}-full-trace.png');known.save(OUT/f'{state}-{role}.png');ImageChops.subtract(full,known).save(OUT/f'{state}-{role}-uncertain-boundary.png')
 group=Image.open(SRC/f'{state}-group-full-trace.png').convert('L');excluded=Image.open(PROPOSAL/f'{state}-keep-excluded-full-trace.png').convert('L');ImageChops.subtract(group,excluded).save(OUT/f'{state}-group-full-trace.png')
manifest=json.loads((SRC/'source-masks.json').read_text());manifest['mask_inventory']=str(OUT/'inventory.json')
for label,projection in manifest['projections'].items():
 for assignment in projection['assignments']:
  if label.endswith('-applied') and assignment['source_node'] in ['building-791','scenery-york-great-hall-northwest-arch']:
   assignment['review_evidence']='Root reviewed revised arch-ring/adjacent hall-wall boundary, with keep doorway and crown excluded; one-pixel uncertainty retained. Geometry first-hit checks still required.'
(OUT/'source-masks.json').write_text(json.dumps(manifest,indent=2)+'\n')
review=json.loads((SRC/'review.json').read_text());review['status']='Root scoped component ownership proposal PASS; new geometry and source coverage review still required';review['supersedes_source_manifest_sha256']=sha(SRC/'source-masks.json');review['component_revision']={'proposal_sha256':sha(PROPOSAL/'proposal.json'),'rule':'Arch reserves traced ring only;646 surplus right of x2797/y534 belongs surrounding hall wall; doorway/crown surplus excluded as keep appearance','source_projection_scope':'Only791 appearance changes authorized for the geometry control; no neighbor source substitution','root_review':'Both proposed overlays independently viewed and scoped PASS with1px uncertainty'};review['geometry_approval']='Prior batch-v3 remains archived;791 correction requires new grouped geometry approval';review['geometry_unchanged']=False
for state in review['states']:
 for row in state['roles']:
  role=row['role'];full=Image.open(OUT/f"{state['state']}-{role}-full-trace.png").convert('L') if role=='shell' else Image.open(OUT/f"{state['state']}-{role}-full-trace.png").convert('L');known=Image.open(OUT/f"{state['state']}-{role}.png").convert('L');row['full_trace_pixels']=sum(x>0 for x in full.get_flattened_data());row['confident_pixels']=sum(x>0 for x in known.get_flattened_data())
(OUT/'review.json').write_text(json.dumps(review,indent=2)+'\n')
for state in ['initial-initial','initial-applied','applied-initial','applied-applied']:
 a,c=state.split('-');source=Image.open(BASE/f'hall-cover-source-combinations-v1/patch001-{a}_patch002-{c}.png').convert('RGB').crop((2710,440,3060,840));domain=Image.open(OUT/f'{state}-group-full-trace.png').convert('L')
 Image.composite(Image.blend(source,Image.new('RGB',source.size,(30,220,100)),.4),source,domain).resize((1050,1200),Image.Resampling.NEAREST).save(OUT/f'{state}-source-domain-overlay.png')
# Validation must be renewed after replacing source boundaries.
(OUT/'contract-validation.json').write_text(json.dumps({'status':'PENDING validation for revised manifest','old_validation':str(SRC/'contract-validation.json')},indent=2)+'\n')
print(OUT)
