"""Compare occurrence-specific live draw order and background-inserted terminal art."""
import json
import sys
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import qualify as q


def main():
    dest = q.OUT / 'profile01-native-loop-composition-v1'
    assert not dest.exists()
    qualification = json.loads((q.OUT / 'qualification.json').read_text())
    row = qualification['rows'][0]
    bindings = json.loads(q.BIND.read_text())
    owners = [r for r in bindings['rows'] if r['profile'] == row['profile']]
    loop_report_path = q.OUT / 'profile02-native-loop-composition-v1/report.json'
    loop_report = json.loads(loop_report_path.read_text())
    occurrences = []
    for owner in owners:
        polyline = owner['patch']['element_fx']['display_polyline']
        minimum = min(p[1] for p in polyline) if polyline else 1e15
        occurrences.append(dict(mission=owner['mission'], patch_index=owner['patch_index'],
            runtime_patch_index=owner['runtime_patch_index'], patch_sha256=owner['patch_sha256'],
            display_polyline=polyline, min_display_y=minimum,
            live_loop_foreground=loop_report['loop_min_display_y'] > minimum,
            integrated_terminal_loop_foreground=True))
    assert sorted(o['min_display_y'] for o in occurrences) == [160, 160, 1e15, 1e15]
    x0, y0, x1, y1 = row['source_union_box']
    crop = [max(0, x0-32), max(0, y0-32), x1+32, y1+32]
    width, height = crop[2]-crop[0], crop[3]-crop[1]
    background = Image.open(q.ROOT / qualification['background']['path']).convert('RGBA').crop(crop)
    loops = []
    for frame in loop_report['loop_frames']:
        layer = Image.new('RGBA', (width, height))
        layer.alpha_composite(q.rgba(frame), (frame['top_left'][0]-crop[0], frame['top_left'][1]-crop[1]))
        loops.append(layer)
    sheet = Image.new('RGB', (width*3, (height+28)*4), (24,24,24))
    draw = ImageDraw.Draw(sheet)
    frames = row['art']['states']['initial']['frames'] + row['art']['states']['transition']['frames']
    records = []
    for j, (phase, frame) in enumerate(zip(['initial', 'transition-0', 'transition-1', 'integrated terminal'], frames)):
        patch = Image.new('RGBA', (width,height))
        patch.alpha_composite(q.rgba(frame), (int(frame['top_left'][0])-crop[0], int(frame['top_left'][1])-crop[1]))
        base_patch = Image.alpha_composite(background, patch)
        loop_front = Image.alpha_composite(base_patch, loops[0])
        patch_front = Image.alpha_composite(Image.alpha_composite(background, loops[0]), patch)
        images = [base_patch, loop_front, patch_front if j < 3 else loop_front]
        labels = ['Backplate + patch', 'Loop foreground: minY160', 'Patch foreground: empty line' if j < 3 else 'Terminal inserted: loop front']
        for i, (label, image) in enumerate(zip(labels, images)):
            sheet.paste(image.convert('RGB'), (i*width, j*(height+28)+28))
            draw.text((i*width+3, j*(height+28)+3), phase, fill='white')
            draw.text((i*width+3, j*(height+28)+15), label, fill='white')
        mask = np.array(patch)[:,:,3] > 0
        records.append(dict(phase=phase, source=frame['image'], body_pixels=int(mask.sum()),
            original_loop_overlap_per_frame=[int((mask & (np.array(loop)[:,:,3]>0)).sum()) for loop in loops]))
    dest.mkdir()
    sheet.save(dest / 'source-composition.png')
    (dest / 'report.json').write_text(json.dumps(dict(status='SOURCE_ORDER_REFERENCE_PHYSICAL_COMPOSITION_PENDING',
        bindings=q.pin(q.BIND), loop_evidence=q.pin(loop_report_path), crop=crop,
        occurrences=occurrences, records=records, image=q.pin(dest/'source-composition.png'),
        recipe=q.pin(Path(__file__)), limits=[
            'The empty polyline receives minimumY sentinel1e15; preserve occurrence-specific live sort entries.',
            'Last transition frame can be inserted into background independently of the live FX row. The integrated terminal column shows that backplate contribution, not a held live-owner substitution.',
            'Original bodyalpha only; shadow-key pixels excluded. Independent frame combinations do not imply a second clock.',
            'Physical canopy05 sampler and exclusion domain remain pending; this diagram cannot qualify final geometry coverage.',
            'No canonical or source material changed.']), indent=2)+'\n')
    print(json.dumps(dict(occurrences=occurrences, records=records)), flush=True)


if __name__ == '__main__':
    main()
