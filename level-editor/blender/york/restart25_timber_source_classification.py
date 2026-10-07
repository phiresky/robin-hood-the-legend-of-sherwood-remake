"""Retain every missing target and classify source evidence without changing shape."""
import json,hashlib
from pathlib import Path
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';OUT=WORK/'timber-sawn-source-review-v1/source-classification.json'
if OUT.exists():raise FileExistsError(OUT)
plan=json.loads((WORK/'timber-sawn-end-plan-v1/plan.json').read_text());source=WORK/'pair-v14/assets/york-southwest-square-west-house/reference/source.png';art=Image.open(source).convert('RGB')
# Native grid review: this brown row belongs to the crossing beam above the pale board.
beam_row={(1269,938),(1270,938),(1271,938)}
# These bright blue pixels continue directly into the large exposed ground patch.
foreign={(1304,952),(1303,953),(1302,954),(1300,955),(1299,956),(1298,957)}
rows=[]
for r in plan['missing']:
 p=tuple(r['pixel']);e,a=r['expected'],r['actual']
 if a is None:status='SILHOUETTE_MISS_RETAIN';why='Observed edge remains outside finite sawn solid; do not bend ends to force fit.'
 elif e[0]==a[0]:status='SAME_WOOD_FACE_ATTRIBUTION';why='Contiguous wood of the same piece; top-versus-side trace is within the native boundary ambiguity. Reproject only after face-attribution review.'
 elif p in beam_row:status='SOURCE_OWNER_CORRECTION_PROPOSED';why='Independent native row is brown beam wood; the cream board starts on the following row. Proposed long-crossing side owner.'
 else:status='DIFFERENT_PIECE_RAY_CONFLICT_RETAIN';why='Keep independently traced source owner; actual first-hit piece is not evidence for reassignment. Needs local allocation resolution.'
 rows.append({**r,'rgb':art.getpixel(p),'classification':status,'reason':why})
blue=json.loads((WORK/'timber-sawn-source-review-v1/blue-side-candidates.json').read_text())['pixels']
for r in blue:
 clear=tuple(r['pixel'])in foreign;r['classification']='FOREIGN_GROUND_PROPOSED_EXCLUSION'if clear else'BOUNDARY_OR_SHADED_WOOD_UNRESOLVED';r['reason']='Exposed blue field continuous beyond dark wood edge, independently inspected in native grid.'if clear else'Blue/dark color alone is insufficient; shaded wood, antialiasing and ground boundary remain distinct possibilities.'
report={'status':'PRIVATE_SOURCE_ALLOCATION_PROPOSAL_NO_MODEL_CHANGE','model_sha256':'0d877de4b63cbbe805ffe12c638e419e79a60dc9cc2a65efb1668447ddf08986','source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'targets_retained':973,'missing_target_count':len(rows),'missing_targets':rows,'accepted_blue_side_review':blue,'counts':{'silhouette_miss':3,'same_piece_face_attribution':10,'proposed_beam_owner_correction':3,'unresolved_other_piece_conflict':28,'clear_foreign_ground_proposed_exclusions':6,'unresolved_blue_or_dark_side_pixels':14},'limitations':['No model, source domain, texture or approval changed.','All973 old targets retained; proposed foreign corrections are explicitly recorded, never silently removed.','Color filter identifies20 candidates only; it is not a complete dark-side ownership audit.','Finite rectangular sawn geometry is fixed. Remaining source discrepancies require allocation work, not silhouette distortion.']}
assert len(rows)==44 and len(blue)==20
OUT.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report['counts']))
