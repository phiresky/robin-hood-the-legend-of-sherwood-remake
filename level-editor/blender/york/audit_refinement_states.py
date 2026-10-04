"""Inventory York state evidence independently of static grouping approval."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/york-refinement'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def image_record(root, graphic):
    if graphic is None:
        return None
    path = root / graphic['image']
    with Image.open(path) as image:
        if list(image.size) != graphic['bbox'][2:]:
            raise ValueError(f'Image bounds differ: {path}')
        alpha = image.convert('RGBA').getchannel('A')
        nontransparent = image.width * image.height - alpha.histogram()[0]
    return {**graphic, 'sha256': sha(path), 'nontransparent_pixels': nontransparent}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=OUT / 'geometry-pass-01/state-source-audit-v1')
    args = parser.parse_args()
    destination = args.output.resolve()
    destination.mkdir(exist_ok=False)
    source = OUT / 'source-states-complete'
    layers_path = source / 'layers.json'
    layers = json.loads(layers_path.read_text())
    level_path = OUT / 'baseline/york.rhp.json'
    level = json.loads(level_path.read_text())
    if len(layers['patches']) != len(level['patches']):
        raise ValueError('Base patch inventory mismatch')
    native = [{**p, 'graphic': image_record(source, p['graphic']),
               'review_status': 'Native source evidence only; geometry and state behavior not approved'}
              for p in layers['patches']]
    mission_rows = []
    for patch in layers['mission_patches']:
        frames = []
        for state, animation in patch['states'].items():
            frames.extend(dict(state=state, frame=index, **image_record(source, frame))
                          for index, frame in enumerate(animation['frames']))
        if not frames:
            raise ValueError(f'Missing decoded mission state: {patch["id"]}')
        mission_rows.append(dict(id=patch['id'], mission=patch['mission'], name=patch['name'],
                                 runtime_patch_index=patch['runtime_patch_index'], frames=frames,
                                 initial=image_record(source, patch.get('initial_graphic')),
                                 applied=image_record(source, patch.get('applied_graphic')),
                                 applied_graphic_mode=patch.get('applied_graphic_mode'),
                                 state=patch['state']))
    # Scan all map missions, including those with no mission patches.
    missions, targets, mobiles = [], [], []
    for path in sorted((ROOT / 'datadirs/fullgame_gog_hackable/Data/Levels').glob('*.rhm.json')):
        data = json.loads(path.read_text())
        if data['header']['map_filename'].lower() != 'york':
            continue
        mission = path.name.removesuffix('.rhm.json')
        decoded = sum(p['mission'] == mission for p in mission_rows)
        if decoded != len(data['mission_patches']):
            raise ValueError(f'Mission patch census mismatch: {mission}')
        missions.append(dict(mission=mission, sha256=sha(path), header=data['header'],
                             patches=decoded, targets=len(data['targets']), mobiles=len(data['mobile_elements'])))
        targets.extend(dict(mission=mission, index=i, record=row) for i, row in enumerate(data['targets']))
        mobiles.extend(dict(mission=mission, index=i, record=row) for i, row in enumerate(data['mobile_elements']))
    profiles = Counter((r['record']['filename'], r['record']['profile_name']) for r in targets)
    animations = Counter((r['sprite']['frame_profile_name'], r['sprite']['profile_name']) for r in level['animations'])
    report = dict(map='york', status='source audit; geometry and state integration pending',
                  layers_sha256=sha(layers_path), level_sha256=sha(level_path),
                  native_patches=native, mission_patches=mission_rows, missions=missions,
                  decoded_mission_frames=sum(len(p['frames']) for p in mission_rows),
                  target_profiles=[dict(bank=k[0], profile=k[1], instances=n) for k, n in sorted(profiles.items())],
                  targets=targets, mobile_elements=mobiles,
                  animation_profiles=[dict(bank=k[0], profile=k[1], instances=n) for k, n in sorted(animations.items())],
                  animation_records=level['animations'],
                  behavior_contract=['Background integration uses the terminal transition frame, independently of final FX visibility.',
                                     'A patch without a final animation can still change masks, obstacles, doors, and background.',
                                     'Reversible patches restore their prior background and initial display state.'],
                  limitations=['Native patch transition sequences still need frame extraction; the covered graphic is not a transition audit.',
                               'All 60 native animations still need decoded visual ownership and applicable state review.',
                               'Target records are not reconstructed visual assets, and living targets must not be discarded as scenery.',
                               'Static grouping decisions are not geometry, texture, or state integration approvals.'])
    (destination / 'manifest.json').write_text(json.dumps(report, indent=2) + '\n')
    board = Image.new('RGB', (800, max(1, len(mission_rows)) * 240), '#292929')
    draw = ImageDraw.Draw(board)
    for row, patch in enumerate(mission_rows):
        for col, state in enumerate(('initial', 'applied')):
            x, y = col * 400, row * 240
            draw.text((x+6, y+6), patch['name'] + ' / ' + state, fill='white')
            graphic = patch[state]
            if graphic is None:
                draw.text((x+6, y+30), 'No displayed endpoint', fill='white')
                continue
            image = Image.open(source / graphic['image']).convert('RGBA')
            scale = min(388 / image.width, 200 / image.height, 4)
            image = image.resize((round(image.width*scale), round(image.height*scale)), Image.Resampling.NEAREST)
            board.paste(image, (x+6, y+30), image)
    board.save(destination / 'mission-door-endpoints.png')
    print(json.dumps(dict(native_patches=len(native), missions=len(missions),
                          mission_patches=len(mission_rows), decoded_mission_frames=report['decoded_mission_frames'],
                          target_instances=len(targets), target_profiles=len(profiles), native_animation_instances=len(level['animations']))))


if __name__ == '__main__':
    main()
