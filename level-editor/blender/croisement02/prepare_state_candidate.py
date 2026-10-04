"""Stage lossless animation resources and state contracts without publishing a scene."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
from catalog import OUT, ROOT, reviewed_catalog

DATA = ROOT / 'datadirs/fullgame_gog_hackable/Data'

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + '\n')

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, default=OUT / 'state-candidate-v1')
    args = parser.parse_args()
    dest = args.output
    dest.mkdir(exist_ok=False)
    baseline = OUT / 'baseline/Croisement02.rhp.json'
    layers_path = OUT / 'source-states/layers.json'
    level = json.loads(baseline.read_text())
    layers = json.loads(layers_path.read_text())
    files = {}
    def preserve(source, relative):
        target = dest / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(source.read_bytes())
        files[relative] = digest(target)
        assert files[relative] == digest(source)
    preserve(baseline, 'source/Croisement02.rhp.json')
    preserve(layers_path, 'source/layers.json')
    entries = []
    animations = []
    for index, native in enumerate(level['animations']):
        sprite = native['sprite']
        bank = DATA / 'Animations/Day' / (sprite['frame_profile_name'] + '.rhs.d')
        manifest = json.loads((bank / 'manifest.json').read_text())
        profile = copy.deepcopy(next(p for p in manifest['profiles'] if p['name'] == sprite['profile_name']))
        folder = f'sprites/animation-{index:02}.rhs.d'
        resources = []
        for row in profile['rows']:
            old_path = row['path']
            row['path'] = f"rows/{len(resources):04}"
            for frame in row['frames']:
                relative = f"{folder}/{row['path']}/{frame['file']}"
                preserve(bank / profile['name'] / old_path / frame['file'], f'resources/{relative}')
                resources.append({'path': relative, 'sha256': files[f'resources/{relative}']})
        manifest = {**manifest, 'pixel_format': 'legacy_color_keys', 'profiles': [profile]}
        manifest_path = dest / 'resources' / folder / 'manifest.json'
        write(manifest_path, manifest)
        resources.append({'path': f'{folder}/manifest.json', 'sha256': digest(manifest_path)})
        cx, cy = profile['center_x'], profile['center_y']
        origin = [sprite['position_x'] + cx, sprite['position_y'] + cy + sprite['elevation'], sprite['elevation']]
        animation = dict(id=f'animation-{index:02}', anchor=[0, 0, 0], file=sprite['frame_profile_name'],
                         resourceDirectory=folder, profile=sprite['profile_name'], center=[cx, cy],
                         active=native['active'], forceDisplay=native['force_display'], shadow=bool(native['blit_type']),
                         displayPolyline=[[x-origin[0], y-origin[1], -origin[2]] for x, y in native['display_polyline']])
        entries.append(dict(id=f'croisement02-animation-{index:02}', name=sprite['profile_name'], map='Croisement02',
                            origin=origin, animations=[animation], resources=resources))
        animations.append(dict(index=index, profile=sprite['profile_name'], frames=sum(len(r['frames']) for r in profile['rows']),
                               role='canopy; resolve duplicate static appearance before integration' if index < 8 else 'ambient sprite',
                               status='source resources and gameplay placement staged; scene integration pending'))
    write(dest / 'animation-recipe.json', {'version': 1, 'entries': entries})
    missions = []
    for name in sorted({p['mission'] for p in layers['mission_patches']}):
        source = DATA / 'Levels' / (name + '.rhm.json')
        data = json.loads(source.read_text())
        preserve(source, f'missions/{source.name}')
        # Preserve associated script resources, including binary compiled scripts, when present.
        script_files = []
        for sibling in sorted(source.parent.glob(name + '.*')):
            if sibling.is_file() and sibling != source:
                preserve(sibling, f'missions/{sibling.name}')
                script_files.append(sibling.name)
        relevant = [p for p in layers['mission_patches'] if p['mission'] == name]
        assert len(relevant) == len(data['mission_patches'])
        for patch in relevant:
            assert patch['state'] == data['mission_patches'][patch['mission_patch_index']]
            for state in patch['states'].values():
                for frame in state['frames']:
                    preserve(layers_path.parent / frame['image'], f"source/{frame['image']}")
        missions.append(dict(mission=name, patches=len(relevant), script_resources=script_files,
                             status='native mission and all decoded visual frames preserved; preview integration pending'))
    mask_rows = json.loads((OUT / 'baseline/masks/manifest.json').read_text())['masks']
    mask_ids = {(m['layer'], m['layer_index']): m['index'] for m in mask_rows}
    groups = json.loads(reviewed_catalog().read_text())['groups']
    state_groups = []
    for patch in layers['patches']:
        state = patch['state']
        controlled = state['old_sight_obstacles'] + state['new_sight_obstacles']
        if not controlled:
            continue
        state_groups.append(dict(patch=patch['id'], groups=[g['id'] for g in groups if any(p.get('obstacle') in controlled for p in g['parts'])],
            initially_active=state['old_sight_obstacles'], applied_active=state['new_sight_obstacles'],
            initial_masks=[mask_ids[(m['layer'], m['index'])] for m in state['old_masks']],
            applied_masks=[mask_ids[(m['layer'], m['index'])] for m in state['new_masks']],
            definitive=state['definitive'], visible_mesh_required=False, graphic=patch['graphic'],
            status='preserve gameplay metadata and patch binding; do not invent visible meshes'))
    write(dest / 'preservation.json', {'files': files, 'all_copies_byte_identical': True})
    write(dest / 'completeness.json', dict(map='Croisement02', baseline_sha256=digest(baseline), layers_sha256=digest(layers_path),
        animations=animations, missions=missions, native_state_groups=state_groups,
        semantics=['Old masks and obstacles active before apply; new active after final transition.',
                   'Definitive patches cannot toggle back through Apply.',
                   'Invalid final animation disables the sprite; it does not imply retaining the transition image.',
                   'Only integrate_in_background patches bake their terminal transition frame into the map.',
                   'Mission instances remain mission-scoped with original indices, triggers, sectors and scripts.'],
        holds=['Animation source round-trip verification required.', 'Eight canopy sequences must not overlay duplicated static crowns.',
               'Mission state rendering and actor ordering require integrated editor/runtime verification.',
               'This candidate contains no geometry approval or publication authorization.'], status='STAGED_NOT_INTEGRATED'))
    print(json.dumps({'output': str(dest), 'animations': len(animations), 'mission_patches': sum(m['patches'] for m in missions), 'state_groups': len(state_groups), 'preserved_files': len(files)}))

if __name__ == '__main__':
    main()
