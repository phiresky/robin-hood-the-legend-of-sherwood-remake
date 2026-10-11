"""Advance private own-source candidates through exact surfaces, contacts and import review."""
import argparse
import hashlib
import json
import subprocess
from pathlib import Path

ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/croisement02-refinement'
RECIPES=Path(__file__).parent
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--indices',type=int,nargs='+',required=True);parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();args.output.mkdir(exist_ok=False);results=[]
    def run(name,command):
        log=args.output/f'{name}.log'
        with log.open('x') as stream:subprocess.run(command,cwd=ROOT,stdout=stream,stderr=subprocess.STDOUT,check=True)
        print(name,'finished',flush=True)
    for number in args.indices:
        assert number in range(2,7)
        fit=BASE/f'restart78-butterfly{number:02}-own-poses-v1/fit.json';assert fit.is_file(),f'Own-source fit still running: {fit}'
        atlas=BASE/f'restart80-butterfly{number:02}-observations-v1'
        if atlas.exists():assert json.loads((atlas/'report.json').read_text())['fit_sha256']==sha(fit)
        else:run(f'{number:02}-surfaces',['python3',str(RECIPES/'restart80_butterfly_own_observations.py'),'--fit',str(fit),'--output',str(atlas)])
        contact=BASE/f'restart83-butterfly{number:02}-contact-candidate-v1'
        run(f'{number:02}-contacts',['python3',str(RECIPES/'restart83_butterfly_contact_candidate.py'),'--fit',str(fit),'--output',str(contact),'--run'])
        proof=json.loads((contact/'contacts/report.json').read_text());hits={key:row['contact_counts'] for key,row in proof['results'].items() if row['contact_counts']}
        row=dict(butterfly=number,source_fit_sha256=sha(fit),contacts_sha256=sha(contact/'contacts/report.json'),contact_witness_groups=len(hits),canonical_writes=False)
        if hits:
            row.update(status='CONTACT_REFINEMENT_REQUIRED',contacts=hits);results.append(row)
            (args.output/'progress.json').write_text(json.dumps(results,indent=2)+'\n');print(number,'contact hold',len(hits),flush=True);continue
        exported=BASE/f'restart84-butterfly{number:02}-fullpath-v1';model=exported/f'butterfly{number:02}-fullpath.glb'
        run(f'{number:02}-export',['python3',str(RECIPES/'restart84_export_own_butterfly.py'),'--fit',str(contact/'fit.json'),'--atlas',str(atlas),'--depth',str(contact/'inputs.json'),'--output',str(exported)])
        readback=BASE/f'restart85-butterfly{number:02}-readback-v1'
        run(f'{number:02}-readback',['python3',str(RECIPES/'restart85_verify_own_butterfly.py'),'--model',str(model),'--fit',str(contact/'fit.json'),'--atlas',str(atlas),'--depth',str(contact/'inputs.json'),'--output',str(readback)])
        imported=BASE/f'restart86-butterfly{number:02}-import-review-v1'
        run(f'{number:02}-import',['/usr/bin/blender','--background','--factory-startup','--threads','2','--python-exit-code','1','--python',str(RECIPES/'restart86_review_own_butterfly_import.py'),'--','--model',str(model),'--fit',str(contact/'fit.json'),'--output',str(imported)])
        row.update(status='PRIVATE_RENDERED_CANDIDATE_NEEDS_SELF_AND_USER_REVIEW',model_sha256=sha(model),import_report_sha256=sha(imported/'report.json'),readback_sha256=sha(readback/'readback.json'))
        results.append(row);(args.output/'progress.json').write_text(json.dumps(results,indent=2)+'\n')
    (args.output/'report.json').write_text(json.dumps(dict(status='PRIVATE_CANDIDATE_BATCH_COMPLETE_NOT_APPROVAL',results=results,limits=['Do not equate contact pass or rendered output with visual acceptance.','Static receiver tests exclude moving states and gameplay composition.','All geometry, depth and appearance proposals remain unapproved.']),indent=2)+'\n')


if __name__=='__main__':main()
