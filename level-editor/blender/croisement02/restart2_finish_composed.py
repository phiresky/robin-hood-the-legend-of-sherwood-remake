"""Bind combined source metadata and finish independent native coverage on a saved composition."""
import argparse,json,sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from evidence_io import sha,write_json
from render_slots import acquire,release

def metadata(worker):
 proof=json.loads((worker/'inspection/crown-wood-composition.json').read_text());digest=sha(worker/'model.blend')
 if proof['model_sha256']!=digest or proof['crown_before']!=proof['crown_after'] or proof['wood_before']!=proof['wood_after']:raise ValueError('Composition failed exact preservation')
 wood=Path(proof['wood_worker']);crown=Path(proof['crown_worker']);report=json.loads((crown/'inspection/refinement.json').read_text());wood_report=json.loads((wood/'inspection/refinement.json').read_text())
 for k in ('wood_domain_mask','coverage_domain_mask'):
  if k in wood_report:report[k]=wood_report[k]
 report.update(model_sha256=digest,composition=proof,status='Private exact crown and scoped wood composition; independent joint review pending');path=worker/'inspection/refinement.json'
 if path.exists():raise FileExistsError(path)
 write_json(path,report)

def main():
 p=argparse.ArgumentParser();p.add_argument('workspace',type=Path);a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);w=a.workspace.resolve();metadata(w);digest=sha(w/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));bpy.context.view_layer.update();objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==w.name]
 from source_coverage import audit
 audit(w,objects,transparent_bounces=256)
 if sha(w/'model.blend')!=digest:raise ValueError('Model changed')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
