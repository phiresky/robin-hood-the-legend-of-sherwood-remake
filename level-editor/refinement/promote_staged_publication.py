"""Prepare a hash-guarded promotion manifest; apply only with explicit --apply.

Preparing never edits the live library. Applying backs up every existing target
before writing and restores copied targets if any replacement raises an error.
"""
import argparse
from contextlib import contextmanager
import fcntl
import hashlib
import json
from asset_index import write_asset_index, discover_asset_index
from pathlib import Path
import shutil
import sys
from scene_manifest import scene_metadata
from asset_scenes import scene_identity
sys.path.insert(0, str(Path(__file__).resolve().parent / 'blender'))
from lossy_assets import verify_derivatives


@contextmanager
def library_lock(library):
    with (Path(library)/'.publication.lock').open('a+') as handle:
        fcntl.flock(handle, fcntl.LOCK_EX)
        yield


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest() if path.exists() else None


def safe_relative(value):
    if (not isinstance(value, str) or not value or
            any(character in value for character in '\\\0:#?%') or
            any(part in ('', '.', '..') for part in value.split('/'))):
        raise ValueError('Unsafe asset-relative path: ' + repr(value))
    return Path(value)


def contained_path(root, relative, *, required=False):
    path = (root / relative).resolve(strict=required)
    if not path.is_relative_to(root.resolve()):
        raise ValueError('Asset path escapes library root: ' + str(relative))
    return path


def asset_file_pairs(stage_assets, library_assets, asset):
    """Include every selectable static endpoint in the same guarded promotion."""
    descriptor_path = safe_relative(asset['descriptor'])
    model_path = safe_relative(asset['model'])
    descriptor = json.loads(contained_path(stage_assets, descriptor_path, required=True).read_text())
    if descriptor.get('id') != asset['id']:
        raise ValueError('Asset descriptor identity mismatch: ' + asset['id'])
    if descriptor_path.parent / safe_relative(descriptor['model']) != model_path:
        raise ValueError('Asset descriptor model path mismatch: ' + asset['id'])
    scene_identity(descriptor)
    paths = [descriptor_path, model_path]
    receipt_path = descriptor_path.parent / 'bundle.receipt.json'
    if descriptor.get('model_scene') is not None and model_path.suffix == '.glb' and 'resources' not in descriptor:
        receipt = json.loads(contained_path(stage_assets, receipt_path, required=True).read_text())
        if (receipt.get('asset_id') != asset['id'] or
                receipt.get('output', {}).get('sha256') != sha(contained_path(stage_assets, model_path, required=True)) or
                receipt.get('output_descriptor_sha256') != sha(contained_path(stage_assets, descriptor_path, required=True))):
            raise ValueError('Bundled asset receipt does not match model')
        paths.append(receipt_path)
    variant_fields = [key for key in ('state_variants', 'standalone_variants') if key in descriptor]
    if len(variant_fields) > 1 or ('standalone_variants' in descriptor and 'states' in descriptor):
        raise ValueError('Conflicting static asset variants')
    variants = descriptor[variant_fields[0]] if variant_fields else None
    if variants is not None:
        if not isinstance(variants, dict) or not variants:
            raise ValueError('Expected nonempty state_variants')
        for state, variant in variants.items():
            if state not in ('initial', 'applied') or not isinstance(variant, dict):
                raise ValueError('Invalid static asset variant')
            if not isinstance(variant.get('name'), str) or not variant['name'].strip():
                raise ValueError('Static asset variant requires a name')
            scene_identity(variant)
            paths.append(descriptor_path.parent / safe_relative(variant.get('model')))
    pairs = [(contained_path(stage_assets, relative, required=True),
              contained_path(library_assets, relative)) for relative in dict.fromkeys(paths)]
    for resource in descriptor.get('resources', []):
        relative = safe_relative(resource['path'])
        source = contained_path(stage_assets.parent/'map-assets', relative, required=True)
        if sha(source) != resource['sha256']: raise ValueError('Asset resource changed')
        pairs.append((source, contained_path(library_assets.parent, relative)))
    return pairs


def prepare(stage, library, main_blend, map_name, catalog_source=None, catalog_target=None,
            browser_waiver=None):
    with library_lock(library):
        return _prepare(stage, library, main_blend, map_name, catalog_source, catalog_target, browser_waiver)


