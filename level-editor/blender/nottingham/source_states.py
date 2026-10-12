"""Freeze Nottingham source artwork, native masks and independent patch states.

Run with system Python (Pillow and NumPy), from any directory. This delegates
initial layer/mission extraction to the shared exporter and adds a source audit.
Sight activation lists are evidence, never approved visual receiver assignments.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import struct
import subprocess

import numpy as np
from PIL import Image


ROOT = Path(__file__).resolve().parents[2]


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def validate_composites(output, layers):
    """Verify exact source RGB outside overlays and report overlapping sprites."""
    original = np.array(Image.open(output / 'revealed.png').convert('RGB'))
    reports = {}
    for name, key in [('covered.png', 'initial_graphic'), ('all-applied.png', 'applied_graphic')]:
        expected = Image.fromarray(original).convert('RGBA')
        ownership = np.zeros(original.shape[:2], dtype=np.uint16)
        for record in layers['patches']:
            graphic = record[key]
            if graphic is None:
                continue
            image = Image.open(output / graphic['image']).convert('RGBA')
            x, y, width, height = graphic['bbox']
            expected.alpha_composite(image, (x, y))
            left, top = max(x, 0), max(y, 0)
            right, bottom = min(x + width, original.shape[1]), min(y + height, original.shape[0])
            alpha = np.array(image)[:, :, 3]
            ownership[top:bottom, left:right] += alpha[top-y:bottom-y, left-x:right-x] > 0
        actual = np.array(Image.open(output / name).convert('RGB'))
        changed = np.any(actual != original, axis=2)
        mismatch = int(np.count_nonzero(np.any(actual != np.array(expected.convert('RGB')), axis=2)))
        outside = int(np.count_nonzero(changed & (ownership == 0)))
        if mismatch or outside:
            raise ValueError(f'Composite validation failed: {name}: mismatch={mismatch}, outside={outside}')
        reports[name] = {'source_rgb_changed_outside_graphics': outside,
                         'expected_composite_mismatched_pixels': mismatch,
                         'changed_pixels': int(np.count_nonzero(changed)),
                         'sprite_owned_pixels': int(np.count_nonzero(ownership)),
                         'overlapping_sprite_pixels': int(np.count_nonzero(ownership > 1)),
                         'overlap_policy': 'Native patch order; no receiver assignment implied.'}
    write(output / 'composite-validation.json', reports)
    return reports


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--datadir', type=Path, default=ROOT.parent / 'datadirs/fullgame_gog_hackable')
    parser.add_argument('--output', type=Path, default=ROOT / 'work/needs-review/nottingham-refinement/source-states')
    args = parser.parse_args()
    output = args.output.resolve()
    data = args.datadir.resolve()
    if output.exists():
        raise FileExistsError(f'Frozen evidence already exists: {output}; choose a fresh output')
    output.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(['node', str(ROOT / 'pipeline/src/export-interior-layers.ts'), 'Nottingham', str(output)],
                   check=True, cwd=ROOT.parent, env={**os.environ, 'HACKABLE_DATADIR': str(data)})
    level_path = data / 'Data/Levels/nottingham.rhp.json'
    artwork = data / 'Data/Levels/Day/Nottingham.map.png'
    level = json.loads(level_path.read_text())
    layers = json.loads((output / 'layers.json').read_text())
    shutil.copy2(level_path, output / 'level.json')
    shutil.copy2(artwork, output / 'revealed.png')
    mask_directory = output / 'masks'
    mask_directory.mkdir()
    mask_records, by_layer = [], {}
    for index, mask in enumerate(level['masks']):
        layer = mask['layer']
        layer_index = len(by_layer.setdefault(layer, []))
        by_layer[layer].append(index)
        source = data / f'Data/Levels/nottingham.rhp.d/masks/{index:06}.png'
        target = mask_directory / source.name
        shutil.copy2(source, target)
        with Image.open(target) as image:
            if list(image.size) != mask['box_size']:
                raise ValueError(f'Mask dimensions disagree: {source}')
        mask_records.append({'global_index': index, 'layer': layer, 'layer_index': layer_index,
                             'image': f'masks/{source.name}', 'sha256': sha(target),
                             'box_top_left': mask['box_top_left'], 'box_size': mask['box_size'],
                             'mask_type': mask['mask_type'],
                             'character_polyline': mask['character_polyline'],
                             'projectile_polyline': mask['projectile_polyline']})
    write(output / 'mask-manifest.json', {'version': 1, 'map': 'Nottingham',
          'index_convention': 'Patch references use layer-local indices; PNG filenames use global indices.',
          'masks': mask_records})
    banks = data / 'Data/Animations/Day'
    source_hashes = {str(level_path): sha(level_path), str(artwork): sha(artwork)}

    def states_for(patch, identifier):
        sprite = patch['element_fx']['sprite']
        if sprite['frame_profile_name'] == 'pixel_vert':
            return {}
        bank = next(p for p in banks.iterdir() if p.name.casefold() == (sprite['frame_profile_name'] + '.rhs.d').casefold())
        manifest_path = bank / 'manifest.json'
        source_hashes[str(manifest_path)] = sha(manifest_path)
        profile = next(p for p in json.loads(manifest_path.read_text())['profiles'] if p['name'] == sprite['profile_name'])
        states = {}
        for name, action, flag in [('initial', 148, 'start_animation_valid'),
                                   ('transition', 149, 'transition_animation_valid'),
                                   ('final', 150, 'end_animation_valid')]:
            if not patch[flag]:
                continue
            row = next(r for r in profile['rows'] if r['action_id'] == action)
            if not row['frames']:
                raise ValueError(f'Missing required frames: {identifier} {name}')
            frames = []
            for number, frame in enumerate(row['frames']):
                source = bank / profile['name'] / row['path'] / frame['file']
                if not source.exists():
                    source = bank / row['path'] / frame['file']
                source_hashes[str(source)] = sha(source)
                pixels = np.array(Image.open(source).convert('RGBA'))
                pixels[np.all(pixels[:, :, :3] == [0, 251, 0], axis=2), 3] = 0
                image = Image.fromarray(pixels)
                relative = f'base-patches/{identifier}/{name}-{number:03}.png'
                target = output / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                image.save(target)
                x = math.floor(sprite['position_x'] + frame['offset_x'] + 0.5)
                y = math.floor(sprite['position_y'] + frame['offset_y'] + 0.5)
                frames.append({'image': relative, 'bbox': [x, y, *image.size],
                               'sha256': sha(target), 'delay': frame['delay'], 'sound_id': frame['sound_id']})
            states[name] = {'action_id': action, 'frames': frames}
        return states

    for record, patch in zip(layers['patches'], level['patches'], strict=True):
        states = states_for(patch, record['id'])
        record['states'] = states
        record['initial_graphic'] = states.get('initial', {}).get('frames', [None])[0]
        record['applied_graphic'] = (states['transition']['frames'][-1] if patch['integrate_in_background']
                                     else states.get('final', {}).get('frames', [None])[0])
        record['native_mask_global_indices'] = {name: [by_layer[m['layer']][m['index']] for m in patch[name]]
                                               for name in ['old_masks', 'new_masks']}
        record['projection_receiver_review'] = 'not-reviewed'
        record['applied_graphic_mode'] = ('baked-last-transition-frame' if patch['integrate_in_background']
                                          else 'final-animation' if patch['end_animation_valid'] else 'no-active-sprite')

    def composite(path, base, graphics):
        canvas = Image.open(base).convert('RGBA')
        for graphic in graphics:
            if graphic:
                canvas.alpha_composite(Image.open(output / graphic['image']).convert('RGBA'), tuple(graphic['bbox'][:2]))
        canvas.convert('RGB').save(path)

    composite(output / 'covered.png', artwork, [r['initial_graphic'] for r in layers['patches']])
    composite(output / 'all-applied.png', artwork, [r['applied_graphic'] for r in layers['patches']])
    for record in layers['patches']:
        directory = output / 'state-context' / record['id']
        directory.mkdir(parents=True)
        composite(directory / 'initial.png', artwork, [record['initial_graphic']])
        composite(directory / 'applied.png', artwork, [record['applied_graphic']])
        record['isolated_state_sources'] = {s: f'state-context/{record["id"]}/{s}.png' for s in ['initial', 'applied']}
    missions = layers.get('mission_patches', [])
    for mission in sorted({r['mission'] for r in missions}):
        records = [r for r in missions if r['mission'] == mission]
        for state in ['initial', 'applied']:
            composite(output / records[0]['projection_sources'][state], output / 'covered.png',
                      [r[f'{state}_graphic'] for r in records])
    layers['source_state_notes'] = [
        'revealed.png is unmodified Day artwork, a substrate rather than a claim that every gameplay patch is applied.',
        'covered.png overlays all base initial graphics including integrated-door initial frames.',
        'all-applied.png is a diagnostic combination of base applied graphics; mission reachability is not asserted.',
        'Per-patch state-context images isolate that patch against the Day substrate; other initial covers are absent.',
        'Source RGB is retained outside nontransparent sprite pixels. Frame offsets are added to serialized top-left; elevation is not subtracted.',
        'Sight activation and native masks do not establish visual receiver ownership. Receiver reviews remain pending.',
        'Mission source composites retain independent mission state and do not claim to include scripted changes to base patches.',
    ]
    layers['native_mask_manifest'] = 'mask-manifest.json'
    write(output / 'layers.json', layers)
    validate_composites(output, layers)
    scene = ROOT / 'library/scenes/nottingham-volumes.scene.glb'
    blob = scene.read_bytes()
    size = struct.unpack_from('<I', blob, 12)[0]
    gltf = json.loads(blob[20:20+size])
    names = {n['name'] for n in gltf['nodes']}
    missing = [i for i in range(len(level['sight_obstacles']))
               if f'building-{i:03}' not in names and f'terrace-{i:03}' not in names]
    source_hashes[str(scene)] = sha(scene)
    write(output / 'audit.json', {'version': 1, 'map': 'Nottingham', 'source_hashes': source_hashes,
          'counts': {'masks': len(mask_records), 'patches': len(layers['patches']), 'mission_patches': len(missions),
                     'source_obstacles': len(level['sight_obstacles']), 'scene_meshes': len(gltf['meshes'])},
          'missing_scene_obstacles': [{'index': i, 'source': level['sight_obstacles'][i]} for i in missing],
          'source_state_notes': layers['source_state_notes'],
          'receiver_approval': 'pending', 'geometry_approval': 'pending',
          'limitations': ['All stateful scene components coexist in the source GLB; visibility needs separate authored review.',
                          'Diagnostic composition order is native patch order; overlapping animated foreground states need gameplay-camera review.']})
    write(output / 'hashes.json', {str(p.relative_to(output)): sha(p) for p in sorted(output.rglob('*')) if p.is_file()})
    print(json.dumps({'output': str(output), 'masks': len(mask_records), 'base_patches': len(layers['patches']),
                      'mission_patches': len(missions), 'missing_scene_obstacles': missing}))


if __name__ == '__main__':
    main()
