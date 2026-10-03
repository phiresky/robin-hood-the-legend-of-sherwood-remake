"""Stage and publish the editor's runtime library as Cloudflare Worker static assets.

Original models, asset descriptors, external model textures, receipts, backups and authoring files
are never uploaded. Pinned scenery sprite banks are runtime resources and are included.
The local library is not modified. --stage-only is offline;
--dry-run additionally asks Wrangler to validate the deployment without publishing.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import struct

from asset_index import generate_asset_index, encoded
from stored_map import asset_source_references

from cloudflare_publish import EDITOR, worker_config, new_output, deploy
MAX_FILE_BYTES = 25 * 1024 * 1024
MAX_FILES = 20_000


def put_catalog(index, put):
    """Keep the legacy index when it fits; otherwise publish immutable asset batches."""
    data = encoded(index)
    if len(data) <= MAX_FILE_BYTES:
        put('3d-assets/index.json', data)
        return
    shards, batch = [], []
    def flush():
        payload = encoded({'version': index['version'], 'assets': batch})
        checksum = digest(payload)
        path = f'3d-assets/catalog-{checksum}.json'
        put(path, payload)
        shards.append({'path': path, 'sha256': checksum})
    size = 0
    for entry in index['assets']:
        length = len(encoded(entry))
        if batch and size + length > MAX_FILE_BYTES // 2:
            flush()
            batch = []
            size = 0
        batch.append(entry)
        size += length
    if batch:
        flush()
    put('3d-assets/index.json', encoded({**index, 'assets': [], 'asset_shards': shards}))


def digest(data):
    return hashlib.sha256(data).hexdigest()


def put_model(relative, data, index, put):
    """Transport large runtime models in verified pieces without changing any GLB bytes."""
    if len(data) <= MAX_FILE_BYTES:
        put(relative, data)
        return
    shards = []
    for start in range(0, len(data), MAX_FILE_BYTES // 2):
        chunk = data[start:start + MAX_FILE_BYTES // 2]
        checksum = digest(chunk)
        path = f'3d-assets/model-chunk-{checksum}.bin'
        put(path, chunk)
        shards.append({'path': path, 'sha256': checksum})
    index.setdefault('model_shards', {})[relative] = {'bytes': len(data), 'sha256': digest(data), 'parts': shards}


def safe_file(root, relative):
    if (not isinstance(relative, str) or not relative or re.search(r'[\\\x00:#?%]', relative)
            or any(part in ('', '.', '..') for part in relative.split('/'))):
        raise ValueError(f'Unsafe runtime library path: {relative!r}')
    path = (root/relative).resolve(strict=True)
    if not path.is_relative_to(root) or not path.is_file():
        raise ValueError(f'Runtime library path escapes its root: {relative}')
    return path


def self_contained_glb(data, name):
    if len(data) < 20:
        raise ValueError(f'Invalid GLB: {name}')
    magic, version, length, chunk_size, chunk_type = struct.unpack_from('<5I', data)
    if (magic != 0x46546c67 or version != 2 or length != len(data)
            or chunk_type != 0x4e4f534a or chunk_size > len(data) - 20):
        raise ValueError(f'Invalid GLB: {name}')
    model = json.loads(data[20:20 + chunk_size])
    for record in model.get('buffers', []) + model.get('images', []):
        if 'uri' in record and not record['uri'].startswith('data:'):
            raise ValueError(f'Runtime GLB requires an external resource: {name}: {record["uri"]}')
    return model


def gameplay_frame_glb(data, entry):
    """Package metadata-only frames without requiring a geometry/texture derivative."""
    parts = entry.get('editor', {}).get('parts', [])
    if not parts or not all(part.get('gameplay_only') is True for part in parts):
        raise ValueError(f'Asset needs a current lossy GLB before publication: {entry["id"]}')
    model = self_contained_glb(data, entry['model'])
    if (len(data) != 20 + struct.unpack_from('<I', data, 12)[0]
            or any(model.get(key) for key in ('meshes', 'buffers', 'bufferViews', 'accessors',
                                             'images', 'textures', 'materials', 'skins', 'animations'))
            or any('mesh' in node or 'skin' in node for node in model.get('nodes', []))):
        raise ValueError(f'Gameplay frame contains renderable resources: {entry["id"]}')
    raw = json.dumps(model, separators=(',', ':')).encode()
    raw += b' ' * (-len(raw) % 4)
    return struct.pack('<5I', 0x46546c67, 2, 20 + len(raw), len(raw), 0x4e4f534a) + raw


def stage_library(library, output, *, worker_name='robinhood-editor-library'):
    library, output = Path(library).resolve(strict=True), Path(output).resolve()
    if output.is_relative_to(library) or library.is_relative_to(output):
        raise ValueError('Publish output must be outside the source library')
    if not re.fullmatch(r'[a-z0-9][a-z0-9-]{0,62}', worker_name):
        raise ValueError('Invalid Worker name')
    if not (library/'game-data/index.json').is_file():
        raise ValueError('Missing game data index.json; run pnpm library:game-data before publishing')
    game_index_data = safe_file(library, 'game-data/index.json').read_bytes()
    game_index = json.loads(game_index_data)
    if (not isinstance(game_index, dict) or game_index.get('version') != 1
            or not isinstance(game_index.get('files'), list)):
        raise ValueError('Invalid game data index')
    index = generate_asset_index(library/'3d-assets')
    output.mkdir(parents=True, exist_ok=False)
    site = output/'site'; site.mkdir()
    originals = {'3d-assets/'+entry['model'] for entry in index['assets']}
    records, models, descriptors = {}, {}, {}

    def put(relative, data):
        if relative in originals or relative.endswith('.receipt.json') or relative.endswith('/asset.json'):
            raise ValueError(f'Authoring asset must not be uploaded: {relative}')
        if len(data) > MAX_FILE_BYTES:
            raise ValueError(f'Cloudflare static asset exceeds 25 MiB: {relative}')
        target = site/'editor/library'/relative
        if relative in records:
            if records[relative]['sha256'] != digest(data):
                raise ValueError(f'Conflicting runtime payload: {relative}')
            return
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        records[relative] = {'bytes': len(data), 'sha256': digest(data)}

    def copy(relative, expected=None, *, glb=False):
        data = safe_file(library, relative).read_bytes()
        if expected is not None and digest(data) != expected:
            raise ValueError(f'Asset changed while staging: {relative}')
        if glb:
            self_contained_glb(data, relative)
            put_model(relative, data, index, put)
        else:
            put(relative, data)
        return data

    for entry in index['assets']:
        if not entry['model'].endswith('.glb'):
            raise ValueError(f'Asset needs a current lossy GLB before publication: {entry["id"]}')
        model = '3d-assets/'+entry['model']
        # Recheck the original at snapshot time, but never add it to the upload.
        with safe_file(library, model).open('rb') as source:
            source_hash = hashlib.file_digest(source, 'sha256').hexdigest()
        if entry.get('lossy_model'):
            lossy = '3d-assets/'+entry['lossy_model']
            receipt = json.loads(safe_file(library, lossy+'.receipt.json').read_bytes())
            if receipt.get('source') != source_hash:
                raise ValueError(f'Original changed while staging: {model}')
            copy(lossy, receipt['output'], glb=True)
        else:
            data = safe_file(library, model).read_bytes()
            if digest(data) != source_hash:
                raise ValueError(f'Original changed while staging: {model}')
            runtime = gameplay_frame_glb(data, entry)
            entry['lossy_model'] = str(Path(entry['model']).with_suffix('.runtime.glb'))
            put('3d-assets/'+entry['lossy_model'], runtime)
        entry['model_sha256'] = source_hash
        descriptor_path = '3d-assets/'+entry['descriptor']
        raw = safe_file(library, descriptor_path).read_bytes()
        if digest(raw) != entry['descriptor_sha256']:
            raise ValueError(f'Descriptor changed while staging: {descriptor_path}')
        descriptor = json.loads(raw)
        # Sprite banks remain separate from self-contained geometry derivatives.
        # Publish only directories explicitly consumed by asset animations.
        directories = set()
        for animation in descriptor.get('gameplay', {}).get('animations', []):
            if 'resourceDirectory' not in animation:
                continue
            directory = animation['resourceDirectory']
            if not isinstance(directory, str) or not directory.endswith('.rhs.d'):
                raise ValueError(f'Invalid scenery resource directory: {entry["id"]}')
            directories.add(directory)
        for directory in sorted(directories):
            pins = [pin for pin in descriptor.get('resources', [])
                    if isinstance(pin.get('path'), str)
                    and pin['path'].startswith(directory + '/')]
            if not any(pin['path'] == directory + '/manifest.json' for pin in pins):
                raise ValueError(f'Missing pinned scenery manifest: {entry["id"]}: {directory}')
            for pin in pins:
                if not isinstance(pin.get('sha256'), str) or not re.fullmatch(r'[0-9a-f]{64}', pin['sha256']):
                    raise ValueError(f'Invalid scenery resource hash: {pin["path"]}')
                copy(pin['path'], pin['sha256'])
        for key in ('id', 'name', 'source_map', 'model_scene'):
            if descriptor.get(key) != entry.get(key):
                raise ValueError(f'Descriptor changed while staging: {descriptor_path}: {key}')
        if str(Path(entry['descriptor']).parent/descriptor['model']) != entry['model']:
            raise ValueError(f'Descriptor model changed: {entry["id"]}')
        for field in ('state_variants', 'standalone_variants'):
            for variant in descriptor.get(field, {}).values():
                if variant['model'] != descriptor['model']:
                    raise ValueError(f'Bundle separate variant models before publication: {entry["id"]}')
        descriptors[descriptor_path] = entry['descriptor_sha256']
        models[model] = source_hash
        if entry.get('preview_model'):
            copy('3d-assets/'+entry['preview_model'], glb=True)
        else:
            # Palette thumbnails can use the optimized model; never fall back to the original.
            entry['preview_model'] = entry['lossy_model']

    sprites = {}
    maps = sorted(path.name for path in (library/'scenes').iterdir()
                  if path.is_file() and path.name.endswith('.rhlos-map.json') and not path.name.startswith('.'))
    for name in maps:
        document = json.loads(copy('scenes/'+name))
        stem = name.removesuffix('.rhlos-map.json')
        for extension in ('.avif', '.webp', '.png'):
            thumbnail = 'scenes/' + stem + extension
            if (library / thumbnail).is_file():
                copy(thumbnail)
        if document.get('version') not in (1, 2) or 'glb' in document:
            raise ValueError(f'Map must use catalog assets: {name}')
        for reference in document.get('sceneAssets', []) + list(asset_source_references(document)):
            if models.get(reference['model']) != reference['model_sha256']:
                raise ValueError(f'Map source has no matching optimized catalog asset: {name}: {reference["model"]}')
            if reference.get('descriptor') and descriptors.get(reference['descriptor']) != reference['descriptor_sha256']:
                raise ValueError(f'Map descriptor pin is stale: {name}: {reference["descriptor"]}')
        population = document.get('population')
        if population:
            required = sprites.setdefault(population['spriteCatalog'], set())
            required.update(item['sprite'] for item in population.get('actors', []) + population.get('items', []))
    for relative, required in sprites.items():
        catalog = json.loads(safe_file(library, relative).read_bytes())
        if catalog.get('version') != 1 or not isinstance(catalog.get('sprites'), dict):
            raise ValueError(f'Invalid population sprite catalog: {relative}')
        selected = {}
        for identity in sorted(required):
            if identity not in catalog['sprites']:
                raise ValueError(f'Missing population sprite: {relative}: {identity}')
            selected[identity] = catalog['sprites'][identity]
            copy(selected[identity]['image'])
        put(relative, encoded({'version': 1, 'sprites': selected}))
    for relative in game_index['files']:
        path = safe_file(library/'game-data', relative)
        put('game-data/'+relative, path.read_bytes())
    put('game-data/index.json', game_index_data)
    put_catalog(index, put)
    put('scenes/index.json', encoded(maps))
    # Static asset responses are public and revalidate paths that change between releases.
    (site/'_headers').write_text('''/editor/library/*
  Access-Control-Allow-Origin: *
  Cache-Control: public, max-age=0, must-revalidate
  X-Content-Type-Options: nosniff

/editor/library/*.glb
  Content-Type: model/gltf-binary
''')
    if len(records) > MAX_FILES:
        raise ValueError('Runtime library exceeds the 20,000-file Workers free-tier limit')
    config = worker_config(worker_name, '/editor/library')
    (output/'wrangler.json').write_bytes(encoded(config))
    report = {'assets': len(index['assets']), 'maps': len(maps), 'files': len(records),
              'bytes': sum(record['bytes'] for record in records.values()),
              'excluded_original_bytes': sum(safe_file(library, name).stat().st_size for name in originals),
              'worker': worker_name, 'payloads': records}
    (output/'report.json').write_bytes(encoded(report))
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--library', type=Path, default=EDITOR/'library')
    parser.add_argument('--output', type=Path, help='Fresh output directory; defaults to a new work/library-publish snapshot')
    parser.add_argument('--worker-name', default='robinhood-editor-library')
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--stage-only', action='store_true', help='Create and validate payloads without running Wrangler')
    mode.add_argument('--dry-run', action='store_true', help='Stage and run Wrangler deploy --dry-run; no upload')
    args = parser.parse_args()
    output = args.output or new_output('library')
    report = stage_library(args.library, output, worker_name=args.worker_name)
    print(json.dumps({key: value for key, value in report.items() if key != 'payloads'}, indent=2), flush=True)
    print(f'Staged deployment: {output.resolve()}', flush=True)
    if not args.stage_only:
        deploy(output, args.dry_run)


if __name__ == '__main__':
    main()
