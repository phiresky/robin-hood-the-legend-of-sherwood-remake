"""Preserve unrelated cached metadata during the scoped three-stone transaction."""
import hashlib
import json
import os
from pathlib import Path
import tempfile


def merge_scope(prior, generated, selected, retired):
    before = {a['id']: a for a in prior['assets']}
    fresh = {a['id']: a for a in generated['assets']}
    retired = set(retired)
    scope = set(selected) | retired
    expected = (set(before) - retired) | set(selected)
    assert set(fresh) == expected, {'unexpected': sorted(set(fresh)-expected), 'missing': sorted(expected-set(fresh))}
    for identity in set(before) - scope:
        # Discovery may compact the cached editor view; all file references,
        # descriptor hashes and actual identity metadata must remain exact.
        assert {k:v for k,v in before[identity].items() if k != 'editor'} == {
            k:v for k,v in fresh[identity].items() if k != 'editor'}, identity
        fresh[identity] = before[identity]
    return dict(prior, assets=[fresh[k] for k in sorted(fresh)])


def check_prior(path, expected):
    assert hashlib.sha256(Path(path).read_bytes()).hexdigest() == expected, 'Stale scoped index preflight'


def writer(prior, expected_hash, selected, retired):
    from asset_index import generate_asset_index, validate_asset_index, encoded
    def write(root, *, target=None, files=None, descriptors=None):
        root = Path(root)
        check_prior(root/'index.json', expected_hash)
        fresh = generate_asset_index(root, files=files, descriptors=descriptors)
        merged = merge_scope(prior, fresh, selected, retired)
        validate_asset_index(root, merged, files=files)
        target = Path(target) if target else root/'index.json'
        fd, temporary = tempfile.mkstemp(prefix='.scoped-c03-', suffix='.tmp', dir=target.parent)
        try:
            with os.fdopen(fd, 'wb') as stream:
                os.fchmod(stream.fileno(), target.stat().st_mode & 0o777 if target.exists() else 0o644)
                stream.write(encoded(merged));stream.flush();os.fsync(stream.fileno())
            check_prior(root/'index.json', expected_hash)
            os.replace(temporary, target)
        finally:
            Path(temporary).unlink(missing_ok=True)
        return merged
    return write
