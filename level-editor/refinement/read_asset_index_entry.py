# /// script
# requires-python = ">=3.11"
# dependencies = ["ijson>=3.3,<4"]
# ///
"""Read one pinned asset reference without materializing the entire catalog."""
import argparse
import json
from pathlib import Path

import ijson


def read_entry(path, identity):
    with Path(path).open('rb') as stream:
        for entry in ijson.items(stream, 'assets.item', use_float=True):
            if entry.get('id') == identity:
                # The descriptor is loaded and hash-checked separately by the caller.
                return {key: value for key, value in entry.items() if key != 'editor'}
    raise ValueError(f'Asset is absent from the catalog: {identity}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('catalog', type=Path)
    parser.add_argument('asset')
    args = parser.parse_args()
    print(json.dumps(read_entry(args.catalog, args.asset), ensure_ascii=False))
