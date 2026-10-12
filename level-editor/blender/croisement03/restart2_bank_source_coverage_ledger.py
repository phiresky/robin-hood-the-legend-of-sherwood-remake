"""Independently account for the complete reviewed bank pixel domain after projection."""
import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path

import numpy as np
from PIL import Image


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def pixels(path):
    return {(int(x), int(y)) for y, x in np.argwhere(np.array(Image.open(path)) > 0)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('folder', type=Path)
    args = parser.parse_args()
    folder = args.folder.resolve()
    base = folder.parent
    out = folder / 'independent-source-coverage-v1'
    assert not out.exists()
    proofpath = folder / 'projection.json'
    proof = json.loads(proofpath.read_text())
    assert sha(folder / 'worker.blend') == proof['model_sha256']
    oldpath = base / 'bank-main-masses-source-v1/projection.json'
    oldproof = json.loads(oldpath.read_text())
    protected = pixels(base / 'bank-exposed-rock-proposal-v1/candidate-rock-seeds.png')
    previous = {tuple(p) for p in oldproof['accepted_pixels']}
    fullpath = base / 'bank-full-visible-stone-audit-v3/additional-interior-proposal.png'
    residualpath = base / 'bank-residual-approved-selection-v1/exact222-private-trial-mask.png'
    broad = pixels(fullpath)
    residual = pixels(residualpath)
    assert len(protected) == 3446 and len(previous) == 7811
    assert len(broad) == 17773 and len(residual) == 222
    domains = {'protected': protected, 'prior_interiors': previous,
               'broader_interiors': broad, 'selected_residual': residual}
    expected = set().union(*domains.values())
    assert sum(map(len, domains.values())) == len(expected) == 29252
    accepted = {tuple(p) for p in proof['accepted_pixels']}
    oldhits = {(r[0], r[1]) for r in proof['protected_firsthits']}
    assert oldhits == protected
    assert len(accepted) == proof['accepted']
    assert not (accepted & protected)
    assigned = oldhits | accepted
    assert protected | previous <= assigned
    assert assigned <= expected, sorted(assigned - expected)
    rejected = {}
    for face in proof['faces']:
        for x, y, receiver in face['ray_rejected']:
            rejected.setdefault((x, y), []).append({'region': face['id'], 'first_receiver': receiver})
    assert set(rejected) == expected - assigned, {
        'unaccounted': sorted(expected - assigned - rejected.keys()),
        'unexpected_rejection': sorted(rejected.keys() - (expected - assigned))}
    actual = proof['full_source_receiver_hits']
    assert {(r['source_xy'][0], r['source_xy'][1]) for r in actual} == assigned & (broad | residual)
    owner_counts = Counter((r['proposal_region'], r['saved_preview_receiver']) for r in actual)
    sourcepath = base.parent / 'baseline/covered.png'
    assert sha(sourcepath) == proof['source_sha256']
    source = Image.open(sourcepath).convert('RGBA')
    overlay = np.array(source)
    # This is a review overlay only; the source image and model remain immutable.
    for x, y in expected:
        color = (0, 220, 240) if (x, y) in assigned else (255, 0, 100)
        overlay[y, x, :3] = color
    out.mkdir()
    Image.fromarray(overlay).save(out / 'coverage-overlay.png')
    records = [{'source_xy': list(xy), 'rejections': rejected[xy]} for xy in sorted(rejected)]
    receipt = {'status': 'EXACT_REVIEWED_DOMAIN_ACCOUNTED_NOT_FINAL_OWNERSHIP',
        'pins': {str(p): sha(p) for p in [proofpath, oldpath, fullpath, residualpath, sourcepath]},
        'model_sha256': proof['model_sha256'], 'expected': len(expected), 'assigned': len(assigned),
        'rejected': len(rejected), 'domains': {name: {'proposed': len(domain),
            'assigned': len(domain & assigned), 'rejected': len(domain - assigned)} for name, domain in domains.items()},
        'receiver_counts': [{'region': region, 'receiver': owner, 'count': count}
            for (region, owner), count in sorted(owner_counts.items())],
        'rejected_pixels': records,
        'limits': ['This checks the full reviewed domain, not the entire visible stone domain.',
            'Current bank receivers remain geometric hypotheses; explicit102 disagreements remain unresolved.',
            'Twelve prior/native overlaps still require joined priority and state provenance.',
            'Unreviewed boundaries and unresolved source pixels are not background or approved fill.']}
    (out / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps({k: receipt[k] for k in ['status', 'expected', 'assigned', 'rejected', 'domains']}))


if __name__ == '__main__':
    main()
