"""Freeze admission for the unchanged, bounded actor crossing diagnostic packet."""
import hashlib
import json
from pathlib import Path

ROOT=Path.cwd()
BASE=ROOT/'level-editor/work/croisement02-refinement/native-effect-production-oct12-v1'
PACKET=BASE/'actor-crossing-ui-v1'
OUT=BASE/'actor-crossing-admission-v1'
TEMPLATE=BASE/'full-map-baseline8-v5/admit.py'
TEMPLATE_SHA='b92e8a011a45c5660e9647be96fdb442a7ed97936b154905eb3b2777d0cb8e0d'
RUNNER_SHA='789b3fee3215026eb4692b3c65ea2988dfd90c24cbbf2ae845f2a278ac2096a0'

def sha(path):
    with path.open('rb') as file:return hashlib.file_digest(file,'sha256').hexdigest()

def main():
    assert sha(TEMPLATE)==TEMPLATE_SHA
    assert sha(PACKET/'run.mjs')==RUNNER_SHA
    manifests={name:sha(PACKET/name) for name in ['inputs.json','runtime-inputs.json','source-snapshot.json','runtime-authority.json','request.json']}
    helper='''
def verify_packet():
    packet = BASE / 'actor-crossing-ui-v1'
    def digest(path):
        with path.open('rb') as file:
            return hashlib.file_digest(file, 'sha256').hexdigest()
    for name, expected in PACKET_MANIFESTS.items():
        if digest(packet / name) != expected:
            raise RuntimeError('Frozen actor manifest drift: ' + name)
    snapshot = json.loads((packet / 'source-snapshot.json').read_text())
    authority = json.loads((packet / 'runtime-authority.json').read_text())
    if sorted(snapshot) != authority['membership']:
        raise RuntimeError('Frozen actor source membership drift')
    checked = {}
    for name in ['inputs.json', 'runtime-inputs.json']:
        for path, expected in json.loads((packet / name).read_text()).items():
            frozen = packet / 'snapshot' / path
            file = frozen if path in snapshot and frozen.exists() else ROOT / path
            if file not in checked:
                checked[file] = digest(file)
            if checked[file] != expected:
                raise RuntimeError('Frozen actor input drift: ' + path)
    if digest(Path(authority['index']['path'])) != authority['index']['initial_sha256']:
        raise RuntimeError('Frozen actor catalog drift')
    return len(checked)

'''
    text=TEMPLATE.read_text()
    start=text.index('RUNNERS = {');end=text.index('\n',start)
    text=text[:start]+"RUNNERS = {'actor': ('actor-crossing-ui-v1', '"+RUNNER_SHA+"')}\nPACKET_MANIFESTS = "+repr(manifests)+text[end:]
    text=text.replace('\ndef main():',helper+'\ndef main():')
    text=text.replace("    args = parser.parse_args()", "    parser.add_argument('--verify-only', action='store_true')\n    args = parser.parse_args()")
    text=text.replace("    args.evidence.mkdir(parents=True, exist_ok=False)","    count = verify_packet()\n    if args.verify_only:\n        print(json.dumps({'status': 'VERIFIED_NO_LAUNCH', 'files': count}))\n        return 0\n    args.evidence.mkdir(parents=True, exist_ok=False)")
    text=text.replace("        child = subprocess.Popen", "        verify_packet()\n        child = subprocess.Popen")
    OUT.mkdir(exist_ok=False)
    (OUT/'admit.py').write_text(text)
    request={'status':'PREPARED_NOT_LAUNCHED','packet':str(PACKET),'runner_sha256':RUNNER_SHA,'admission_sha256':sha(OUT/'admit.py'),'template_sha256':TEMPLATE_SHA,'manifest_pins':manifests,'scope':'Unchanged bounded receivers and old B05 private217 diagnostic; no corrected B05 or current texture-display fix. Same shared two-slot FIFO, browser lock and 10GiB reserve; complete packet checked before queue and after admission.'}
    (OUT/'request.json').write_text(json.dumps(request,indent=2)+'\n')
    print(json.dumps(request,indent=2))

if __name__=='__main__':main()