def _prepare(stage, library, main_blend, map_name, catalog_source=None, catalog_target=None,
             browser_waiver=None):
    if (catalog_source is None) != (catalog_target is None):
        raise ValueError('Catalog source and target must be supplied together')
    required = ['asset-verification.json', 'handoff-verification.json']
    if browser_waiver is None:
        required.append('browser-result.json')
    for name in required:
        if json.loads((stage/name).read_text())['status'] != 'PASS':
            raise ValueError('Missing successful verification: ' + name)
    index_path=library/'3d-assets/index.json'
    staged_root = stage/'map-assets/3d-assets'
    if not any(staged_root.rglob('asset.json')): staged_root = stage/'assets'
    staged = discover_asset_index(staged_root)
    merged=stage/'promotion-library-index.json'
    asset_library = stage / 'map-assets'
    document_path = stage / 'browser-document.rhlos-map.json'
    if not document_path.exists():
        document_path = stage / f'{map_name}.rhlos-map.json'
    document = json.loads(document_path.read_text())
    scene_metadata(asset_library, document)
    pairs=[(stage/'worker.blend',main_blend)]
    from stored_map import asset_source_references
    for reference in document['sceneAssets'] + list(asset_source_references(document)):
        descriptor = (json.loads(contained_path(asset_library, safe_relative(reference['descriptor']), required=True).read_text())
                      if reference.get('descriptor') else reference)
        relatives = [reference['model']] + [resource['path'] for resource in descriptor.get('resources', [])]
        if reference.get('descriptor'): relatives.append(reference['descriptor'])
        pairs.extend((contained_path(asset_library,safe_relative(relative),required=True),
                      contained_path(library,safe_relative(relative))) for relative in relatives)
    if catalog_source is not None:
        catalog = json.loads(catalog_source.read_text())
        if catalog.get('map', '').lower() != map_name.lower() or not isinstance(catalog.get('groups'), list):
            raise ValueError('Catalog source does not match the published map')
        pairs.append((catalog_source, catalog_target))
    selected=discover_asset_index(stage/'assets')
    for asset in selected['assets']:
        pairs.extend(asset_file_pairs(stage/'assets', library/'3d-assets', asset))
    # Discover derivatives from the staged directories, then carry their receipts with them.
    problems = verify_derivatives(staged_root)
    if problems:
        raise ValueError('Stale staged derivatives: ' + '; '.join(problems[:5]))
    for asset in staged['assets']:
        for key in ('lossy_model', 'preview_model'):
            if asset.get(key):
                for relative in (safe_relative(asset[key]), safe_relative(asset[key] + '.receipt.json')):
                    pairs.append((contained_path(staged_root, relative, required=True),
                                  contained_path(library/'3d-assets', relative)))
            else:
                kind = key.removesuffix('_model')
                model = safe_relative(asset['model'])
                for name in {kind + '.glb', model.stem + '.' + kind + '.glb'}:
                    for suffix in ('', '.receipt.json'):
                        target = contained_path(library/'3d-assets', model.with_name(name + suffix))
                        if target.exists():
                            pairs.append((None, target))
    # Install manifests only after all referenced assets exist.
    asset_root = (library/'3d-assets').resolve()
    prospective = {str(target.resolve().relative_to(asset_root)): source for source, target in pairs
                   if target.resolve().is_relative_to(asset_root)}
    write_asset_index(asset_root, target=merged, files=prospective)
    pairs.extend([(document_path,library/f'scenes/{map_name}.rhlos-map.json'),(merged,index_path)])
    records=[]
    targets={}
    for index,(source,target) in enumerate(pairs):
        source=source.resolve(strict=True) if source is not None else None;target=target.resolve()
        if target in targets:
            if targets[target] != source and (source is None or targets[target] is None or sha(targets[target]) != sha(source)):
                raise ValueError('Conflicting promotion target: ' + str(target))
            continue
        targets[target]=source
        records.append({'source':str(source) if source is not None else None,'target':str(target),
                        'source_sha256':sha(source) if source is not None else None,
                        'previous_sha256':sha(target),'backup':str(stage/'promotion-backup'/f'{index:03d}-{target.name}')})
    protected=[]
    for suffix in ('-volumes.scene.json', '-volumes.scene.glb'):
        path=library/f'scenes/{map_name}{suffix}'
        protected.append({'path':str(path),'sha256':sha(path)})
    manifest={'status':'PREPARED_NOT_APPLIED','stage':str(stage),'files':records,'protected_files':protected,
              'browser_check': ({'status': 'WAIVED', 'reason': browser_waiver} if browser_waiver is not None
                                else {'status': 'PASS'}),
              'library':str(library.resolve()), 'index_generation':{'target':str(index_path.resolve())}}
    path=stage/'promotion.json'
    if path.exists():
        raise FileExistsError(path)
    path.write_text(json.dumps(manifest,indent=2)+'\n')
    print(path)


