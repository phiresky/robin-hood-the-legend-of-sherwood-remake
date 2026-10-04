"""Collect manually reviewed Croisement02 texture experiments without moving them."""
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from catalog import OUT
sys.path.insert(0, str(HERE.parents[1] / 'refinement'))
from build_texture_gallery import collect


def main():
    roots = sorted({review.parent.parent for review in
                    (OUT / 'texture-fill-round-1').rglob('texture-review.json')})
    if not roots:
        raise ValueError('No manually reviewed texture experiments exist')
    print(collect(roots[0], OUT / 'texture-review', 'Croisement02', roots[1:]))


if __name__ == '__main__':
    main()
