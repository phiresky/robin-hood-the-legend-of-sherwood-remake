"""Generate the 3D asset catalog from directory descriptors, without Blender.

Callers keep their existing publication locks and install payloads before the index.
Prospective file mappings allow transaction preflight and staged merged catalogs to
validate the exact payloads that will be installed, including unchanged live assets.
Historical transaction rollback restores backups rather than publishing a new index.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile


def _relative(value):
    if (not isinstance(value, str) or not value or '\\' in value or '\0' in value
            or any(part in ('', '.', '..') for part in value.split('/'))
            or Path(value).is_absolute()):
        raise ValueError(f'Unsafe asset index path: {value!r}')
    return value


def _document(index):
    return json.loads(index) if isinstance(index, (bytes, str)) else index


def lossy_problems(root, index, *, files=None):
    """Check source/output receipt hashes for every declared lossy derivative."""
    root, index = Path(root), _document(index)
    files = files or {}
    hashes = {}

    def resolve(name):
        name = _relative(name)
        if name in files and files[name] is None:
            raise FileNotFoundError(f'Asset file removed: {name}')
        return Path(files[name]) if name in files else root / name

    def sha(name):
        path = resolve(name)
        if path not in hashes:
            with path.open('rb') as stream:
                hashes[path] = hashlib.file_digest(stream, 'sha256').hexdigest()
        return hashes[path]

    problems = []
    for entry in index['assets']:
        if 'lossy_model' not in entry:
            continue
        identity = entry['id']
        lossy = _relative(entry['lossy_model'])
        model = _relative(entry['model'])
        try:
            if not resolve(lossy).is_file() or not resolve(lossy + '.receipt.json').is_file():
                problems.append(f'{identity}: lossy model or receipt missing')
                continue
            receipt = json.loads(resolve(lossy + '.receipt.json').read_bytes())
            if not isinstance(receipt, dict):
                raise ValueError('receipt must be an object')
            if receipt.get('source') != sha(model):
                problems.append(f'{identity}: lossy receipt does not bind the current model')
            if receipt.get('output') != sha(lossy):
                problems.append(f'{identity}: lossy model bytes differ from its receipt')
        except (OSError, ValueError) as error:
            problems.append(f'{identity}: cannot validate lossy asset: {error}')
    return problems


def validate_asset_index(root, index, *, files=None):
    """Reject malformed catalogs and stale, missing, or corrupt lossy assets."""
    index = _document(index)
    if not isinstance(index, dict) or not isinstance(index.get('assets'), list):
        raise ValueError('Asset index must contain an assets array')
    ids = set()
    for entry in index['assets']:
        if not isinstance(entry, dict) or not isinstance(entry.get('id'), str) or not entry['id']:
            raise ValueError('Asset index entry requires an ID')
        if entry['id'] in ids:
            raise ValueError('Duplicate asset index ID: ' + entry['id'])
        ids.add(entry['id'])
        for key in ('model', 'descriptor', 'lossy_model', 'preview_model'):
            if key in entry:
                _relative(entry[key])
        if 'lossy_model' in entry and 'model' not in entry:
            raise ValueError('Lossy asset requires a source model: ' + entry['id'])
    problems = lossy_problems(root, index, files=files)
    if problems:
        raise ValueError('Asset index has non-current lossy assets:\n' + '\n'.join(problems))


def encoded(index):
    return (json.dumps(index, separators=(',', ':'), ensure_ascii=False) + '\n').encode()


_EDITOR_FIELDS = ('version', 'kind', 'id', 'name', 'source_map', 'source_origin_scene',
                  'model', 'model_scene', 'resources', 'states', 'editor_usage', 'gameplay')
_EDITOR_PART_FIELDS = ('node', 'name', 'default_hidden', 'appearance', 'gameplay_only', 'source_obstacle',
                       'source_components', 'mission_profile', 'scenery',
                       'obstacle_local_game', 'collision', 'sight_join_edges', 'sight_join_caps')


def editor_descriptor(descriptor):
    """Only the catalog fields needed to insert, display, and save editor assets."""
    def parts(values):
        return [{key: value[key] for key in _EDITOR_PART_FIELDS if key in value}
                for value in values]
    result = {key: descriptor[key] for key in _EDITOR_FIELDS if key in descriptor}
    if descriptor.get('editor_usage') == 'map-background':
        result['components'] = descriptor['components']
    result['parts'] = parts(descriptor.get('parts', []))
    for field in ('state_variants', 'standalone_variants'):
        if field in descriptor:
            result[field] = {
                state: {key: (parts(value[key]) if key == 'parts' else value[key])
                        for key in ('name', 'model', 'model_scene', 'parts') if key in value}
                for state, value in descriptor[field].items()}
    return result


def discover_asset_index(root, *, files=None, descriptors=None):
    """Build a catalog from asset.json files, never from the previous index.

    Hidden, backup, and shared-blob directories are not asset directories. File
    overrides represent an upcoming transaction; None represents a removed file.
    `descriptors` restricts private audit catalogs to their explicit scope.
    Discovery can inspect stale derivatives so the derivation tool can repair them;
    generation/publication always validates them.
    """
    root = Path(root)
    files = files or {}
    if not root.is_dir() and not files:
        raise FileNotFoundError(f'Asset root is not a directory: {root}')
    ignored = {'backups', 'backup', 'blobs', 'node_modules'}
    def visible(name):
        return not any(part.startswith('.') or part in ignored for part in Path(name).parts[:-1])
    names = set()
    def walk_error(error):
        raise error
    if root.exists():
        for folder, dirs, leaves in os.walk(root, onerror=walk_error):
            dirs[:] = sorted(d for d in dirs if not d.startswith('.') and d not in ignored
                             and not (Path(folder)/d).is_symlink())
            if 'asset.json' in leaves:
                names.add((Path(folder)/'asset.json').relative_to(root).as_posix())
    for name, source in files.items():
        _relative(name)
        if Path(name).name == 'asset.json' and visible(name):
            if source is None: names.discard(name)
            else: names.add(name)
    if descriptors is not None:
        selected = {_relative(name) for name in descriptors}
        missing = selected - names
        if missing:
            raise ValueError('Missing asset descriptors: ' + ', '.join(sorted(missing)))
        names = selected

    def resolve(name):
        source = files.get(name, root/name)
        return Path(source) if source is not None else None

    def exists(name):
        source = resolve(name)
        return source is not None and source.is_file()

    entries = []
    for name in sorted(names):
        source = resolve(name)
        if source is None:
            continue
        descriptor_bytes = source.read_bytes()
        descriptor = json.loads(descriptor_bytes)
        if not isinstance(descriptor, dict):
            raise ValueError(f'{name}: descriptor must be an object')
        for key in ('id', 'name', 'source_map', 'model'):
            if not isinstance(descriptor.get(key), str) or not descriptor[key].strip():
                raise ValueError(f'{name}: descriptor requires {key}')
        if any(char in descriptor['id'] for char in '/\\:\0'):
            raise ValueError(f'{name}: invalid asset ID')
        for field in ('model_scene', 'asset_type'):
            if field in descriptor and (not isinstance(descriptor[field], str) or not descriptor[field].strip()):
                raise ValueError(f'{name}: invalid {field}')
        if 'tags' in descriptor and (not isinstance(descriptor['tags'], list)
                                    or any(not isinstance(tag, str) for tag in descriptor['tags'])):
            raise ValueError(f'{name}: tags must be an array of strings')
        if 'editor_usage' in descriptor and descriptor['editor_usage'] != 'map-background':
            raise ValueError(f'{name}: invalid editor_usage')
        parent = Path(name).parent
        def local(value):
            return (parent / _relative(value)).as_posix()
        model = local(descriptor['model'])
        if not exists(model):
            raise ValueError(f'{name}: source model missing: {model}')
        entry = {key: descriptor[key] for key in ('id', 'name', 'source_map')}
        entry.update(descriptor=name, model=model)
        entry['descriptor_sha256'] = hashlib.sha256(descriptor_bytes).hexdigest()
        entry['editor'] = editor_descriptor(descriptor)
        for key in ('model_scene', 'editor_usage', 'asset_type', 'tags'):
            if key in descriptor: entry[key] = descriptor[key]
        for kind in ('lossy', 'preview'):
            field = kind + '_model'
            model_name = Path(descriptor['model'])
            basename = kind + '.glb' if model_name.name == 'model.glb' else model_name.stem + '.' + kind + '.glb'
            candidates = list(dict.fromkeys([local(model_name.with_name(basename).as_posix()),
                                            local(model_name.with_name(kind + '.glb').as_posix())]))
            found = [path for path in candidates if exists(path) or exists(path + '.receipt.json')]
            if len(found) > 1:
                raise ValueError(f'{name}: ambiguous {field}: {found}')
            if not found: continue
            candidate = found[0]
            if kind == 'preview' and not exists(candidate):
                raise ValueError(f'{name}: preview model missing: {candidate}')
            entry[field] = candidate
        entries.append(entry)
    return {'version': 1, 'assets': sorted(entries, key=lambda entry: entry['id'])}


def generate_asset_index(root, *, files=None, descriptors=None):
    index = discover_asset_index(root, files=files, descriptors=descriptors)
    validate_asset_index(root, index, files=files)
    return index


def write_asset_index(root, *, target=None, files=None, descriptors=None):
    """Generate from directories, validate, and atomically replace the disposable index."""
    index = generate_asset_index(root, files=files, descriptors=descriptors)
    data = encoded(index)
    target = Path(target) if target is not None else Path(root) / 'index.json'
    mode = target.stat().st_mode & 0o777 if target.exists() else 0o644
    fd, temporary = tempfile.mkstemp(prefix='.' + target.name + '-', suffix='.tmp', dir=target.parent)
    try:
        with os.fdopen(fd, 'wb') as stream:
            os.fchmod(stream.fileno(), mode)
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, target)
    finally:
        Path(temporary).unlink(missing_ok=True)
    return index


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path, nargs='?', default=Path(__file__).resolve().parents[1]/'library/3d-assets',
                        help='3d-assets directory (defaults to the editor library)')
    parser.add_argument('--check', action='store_true', help='Generate and validate without writing')
    parser.add_argument('--print', dest='print_index', action='store_true', help='Print the generated catalog')
    parser.add_argument('--files', type=json.loads, help='JSON mapping of prospective relative paths to staged files')
    args = parser.parse_args()
    index = (generate_asset_index(args.root, files=args.files) if args.check
             else write_asset_index(args.root, files=args.files))
    if args.print_index:
        sys.stdout.buffer.write(encoded(index))
    else:
        action = 'Validated' if args.check else 'Generated'
        print(f"{action} {len(index['assets'])} assets from {args.root}")
