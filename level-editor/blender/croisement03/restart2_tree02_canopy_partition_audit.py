"""Read-only temporal source partition audit before private Tree02 construction."""
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
B = ROOT / 'level-editor/work/croisement03-refinement/restart2'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    out = B / 'tree02-canopy-partition-audit-v1'
    out.mkdir(exist_ok=False)
    receipt = B / 'tree02-bark-proposal-v1/root-source-classification.json'
    accepted = json.loads(receipt.read_text())
    for name, expected in accepted['evidence'].items():
        assert sha(Path(name)) == expected, name
    own = np.array(Image.open(B / 'tree02-bark-proposal-v1/proposed-bark.png')) > 0
    neighbor = np.array(Image.open(B / 'tree03-bark-proposal-v1/proposed-bark.png')) > 0
    assert own.sum() == 377 and not np.any(own & neighbor)
    prior = B / 'tree03-canopy-fragment-source-v1/scope.json'
    scope = json.loads(prior.read_text())
    assert scope['absolute_interval'] == [225, 0, 435, 175]
    rows = []
    union = None
    for row in scope['frames']:
        src = Path(row['source_path'])
        assert sha(src) == row['source_sha256']
        rgba = np.array(Image.open(src).convert('RGBA'))
        fragment = Path(prior.parent / f"{row['frame']:03}.png")
        assert sha(fragment) == row['fragment_sha256']
        assert np.array_equal(rgba[:175, 225:435], np.array(Image.open(fragment)))
        alpha = rgba[:, :, 3] > 0
        union = alpha.copy() if union is None else union | alpha
        h, w = alpha.shape
        own_local = own[:h, :w]
        assigned03 = np.zeros(alpha.shape, bool)
        assigned03[:175, 225:435] = alpha[:175, 225:435]
        # This is a candidate non-overlapping construction interval, not tree membership.
        candidate = np.zeros(alpha.shape, bool)
        candidate[:175, 175:225] = alpha[:175, 175:225]
        assert not np.any(candidate & assigned03)
        rows.append(dict(frame=row['frame'], source_sha256=sha(src),
                         candidate_interval_leaf_pixels=int(candidate.sum()),
                         own_bark_under_full_canopy=int((own_local & alpha).sum()),
                         own_bark_under_existing_tree03_fragment=int((own_local & assigned03).sum()),
                         candidate_vs_existing_tree03_overlap=0))
    result = dict(status='PRIVATE source partition diagnostic; no geometry or runtime membership approval',
                  accepted_bark_review_sha256=sha(receipt), tree03_scope_sha256=sha(prior),
                  candidate_interval=[175, 0, 225, 175], candidate_is_not_exclusive_tree_membership=True,
                  candidate_14frame_union_pixels=int(union[:175, 175:225].sum()), frames=rows,
                  construction_constraints=[
                      'Retain the bent right branch beyond x225 beneath existing Tree03 red leaf cells where those native samples overlap; do not duplicate the cells into Tree02.',
                      'The candidate x175..225 interval is a conservative construction starting point. Its straight bounds cannot become final physical crown edges.',
                      'Only own source and the two permitted Leicester references may guide irregular volume and branch support.',
                      'All1090 Tree03 accepted bark rays and exact existing native leaf rays require joint saved first-hit proof.',
                      'Root/ledge lower closure and complete shared Arbre08 runtime membership remain unresolved.',
                      'No models, images, API outputs or live scene writes are produced by this audit.'])
    (out / 'receipt.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(dict(frames=len(rows), union=result['candidate_14frame_union_pixels'], frame0=rows[0])))


if __name__ == '__main__':
    main()
