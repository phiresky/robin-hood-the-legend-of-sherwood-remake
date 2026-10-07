"""Resolve source ownership at pixel footprints on fixed sawn timber geometry."""
import ast,json,math
from pathlib import Path
import numpy as np
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';OUT=WORK/'timber-subpixel-allocation-v1'
if OUT.exists():raise FileExistsError(OUT)
code=Path(__file__).with_name('restart22_timber_corrected_plan.py').read_text();exec(compile(ast.Module(body=[n for n in ast.parse(code).body if isinstance(n,ast.FunctionDef)],type_ignores=[]),'<fixed geometry source rays>','exec'))
s=math.sin(math.radians(35));c=math.cos(math.radians(35));back=np.array([0,-c,s]);faces=[(0,3,2,1),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)]
plan=json.loads((WORK/'timber-sawn-end-plan-v1/plan.json').read_text());parts=plan['pieces'];rows=plan['missing'];offsets=[((i+.5)/8-.5,(j+.5)/8-.5)for j in range(8)for i in range(8)]
actual=ray_owners(parts,[[r['pixel'][0]+dx,r['pixel'][1]+dy]for r in rows for dx,dy in offsets]);report=[]
for i,r in enumerate(rows):
 values=actual[i*64:(i+1)*64];exact=[off for off,o in zip(offsets,values)if o is not None and tuple(o)==tuple(r['expected'])];same=[off for off,o in zip(offsets,values)if o is not None and o[0]==r['expected'][0]]
 report.append({**r,'expected_face_subpixel_count':len(exact),'expected_piece_subpixel_count':len(same),'expected_face_offsets':exact,'classification':'EXACT_FACE_PRESENT_IN_PIXEL_FOOTPRINT'if exact else'SAME_PIECE_OTHER_FACE_PRESENT'if same else'EXPECTED_PIECE_ABSENT_FROM_PIXEL_FOOTPRINT'})
OUT.mkdir();(OUT/'report.json').write_text(json.dumps({'status':'CPU_FIXED_GEOMETRY_SUBPIXEL_DIAGNOSTIC','sample_grid':8,'rows':report,'limits':['No geometry or material changed.','Subpixel source contribution is valid only where that piece is genuinely first visible; coverage is not permission to copy to the wrong receiver.','Saved float32 geometry verification and render still required.']},indent=2)+'\n');print(json.dumps({k:sum(r['classification']==k for r in report)for k in sorted(set(r['classification']for r in report))}))
