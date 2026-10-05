"""Reconcile exact post-cap candidate, accepted roles and current neighbor evidence."""
import json,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,scenery_workspace
from evidence_io import sha,write_json

def main():
 asset='croisement02-east-upright-rail-fence-95';worker=scenery_workspace(asset);digest=sha(worker/'model.blend');assert digest=='dde0fc7944ea5bb4ce3613edc401f506c3ee189210f13427207adaf5c81cb414'
 package=worker.parents[1];delta=OUT/'restart2-vegetation/source-role-delta-v3/delta.json';roles=json.loads(delta.read_text());assert roles['receiver_model_sha256']==digest and roles['new_domain']==6009 and roles['pixels']==8
 for p,h in roles['evidence'].items():assert sha(Path(p))==h,p
 joint=OUT/'restart2-wood/tree38-combined-neighbour-v2/ddee5ea6-9eef5da2-dde0fc79';report=json.loads((joint/'evidence.json').read_text());expected={'ddee5ea699247386c980fa2b3ad89542411c8246c95643ce034639563c0064ab','9eef5da2ec8e661d1094dd3cb5605fa6d76dd0f5185c008f481aa2a0e98a076e',digest};assert {r['model_sha256'] for r in report['workers']}==expected
 assert report['native_first'] and report['context_world_transforms_verified']
 for r in report['workers']:assert sha(Path(r['path'])/'model.blend')==r['model_sha256']
 assert sha(joint/'sheet.png')==report['sheet_sha256'];assert sha(joint/'source-comparison.png')==report['source_comparison_sha256']
 paths=[package/'root-review.json',package/'source-comparison/evidence.json',worker/'validation.json',worker/'inspection/reopened-preservation.json',worker/'inspection/actual-materials/sheet.png',delta,joint/'evidence.json',joint/'sheet.png',joint/'source-comparison.png',joint/'contact-sheet.png']
 dest=OUT/'restart2-fence/fence95-readiness-v1.json';assert not dest.exists()
 write_json(dest,dict(status='Technical packet reconciled; current joint visual verdict and fresh user approval required',asset_id=asset,worker=str(worker),model_sha256=digest,files={str(p):sha(p) for p in paths},standalone_root_status='Scoped cap geometry/source-role PASS',source_domain=6009,neighbor_hashes=sorted(expected),limitations=['Cap6/7 fully covered, remaining1558,725 alpha32/255 accepted as contour antialiasing; existing1604,706 covered.','Current foliage75 boundary26/28 full, two partial alpha64/255 and96/255 retained honestly.','Rear faces remain gray pending fresh exact-geometry approval and texture fill.','No inherited user approval, API request, selector or canonical mutation.']))
 print(dest)
if __name__=='__main__':main()
