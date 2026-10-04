"""Reconcile every native and mission patch without treating it as scenery."""
import hashlib
import json
from collections import Counter
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/croisement01-refinement'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    source = OUT / 'source-states'
    manifest = source / 'layers.json'
    layers = json.loads(manifest.read_text())
    destination = OUT / 'state-audit-v1'
    destination.mkdir(exist_ok=False)
    rows, profiles = [], {}
    for patch in layers['mission_patches']:
        state = patch['state']
        frames = []
        for name, animation in patch['states'].items():
            for index, frame in enumerate(animation['frames']):
                path = source / frame['image']
                with Image.open(path) as image:
                    if list(image.size) != frame['bbox'][2:]:
                        raise ValueError(f'Frame bounds mismatch: {path}')
                    alpha = image.convert('RGBA').getchannel('A')
                    pixels = image.width * image.height - alpha.histogram()[0]
                frames.append(dict(state=name, index=index, image=frame['image'],
                                   sha256=sha(path), bbox=frame['bbox'],
                                   delay=frame['delay'], opaque_pixels=pixels))
        if not frames:
            raise ValueError(f'Mission patch has no decoded frames: {patch["id"]}')
        rows.append(dict(id=patch['id'], mission=patch['mission'], name=patch['name'],
                         runtime_patch_index=patch['runtime_patch_index'],
                         initial_graphic=patch.get('initial_graphic'),
                         applied_graphic=patch.get('applied_graphic'),
                         applied_graphic_mode=patch.get('applied_graphic_mode'),
                         integrate_in_background=state['integrate_in_background'],
                         frames=frames, status='native evidence complete; geometry/state integration pending'))
        profiles.setdefault(patch['name'], patch)
    tile_w, tile_h = 400, 240
    board = Image.new('RGB', (tile_w * 2, tile_h * len(profiles)), '#292929')
    draw = ImageDraw.Draw(board)
    for row, (name, patch) in enumerate(sorted(profiles.items())):
        for column, field in enumerate(('initial_graphic', 'applied_graphic')):
            frame = patch.get(field)
            x, y = column * tile_w, row * tile_h
            draw.text((x + 6, y + 5), name + ' / ' + field.split('_')[0], fill='white')
            if not frame:
                draw.text((x + 6, y + 30), 'No displayed endpoint', fill='white')
                continue
            image = Image.open(source / frame['image']).convert('RGBA')
            scale = min((tile_w-12)/image.width, (tile_h-35)/image.height, 4)
            image = image.resize((round(image.width*scale), round(image.height*scale)), Image.Resampling.NEAREST)
            board.paste(image, (x + 6, y + 30), image)
    board.save(destination / 'patch-endpoints.png')
    receipt = dict(source_manifest_sha256=sha(manifest), native_patches=layers['patches'],
                   mission_count=len({r['mission'] for r in rows}),
                   profile_counts=dict(Counter(r['name'] for r in rows)),
                   mission_patch_count=len(rows), decoded_frame_count=sum(len(r['frames']) for r in rows),
                   endpoint_sheet_sha256=sha(destination/'patch-endpoints.png'),
                   records=rows,
                   limitations=['Endpoint extraction does not establish transition timing or draw-order correctness.',
                                'Background-integrated terminal frames require state-scoped ground ownership.',
                                'Native nonvisual patches still change obstacle and mask activation.'])
    (destination/'manifest.json').write_text(json.dumps(receipt, indent=2)+'\n')
    targets, mobiles, target_profiles = [], [], Counter()
    for mission in sorted({r['mission'] for r in rows}):
        path = ROOT/'datadirs/fullgame_gog_hackable/Data/Levels'/f'{mission}.rhm.json'
        data = json.loads(path.read_text())
        for index, record in enumerate(data['targets']):
            target_profiles[(record['filename'], record['profile_name'])] += 1
            targets.append(dict(mission=mission, mission_sha256=sha(path),
                                target_index=index, record=record))
        for index, record in enumerate(data['mobile_elements']):
            mobiles.append(dict(mission=mission, mission_sha256=sha(path),
                                mobile_index=index, record=record))
    target_inventory = dict(status='source records only; sprite reconstruction and script timing pending',
                            target_instances=len(targets), mobile_instances=len(mobiles),
                            profiles=[dict(bank=k[0], profile=k[1], instances=n)
                                      for k, n in target_profiles.items()],
                            targets=targets, mobile_elements=mobiles,
                            limitations=['Living targets and embedded horse teams are not static scenery.',
                                         'Trap and cart bodies require separate visual evidence from their shadow patches.'])
    (destination/'target-mobile-records.json').write_text(json.dumps(target_inventory, indent=2)+'\n')
    print(json.dumps({k: receipt[k] for k in ('mission_count','mission_patch_count','decoded_frame_count','profile_counts')}))


if __name__ == '__main__':
    main()
