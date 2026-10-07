"""Run a hash-bound staged Editor and terrain preview proof under the shared lease."""
import argparse,hashlib,json,subprocess,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release

def sha(path):
    if not path.exists():return None
    with path.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
def snapshot(stage,fixture):
    candidate=json.loads((stage/'candidate.json').read_text())
    manifest=json.loads((stage/'promotion-draft.json').read_text())
    assert not candidate['inherited_derivative_holds']
    paths={stage/name for name in ['candidate.json','promotion-draft.json','promotion-library-index.json','croisement02.rhlos-map.json']}
    for row in manifest['files']:
        source,target=Path(row['source']),Path(row['target'])
        assert sha(source)==row['source_sha256']
        assert (sha(target)if target.exists()else None)==row['previous_sha256']
        paths.add(source)
        if target.exists():paths.add(target)
    for row in manifest['protected_files']:
        path=Path(row['path']);assert sha(path)==row['sha256'];paths.add(path)
    for directory in [stage/'map-assets',ROOT/'level-editor/app/src',ROOT/'level-editor/shared/src',ROOT/'level-editor/library/game-data']:
        paths.update(path for path in directory.rglob('*')if path.is_file())
    paths.update(fixture/name for name in ['editor-review.html','editor-review.tsx','runner.mjs'])
    paths.update([ROOT/'level-editor/app/vite.config.ts',Path(__file__)])
    return {str(path):sha(path)for path in sorted(paths)}
def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('stage',type=Path);parser.add_argument('--fixture-name',default='editor-review');args=parser.parse_args();stage=args.stage.resolve();assert '/'not in args.fixture_name and args.fixture_name.startswith('editor-review');fixture=stage/args.fixture_name;assert not(fixture/'runtime').exists()
    acquire()
    try:
        pins=snapshot(stage,fixture);(fixture/'prelaunch-pins.json').write_text(json.dumps(pins,indent=2)+'\n')
        subprocess.run(['node',str(fixture/'runner.mjs')],cwd=ROOT,check=True)
        assert snapshot(stage,fixture)==pins,'Inputs changed during staged proof'
        output=fixture/'runtime';result=json.loads((output/'result.json').read_text());terrain=json.loads((output/'terrain-preview-proof.json').read_text());assert result['status'].startswith('PASS')and terrain['status'].startswith('PASS')
        repair=json.loads((stage/'candidate.json').read_text())['terrain_preview_repair'];assert terrain['models']['terrain-preview']['sha256']==repair['output_sha256'];assert terrain['models']['terrain-lossy']['sha256']==repair['source_sha256']
        served=[json.loads(line)for line in(output/'served-overlay.jsonl').read_text().splitlines()];served_paths={row['request']for row in served};assert all(any(f'3d-assets/croisement02/croisement02-tree-{n}/{name}.glb'in served_paths for name in ['model','lossy'])for n in [18,24,38,39,45]);assert '3d-assets/croisement02/croisement02-terrain/preview.glb'in served_paths
        evidence={str(output/name):sha(output/name)for name in ['result.json','loaded.png','rotated.png','terrain-preview.png','terrain-lossy.png','terrain-preview-proof.json','actual-runtime-gameplay.json','process-final.json','served-overlay.jsonl','overlay-preflight.json']}
        proof={'status':'PASS staged production Editor and exact terrain preview/source load; independent image review pending','stage':str(stage),'candidate_sha256':sha(stage/'candidate.json'),'prelaunch_pins_sha256':sha(fixture/'prelaunch-pins.json'),'evidence':evidence,'canonical_writes':False}
        (fixture/'verification.json').write_text(json.dumps(proof,indent=2)+'\n');print('PASS staged Editor and terrain preview; exact inputs unchanged',flush=True)
    finally:release()
if __name__=='__main__':main()