def apply(path):
    manifest=json.loads(path.read_text())
    library=Path(manifest.get('library', Path(next(item['target'] for item in manifest['files']
        if item['target'].endswith('/3d-assets/index.json'))).parents[1]))
    with library_lock(library):
        return _apply(path)


def _apply(path):
    manifest=json.loads(path.read_text())
    if manifest['status']!='PREPARED_NOT_APPLIED':
        raise ValueError('Promotion manifest already applied')
    merge=manifest.get('index_generation')
    if merge:
        item=next(item for item in manifest['files'] if item['target']==merge['target'])
        target=Path(item['target'])
        previous=sha(target)
        if sha(target)!=previous:
            raise ValueError('Library index changed during merge')
        asset_root = target.parent.resolve()
        prospective = {str(Path(record['target']).resolve().relative_to(asset_root)):
                       Path(record['source']) if record['source'] is not None else None
                       for record in manifest['files']
                       if Path(record['target']).resolve().is_relative_to(asset_root)}
        write_asset_index(asset_root, target=Path(item['source']), files=prospective)
        item['source_sha256']=sha(Path(item['source']))
        item['previous_sha256']=previous
        # Install the index last, after every file it references exists.
        manifest['files']=[record for record in manifest['files'] if record is not item]+[item]
        path.write_text(json.dumps(manifest,indent=2)+'\n')
    for record in manifest['protected_files']:
        if sha(Path(record['path']))!=record['sha256']:
            raise ValueError('Protected editor document changed')
    for item in manifest['files']:
        source_hash = sha(Path(item['source'])) if item['source'] is not None else None
        if source_hash!=item['source_sha256'] or sha(Path(item['target']))!=item['previous_sha256']:
            raise ValueError('Promotion input/target changed: '+item['target'])
        if Path(item['backup']).exists():
            raise FileExistsError(item['backup'])
    for item in manifest['files']:
        target,backup=Path(item['target']),Path(item['backup'])
        backup.parent.mkdir(parents=True,exist_ok=True)
        if target.exists():shutil.copy2(target,backup)
    written=[]
    try:
        for item in manifest['files']:
            target=Path(item['target']);target.parent.mkdir(parents=True,exist_ok=True)
            if sha(target)!=item['previous_sha256']:
                raise ValueError('Promotion target changed before write: '+str(target))
            temporary=target.with_name(target.name+'.publication-tmp')
            if temporary.exists():raise FileExistsError(temporary)
            if item['source'] is None:
                target.unlink()
            elif target.resolve() == (Path(manifest['library'])/'3d-assets/index.json').resolve():
                write_asset_index(target.parent)
            else:
                shutil.copy2(item['source'],temporary)
                temporary.replace(target)
            written.append(item)
            if sha(target)!=item['source_sha256']:raise ValueError('Copied hash mismatch')
    except Exception:
        for item in reversed(written):
            if item['previous_sha256'] is None:Path(item['target']).unlink()
            else:shutil.copy2(item['backup'],item['target'])
        raise
    manifest['status']='APPLIED'
    path.write_text(json.dumps(manifest,indent=2)+'\n')
    print('Applied '+str(len(written))+' files; backups retained')


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('stage',type=Path)
    parser.add_argument('--library',type=Path,default=Path('level-editor/library'))
    parser.add_argument('--main-blend',type=Path)
    parser.add_argument('--map',default='derby')
    parser.add_argument('--catalog-source',type=Path,help='Optional staged authored catalog to promote atomically')
    parser.add_argument('--catalog-target',type=Path,help='Live authored catalog target; requires --catalog-source')
    parser.add_argument('--apply',action='store_true')
    parser.add_argument('--waive-browser-check',metavar='REASON',
                        help='Prepare without a passing browser-result.json; the reason is recorded in promotion.json')
    args=parser.parse_args();stage=args.stage.resolve(strict=True)
    if args.apply:apply(stage/'promotion.json')
    else:
        if args.main_blend is None:parser.error('--main-blend is required to prepare')
        # A first publication may create its main working blend. The promotion
        # manifest records a missing target and guards that absence before apply.
        prepare(stage,args.library.resolve(strict=True),args.main_blend.resolve(),args.map,
                args.catalog_source.resolve(strict=True) if args.catalog_source else None,
                args.catalog_target.resolve() if args.catalog_target else None,
                args.waive_browser_check)
