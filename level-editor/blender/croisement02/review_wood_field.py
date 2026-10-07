"""Run saved source/ground guards before the complete scoped wood review."""
import argparse,json,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from evidence_io import sha

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--tree',type=int,choices=[32,38],required=True)
    parser.add_argument('--worker',type=Path,required=True);parser.add_argument('--preservation-base',type=Path,required=True)
    parser.add_argument('--review-name',required=True);parser.add_argument('--domain-review',type=Path)
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);worker=args.worker.resolve()
    digest=sha(worker/'model.blend');record=json.loads((worker/'evidence.json').read_text())
    if digest!=record['model_sha256'] or not record['partition_preserves_world_faces'] or record['internal_caps_added']!=0:raise ValueError('Saved source/geometry evidence invalid')
    acquire();original_args=sys.argv
    try:
        if args.tree==32:import audit_tree32_roots as audit
        else:import audit_tree38_contour as audit
        sys.argv=[original_args[0],'--','--worker',str(worker),'--preservation-base',str(args.preservation_base),'--ground-occlusion']
        if args.domain_review:sys.argv.extend(['--domain-review',str(args.domain_review)])
        audit.main(release_slot=False)
        path=worker/'inspection/root-source-coverage-ground/report.json';coverage=json.loads(path.read_text())
        measures={key:coverage[key] for key in ['source_coverage','root_source_coverage','interface_source_coverage']}
        if 'extension_coverage' in coverage:measures['extension_coverage']=coverage['extension_coverage']
        if min(measures.values())<.95:raise ValueError('Saved source/ground coverage failed: '+str(measures))
        import inspect_tree07_base
        sys.argv=[original_args[0],'--','--mask',str(args.tree),'--model',str(worker/'model.blend'),'--output-name',args.review_name,'--target-z','75' if args.tree==32 else '70','--scale','220' if args.tree==32 else '200']
        inspect_tree07_base.main()
        if sha(worker/'model.blend')!=digest:raise ValueError('Review changed saved model')
        receipt=dict(status='Saved source guards passed; actual/solid self-review still required',model_sha256=digest,coverage=measures,coverage_report=str(path),coverage_sha256=sha(path),gallery_ready=False)
        (worker/'inspection/field-review-guards.json').write_text(json.dumps(receipt,indent=2)+'\n')
    finally:sys.argv=original_args;release()

if __name__=='__main__':main()
