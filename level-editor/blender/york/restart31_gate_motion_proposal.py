"""Build a bounded private gate-motion hypothesis with explicit uncertain poses."""
import json,math
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];D=ROOT/'level-editor/work/york-refinement/restart2/gate-crossbar-tracking-v1';tracking=json.loads((D/'tracking.json').read_text());tail=json.loads((D/'tail-features.json').read_text());rows=[]
for r in tracking['rows']:
 i=r['frame']
 if i<=33:shift=r['full_fit']['shift_up_pixels'];confidence='Multiple visible timber features; whole-sprite matching corroborates extent within one pixel';bounds=[max(0,shift-1),shift+1]
 elif i==34:shift=55;confidence='Late tip matches settled frame shifted down two pixels; partial support16of25observed pixels';bounds=[54,56]
 elif i==36:shift=57;confidence='UNRESOLVED: no exposed tip below upper sliver. Nominal holds settled pose, not a measured displacement';bounds=[57,65]
 elif i==38:shift=56;confidence='Low confidence:11of15tip pixels match settled image shifted down one pixel';bounds=[55,58]
 else:shift=57;confidence='Settled tip coordinates unchanged; subset/opacity changes must not be converted automatically to whole-gate bounce';bounds=[56,58]if i in(35,37,40)else[57,57]
 rows.append({'frame':i,'clip_tick':2*i,'nominal_lift_source_pixels':shift,'nominal_lift_world_z':shift/math.cos(math.radians(35)),'source_pixel_interval':bounds,'confidence':confidence})
report={'status':'PRIVATE_MOTION_PROPOSAL_NOT_REVIEW_READY','approved_endpoints':{'initial_lift':0,'final_lift_source_pixels':57},'rows':rows,'conclusion':'Do not use bbox-derived65pixel overshoot as a physical claim. Frames35/37/40 preserve small subsets of settled wood at the same positions;39/42/43/44tipRGB and alpha agree exactly. Frame36 has no lower feature to measure.','source_appearance_requirement':'Retain authoritative native patch state/background composition separately; missing sprite pixels are not permission to delete or distort reusable timber geometry.','remaining':['Resolve frame36 with broader native context/occluder geometry before exporting a45pose clip.','Compare proposed all45poses with source and exact gatehouse context; no Blender work performed here.','No intermediate motion approval inferred from endpoint geometry/texture approvals.'],'not_changed':['Geometry','Texture','Runtime','Map','Catalog'],'late_tip_method_evidence':'tail-features.json'};(D/'motion-proposal.json').write_text(json.dumps(report,indent=2)+'\n');print('Private45pose CPU proposal with frame36 explicitly unresolved')
