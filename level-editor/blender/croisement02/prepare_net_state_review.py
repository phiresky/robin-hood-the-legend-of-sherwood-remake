"""Bind covered rigging, capture patches and mission interaction source records."""
import hashlib
import json
from pathlib import Path

from PIL import Image, ImageDraw

from catalog import OUT


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def sequence_frame(frames, tick, loop=False):
    duration = sum(frame['delay'] + 1 for frame in frames)
    if loop:
        tick %= duration
    for index, frame in enumerate(frames):
        tick -= frame['delay'] + 1
        if tick < 0:
            return index, frame
    return len(frames) - 1, frames[-1]


def main():
    source = OUT / 'state-target-evidence'
    targets_path = source / 'manifest.json'
    targets = json.loads(targets_path.read_text())
    layers_path = OUT / 'source-states/layers.json'
    layers = json.loads(layers_path.read_text())
    destination = OUT / 'net-state-source-review-v1'
    destination.mkdir(exist_ok=True)
    profiles = {profile['id']: profile for profile in targets['profiles']}
    marker_instances, net_instances = [], []
    for instance in targets['instances']:
        profile = profiles[instance['profile_id']]
        if profile['profile'] not in ('Bow Target', 'Croisement02 - piege01h', 'Croisement02 - piege03h'):
            continue
        mission = instance['mission']
        script_path = OUT / 'state-candidate-v1/missions' / f'{mission}.scb.json'
        classes = json.loads(script_path.read_text())['classes']
        script_class = next(row for row in classes if row['class_name'] == instance['target']['script_class'])
        record = dict(instance=instance, script_source_sha256=sha(script_path), script_class=script_class,
                      profile_sha256=profile['profile_sha256'])
        if profile['profile'] == 'Bow Target':
            record['role'] = 'Animated interaction marker; preserve script actions and mission identity, not a static archery prop.'
            marker_instances.append(record)
            continue
        family = profile['profile'].removesuffix('h')
        patches = [patch for patch in layers['mission_patches']
                   if patch['mission'] == mission and patch['name'] in [family + suffix for suffix in 'eig']]
        assert len(patches) == 3, (mission, family, len(patches))
        record['patches'] = patches
        record['covered_target_frames'] = profile['rows']
        record['transition_contract'] = 'Covered target action160 is transparent; e/i body alternatives plus g loose-leaf effect are separate mission patches. Script selects body alternative; do not activate both.'
        net_instances.append(record)
    assert len(marker_instances) == 45
    assert len(net_instances) == 10
    representatives = []
    for family in ('Croisement02 - piege01', 'Croisement02 - piege03'):
        representative = next(record for record in net_instances
                              if profiles[record['instance']['profile_id']]['profile'] == family + 'h')
        profile = profiles[representative['instance']['profile_id']]
        target = representative['instance']['target']
        initial = next(row for row in profile['rows'] if row['action_id'] == 0)['frames'][0]
        initial_xy = [int(target['position_x'] + initial['offset'][0]), int(target['position_y'] + initial['offset'][1])]
        patches = {patch['name'][-1]: patch for patch in representative['patches']}
        all_frames = [frame for patch in patches.values() for state in patch['states'].values() for frame in state['frames']]
        boxes = [frame['bbox'] for frame in all_frames] + [[*initial_xy, *initial['size']]]
        box = [min(row[0] for row in boxes) - 8, min(row[1] for row in boxes) - 8,
               max(row[0] + row[2] for row in boxes) + 8, max(row[1] + row[3] for row in boxes) + 8]
        folder = destination / family.rsplit(' ', 1)[-1]
        folder.mkdir(exist_ok=True)
        records = []
        for variant in 'ei':
            body, debris = patches[variant], patches['g']
            terminal_tick = max(sum(frame['delay'] + 1 for frame in patch['states']['transition']['frames']) for patch in (body, debris))
            final_duration = sum(frame['delay'] + 1 for frame in body['states']['final']['frames'])
            for tick in range(-1, terminal_tick + final_duration):
                canvas = Image.new('RGBA', (box[2] - box[0], box[3] - box[1]))
                selected = []
                if tick == -1:
                    canvas.alpha_composite(Image.open(initial['image']).convert('RGBA'), (initial_xy[0] - box[0], initial_xy[1] - box[1]))
                    selected.append(dict(kind='covered-target', image_sha256=initial['image_sha256']))
                else:
                    for patch in (body, debris):
                        transition = patch['states']['transition']['frames']
                        duration = sum(frame['delay'] + 1 for frame in transition)
                        state = 'transition' if tick < duration else 'final'
                        index, frame = sequence_frame(patch['states'][state]['frames'], tick if state == 'transition' else tick - duration, state == 'final')
                        path = OUT / 'source-states' / frame['image']
                        image = Image.open(path).convert('RGBA')
                        x, y, w, h = frame['bbox']
                        assert image.size == (w, h)
                        canvas.alpha_composite(image, (x - box[0], y - box[1]))
                        selected.append(dict(patch=patch['id'], state=state, frame=index, source_sha256=sha(path), sound_id=frame['sound_id']))
                image_name = f'{variant}-{tick:03}.png'
                canvas.save(folder / image_name)
                records.append(dict(variant=variant, tick=tick, image=image_name, sha256=sha(folder / image_name), selected=selected))
            sample_ticks = [-1, 0, 10, 20, 40, terminal_tick]
            sheet = Image.new('RGBA', (6 * 250, 320), (60, 60, 60, 255))
            draw = ImageDraw.Draw(sheet)
            for column, tick in enumerate(sample_ticks):
                image = Image.open(folder / f'{variant}-{tick:03}.png').convert('RGBA')
                image.thumbnail((248, 290))
                sheet.alpha_composite(image, (column * 250, 25))
                draw.text((column * 250 + 3, 4), f'{variant}: tick {tick}', fill='white')
            sheet.save(folder / f'{variant}-storyboard.png')
        data = dict(family=family, source_mission=representative['instance']['mission'], bbox=box, records=records,
                    note='Exact decoded source review; body alternatives retain their native names. Geometry, attachment depth and per-mission trigger playback are not completed by this source viewer.')
        (folder / 'manifest.json').write_text(json.dumps(data, indent=2) + '\n')
        representatives.append(dict(family=family, folder=folder.name, manifest_sha256=sha(folder / 'manifest.json')))
    report = dict(status='Bound source evidence; private source review, not refined geometry or published states',
                  target_manifest_sha256=sha(targets_path), layer_manifest_sha256=sha(layers_path),
                  net_instances=net_instances, marker_instances=marker_instances, representatives=representatives,
                  semantics=dict(native_hz=25, target_action160='transparent from first frame',
                                 patch_body='Choose e or i from the mission script branch; g is the separate leaf/debris visual.',
                                 activation='Play-animation-freeze starts target action and terminates its sequence command immediately; subsequent patch calls do not wait for the target frame duration.',
                                 final='Keep 14-frame body final animation and distinct debris final state; do not replace all endpoints with one frozen source image.'),
                  limitations=['Script bytecode is bound for all 45 markers; per-call actor-index reconciliation and runtime activation verification remain separate.',
                               'Patch relative composition shown body then debris; full native scene sorting and foreground occlusion still require integration review.'])
    (destination / 'manifest.json').write_text(json.dumps(report, indent=2) + '\n')
    print('Bound', len(net_instances), 'net targets,', len(marker_instances), 'markers and', len(representatives), 'source families')


if __name__ == '__main__':
    main()
