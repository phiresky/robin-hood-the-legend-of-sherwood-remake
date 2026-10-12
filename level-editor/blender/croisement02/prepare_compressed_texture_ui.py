"""Freeze the focused mipmap fix over the previous full-map diagnostic runtime."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

ROOT=Path.cwd()
BASE=ROOT/'level-editor/work/croisement02-refinement/native-effect-production-oct12-v1'
OLD=BASE/'full-map-baseline8-v4'
OUT=BASE/'full-map-baseline8-v5'
COMMIT='3835cf3f7'
FILES=['level-editor/app/src/texture-display.ts','level-editor/app/src/texture-display.test.ts']

def sha(path):
    with path.open('rb') as file:return hashlib.file_digest(file,'sha256').hexdigest()
def read(path):return json.loads(path.read_text())
def dump(path,value):path.write_text(json.dumps(value,indent=2)+'\n')
def relocate(text):return text.replace(str(OLD),str(OUT)).replace(str(OLD.relative_to(ROOT)),str(OUT.relative_to(ROOT)))

def main():
    OUT.mkdir(exist_ok=False)
    shutil.copytree(OLD/'snapshot',OUT/'snapshot')
    for path in OLD.iterdir():
        if path.is_file():
            (OUT/path.name).write_text(relocate(path.read_text()))
    snapshot=read(OUT/'source-snapshot.json')
    for name in FILES:
        path=OUT/'snapshot'/name
        path.write_bytes(subprocess.check_output(['git','show',f'{COMMIT}:{name}'],cwd=ROOT))
        snapshot[name]=sha(path)
    previous=read(OLD/'source-snapshot.json')
    assert set(snapshot)==set(previous)
    assert sorted(k for k in snapshot if snapshot[k]!=previous[k])==sorted(FILES)
    dump(OUT/'source-snapshot.json',snapshot)
    runtime=read(OUT/'runtime-inputs.json');runtime.update(snapshot)
    module=OUT/'compressed-texture-proof.mjs'
    shutil.copy2(ROOT/'level-editor/blender/croisement02/compressed_texture_ui_proof.mjs',module)
    runtime[str(module)]=sha(module)
    dump(OUT/'runtime-inputs.json',runtime)
    check=OUT/'check.js';text=check.read_text()
    marker=" const mode=[...document.querySelectorAll('.editor-modes button')]"
    assert text.count(marker)==1
    text=text.replace(marker," await (await import('/@fs"+str(module)+"')).proveCompressedTextureModes({v,check,wait,digest});\n"+marker)
    text=text.replace('checks,states,actorCrossing:', 'checks,states,compressedTextureModes:window.compressedTextureModeProof,actorCrossing:')
    check.write_text(text)
    runner=OUT/'run.mjs';text=runner.read_text()
    marker=" await writeFile(join(out,'browser-errors.json'),JSON.stringify(browserErrors,null,2));"
    assert text.count(marker)==1
    text=text.replace(marker," const stderr=await readFile(join(out,'chrome-stderr-live.log'),'utf8');const unsupported=stderr.split('\\n').filter(line=>/glGenerateMipmap.*Texture format does not support mipmap generation/.test(line));await writeFile(join(out,'gpu-mipmap-check.json'),JSON.stringify({unsupportedCalls:unsupported.length,lines:unsupported}));if(unsupported.length)throw Error('Unsupported compressed mipmap generation recorded');\n"+marker)
    runner.write_text(text)
    # Same admission implementation and FIFO pool, separate variant table from the live v4 experiment.
    admission=(ROOT/'level-editor/blender/croisement02/run_full_map_admitted.py').read_text()
    admission=admission.replace('ROOT = Path(__file__).resolve().parents[3]','ROOT = Path('+repr(str(ROOT))+')')
    start=admission.index('RUNNERS = {');end=admission.index('\n}\n',start)+2
    admission=admission[:start]+"RUNNERS = {'fixed8': ('full-map-baseline8-v5', '"+sha(runner)+"')}"+admission[end:]
    (OUT/'admit.py').write_text(admission)
    pins=read(OUT/'inputs.json')
    for path in OUT.iterdir():
        if path.is_file() and path.name not in {'inputs.json','request.json'}:pins[str(path)]=sha(path)
    dump(OUT/'inputs.json',pins)
    # Verify the actually served frozen source and every canonical target resource.
    for path,expected in runtime.items():
        file=Path(path)
        if not file.is_absolute():file=OUT/'snapshot'/path if (OUT/'snapshot'/path).exists() else ROOT/path
        assert sha(file)==expected,path
    assert read(OUT/'canonical-resource-pins.json')==read(OLD/'canonical-resource-pins.json')
    request={'status':'PREPARED_VERIFIED_NOT_LAUNCHED','baseline':str(OLD),'focused_commit':COMMIT,'changed_runtime_files':FILES,'run_sha256':sha(runner),'admission_sha256':sha(OUT/'admit.py'),'source_snapshot_sha256':sha(OUT/'source-snapshot.json'),'runtime_pins_verified':len(runtime),'canonical_resources':len(read(OUT/'canonical-resource-pins.json')),'scope':'Same full canonical scene and B05 diagnostic as v4; actual texture UI modes and GPU error checks added. No appearance approval.'}
    dump(OUT/'request.json',request)
    print(json.dumps(request,indent=2))

if __name__=='__main__':main()
