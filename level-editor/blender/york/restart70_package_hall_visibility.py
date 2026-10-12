"""Package a private runtime proposal; never apply it to shared editor sources."""
import difflib
import hashlib
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
W = ROOT / 'level-editor/work/active/york/restart2'
OUT = W / 'restart70-hall-visibility-private-v1'
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
old = OUT / 'baseline'
new = OUT / 'mirror'
paths = sorted(str(p.relative_to(old)) for p in old.rglob('*.ts'))
paths += ['level-editor/shared/src/source-cover-visibility.ts', 'level-editor/app/src/source-cover-visibility.test.ts']
patch = []
pins = {}
for rel in paths:
    src, dst = old / rel, new / rel
    if src.exists():
        assert sha(src) == sha(ROOT / rel), f'Shared source changed: {rel}'
        pins[rel] = sha(src)
    else:
        assert not (ROOT / rel).exists(), f'New shared file already exists: {rel}'
    patch.extend(difflib.unified_diff(src.read_text().splitlines(True) if src.exists() else [], dst.read_text().splitlines(True), fromfile='a/' + rel if src.exists() else '/dev/null', tofile='b/' + rel))
(OUT / 'runtime-proposal.patch').write_text(''.join(patch))
(OUT / 'baseline-pins.json').write_text(json.dumps(pins, indent=2) + '\n')
# Immutable approved staged resources are linked, avoiding another model copy.
base = W / 'restart69-hall-approved-delivery-v1/library'
for src in base.rglob('*'):
    if src.is_file():
        dst = OUT / 'library' / src.relative_to(base)
        dst.parent.mkdir(parents=True, exist_ok=True)
        if not dst.exists(): os.link(src, dst)
contract = OUT / 'contract.proposed.json'
report = {
    'status': 'PRIVATE_IMPLEMENTATION_CPU_VERIFIED_ROOT_RELEASE_REQUIRED',
    'shared_source_baselines': pins,
    'patch_sha256': sha(OUT / 'runtime-proposal.patch'),
    'contract_sha256': sha(contract),
    'tests': {'passed': 73, 'failed': 0, 'log_sha256': sha(OUT / 'test-results.txt'), 'typescript_no_emit': True},
    'appearance_authority': 'Existing Editor3D appearance checkbox -> viewport.setPatchRevealed -> PatchDisplay -> actual remapped hall mesh/material; same state selects artwork and effect visibility.',
    'effect_visibility': 'Patch002 applied only. Patch001 changes its own cover artwork but never controls map-animation000.',
    'clock': 'Map-animation active/play/tick remain independent; hidden loop continues advancing. No new patch UI or cursor.',
    'native_artwork': json.loads((OUT / 'native-cover-artwork.json').read_text()),
    'lifetime': 'Source record hashes, placement model/descriptor, appearance mapping and transform verified. Invalid binding fails closed. Clear/disposal retire clock and artwork ownership.',
    'proof_scope': '73 CPU tests including real viewport setter, original source pixels for all four cover combinations and reset, overlay composition, advancing tick, invalid source and retired lifetime. No browser proof claimed.',
    'remaining_release_dependencies': [
        'Root explicitly releases shared runtime freeze and reviews/applies proposal after rechecking source baseline hashes.',
        'Root performs real normal-editor plus source-art-window initial/applied/reset browser proof using installed exact hall model and source-pinned resources.',
        'Root stages proposed contract and artwork with exact hashes, updates catalog only after required proof; restart69 ungated contract must not be published.',
    ],
    'canonical_writes': False, 'shared_runtime_writes': False, 'browser_run': False, 'resynthesis': False,
}
(OUT / 'handoff.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps({'patch_sha256': report['patch_sha256'], 'handoff_sha256': sha(OUT / 'handoff.json'), 'paths': len(paths)}))
