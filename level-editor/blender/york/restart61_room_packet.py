"""Freeze one grouped-review card for the independent room cover and receivers."""
import hashlib,json,html
from pathlib import Path
from collections import deque
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2';OUT=W/'restart61-room-geometry-review-v1'
assert not OUT.exists()
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
model=W/'restart61-room-interior-v13/model.blend';digest=sha(model)
reports={}
for name,directory in [('source','restart61-room-source-v1'),('rays','restart61-room-interior-audit-v13'),('partition','restart61-room-final-audit-v13'),('winch','restart61-room-winch-contact-v13'),('gate','restart61-room-gate-contact-v13'),('tight','restart61-room-tight-v13'),('views','restart61-room-interior-contact-v13')]:
 p=W/directory/('source.json'if name=='source'else'report.json');reports[name]=json.loads(p.read_text())
 if name not in {'source','views'}:assert reports[name]['model_sha256']==digest
assert reports['views']['candidate_sha256']==digest
assert reports['rays']['initial_changed_pixels']==reports['rays']['applied_changed_outside_source_alpha']==0
assert reports['partition']['zero_area_faces']==0 and reports['partition']['max_partition_relative_area_error']<3e-6
assert reports['winch']['status']=='PASS_CONSERVATIVE_AABB_CLEARANCE'
assert reports['gate']['status']=='PASS_NO_CONFIRMED_GATE_CROSSINGS'
mask=Image.open(W.parent/'baseline/masks/000615.png').convert('L');q=deque([(40,150)]);seen=set(q)
while q:
 x,y=q.popleft()
 for xx,yy in [(x+1,y),(x-1,y),(x,y+1),(x,y-1)]:
  if 0<=xx<mask.width and 0<=yy<mask.height and not mask.getpixel((xx,yy)) and (xx,yy)not in seen:seen.add((xx,yy));q.append((xx,yy))
changed=json.loads((W/'restart61-room-interior-audit-v13/changed.json').read_text());assert {tuple(r['pixel'])for r in changed}=={(x+2353,y+783)for x,y in seen};assert len(seen)==5699
assert all(r['applied_owner'].startswith('Room inferred ')for r in changed)
images=[W/'restart61-room-tight-v13/comparison.png']
for state in ['initial','applied']:
 for mode in ['textured','solid']:
  folder=W/'restart61-room-interior-contact-v13'/state
  images.append(folder/('eight.png'if mode=='textured'else'solid-eight.png'))
  images.extend(folder/f'view-{i}-{mode}.png'for i in range(8))
 for mode in ['textured','solid']:images.append(W/'restart61-room-tight-v13'/state/f'native-{mode}.png')
for p in images:assert p.is_file()
OUT.mkdir()
report={'status':'GEOMETRY_REVIEW_PENDING','model':str(model.relative_to(ROOT)),'model_sha256':digest,'scope':'Four source-owned wall surface partitions (building-764/765/776/779) plus seven inferred room scenery solids. Gate, jamb and winch geometry are context only.','source_ownership':'Initial-only covers and always-visible remainders retain their original building source_node; projection_component distinguishes disjoint surfaces.','state':'Independent patch005: initial cover disappears immediately on apply; reset restores it. No transition or final sprite, no background integration. Independent of gate000 and winch004.','masks':{'old':[610,611,612,613],'new':[614,615,616,617]},'gameplay':'Patch005 has no door indices, sight-obstacle transitions or pathfinder changes. Preserve original collision metadata during future export. Never derive replacement gameplay footprints from cavity meshes. New room receivers are scenery only. Root owns integration.','inference':'Seven closed box receivers: floor, two side walls, three folded rear/return sections and left inner return. Hidden depth and 2-unit thickness inferred. Rear corner folds around the approved gate envelope. Exact bounds in winch audit.','appearance':'Known revealed source pixels projected only into the exact mask615 aperture on source-facing surfaces. Hidden gray surfaces await geometry approval and subsequent texture synthesis. Initial source comparison retains previously approved static texture appearance.','checks':{'native_rays':36120,'initial_changed':0,'applied_aperture_pixels':5699,'outside_changes':0,'aperture_pixel_set_exact':True,'gate_poses':45,'winch_poses':45,'winch_components_per_pose':109,'support_floor_max_gap':reports['winch']['support_floor_max_gap'],'partition_relative_area_error':reports['partition']['max_partition_relative_area_error']},'limits':'Source-ray preservation tolerance 0.005 world units; original wall presentation surfaces remain open. Gate check tests triangle crossings and gate-vertex containment; coplanar contact not a crossing. Mechanism tests do not certify every unrelated static wall.','review_layout':'Original camera top-left in all eight-view sheets. Tight crop shows patch005 independently, without winch overlay. Full context includes unchanged winch pose0, not the native absent initial winch.','source_manifest_sha256':reports['source']['native_manifest_sha256'],'files':{str(p.relative_to(ROOT)):sha(p)for p in images},'reports':reports}
(OUT/'handoff.json').write_text(json.dumps(report,indent=2)+'\n')
parts=['<!doctype html><meta charset="utf-8"><title>York room cover — geometry review</title><style>body{background:#192027;color:#eee;font:17px system-ui;max-width:1400px;margin:30px auto}img{max-width:100%;display:block;margin:20px 0}a{color:#9cd7ff}p{max-width:1050px;line-height:1.5}</style><h1>York: independent room cover and revealed interior</h1><p>Geometry approval pending. This single card covers the four split wall surfaces and seven inferred room receivers. Gate and winch are unchanged context.</p>']
for key in ['state','inference','appearance','gameplay']:parts.append('<p>'+html.escape(report[key])+'</p>')
parts.append('<h2>Original artwork and exact camera crop</h2><p>Covered source / covered model / revealed source / revealed model. This crop isolates the room; the winch is not overlaid.</p><img src="../restart61-room-tight-v13/comparison.png">')
for state,label in [('initial','Covered room'),('applied','Revealed room')]:
 parts.append('<h2>'+label+'</h2><p>Original camera at top-left. Unchanged approved winch pose 0 is included here only to show contact and clearance.</p>')
 for file in ['eight.png','solid-eight.png']:parts.append(f'<img src="../restart61-room-interior-contact-v13/{state}/{file}">')
parts.append('<p>Checks: initial geometry preserved across 36,120 camera rays; exactly 5,699 source-aperture pixels reveal room surfaces; no changes outside the patch. All 45 gate poses and all 45 winch poses checked. Gray hidden surfaces still need texture work after geometry approval.</p><p><a href="handoff.json">Frozen hashes, source ownership, inference bounds and audit reports</a></p>')
(OUT/'index.html').write_text('\n'.join(parts))
print(OUT)
