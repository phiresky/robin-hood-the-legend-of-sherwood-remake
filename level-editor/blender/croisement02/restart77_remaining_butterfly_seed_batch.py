"""Fit private initial physical pose hypotheses to six independent source loops."""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS', '1')
os.environ.setdefault('OMP_NUM_THREADS', '1')
import argparse
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
import restart21_butterfly07_global_registration_v2 as ref
from restart21_butterfly07_geometry_v2 import geometry, fixed_geometry

BASE = ref.ROOT / 'level-editor/work/croisement02-refinement'
PLAN = BASE / 'restart14-butterflies/all7-context-plan-v1/plan.json'
TEMPLATE = BASE / 'restart66-butterfly07-global-bright-fit-v1/fit.json'


def extent(sequence):
    sizes = []
    for frame in sequence['path']:
        rgba = np.asarray(Image.open(frame['source']).convert('RGBA'))
        y, x = np.nonzero(rgba[:, :, 3])
        sizes.append(max(x.max()-x.min()+1, y.max()-y.min()+1))
    return float(np.percentile(sizes, 95))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(exist_ok=False)
    plan = json.loads(PLAN.read_text())
    template = json.loads(TEMPLATE.read_text())
    template_sequence = next(s for s in plan['sequences'] if s['index'] == 14)
    summaries = []
    for sequence in plan['sequences']:
        index = sequence['index']
        if index == 14:
            continue
        scale = extent(sequence)/extent(template_sequence)
        def scaled_geometry(p):
            body, wings = geometry(p)
            return body*scale, [w*scale for w in wings]
        ref.geometry = scaled_geometry
        banks = []
        colors = []
        for phase, source in enumerate(sequence['path']):
            ctx = ref.context(source)
            candidates = [ref.evaluate(row['parameters'], ctx)
                          for row in template['rows'][::3]]
            candidates.sort(key=lambda row: row['loss'])
            banks.append(candidates[:6])
            rgba = np.asarray(Image.open(source['source']).convert('RGBA'))
            colors.extend(rgba[:, :, :3][rgba[:, :, 3] > 0].tolist())
        # A closed, continuous phase selection; no arbitrary seam at frame zero.
        best = None
        for first in range(6):
            costs = np.full(6, np.inf)
            costs[first] = banks[0][first]['loss']
            predecessors = []
            for phase in range(1, 99):
                total = costs[:, None] + np.array([
                    [ref.transition(a, b) for b in banks[phase]]
                    for a in banks[phase-1]])
                pred = total.argmin(axis=0)
                costs = total[pred, np.arange(6)] + [r['loss'] for r in banks[phase]]
                predecessors.append(pred)
            costs += [ref.transition(r, banks[0][first]) for r in banks[-1]]
            last = int(costs.argmin())
            if best is None or costs[last] < best[0]:
                selected = [last]
                for pred in reversed(predecessors):
                    selected.append(int(pred[selected[-1]]))
                best = (float(costs[last]), list(reversed(selected)))
        rows = []
        sheet = Image.new('RGB', (11*128, 9*128), '#eeeeee')
        draw = ImageDraw.Draw(sheet)
        for phase, source in enumerate(sequence['path']):
            row = dict(banks[phase][best[1][phase]], phase=phase, source=source)
            rows.append(row)
            rgba = Image.open(source['source']).convert('RGBA')
            # Source comparisons are deliberately RGB-independent pose evidence.
            image = Image.new('RGB', rgba.size, '#444444')
            image.paste(rgba, mask=rgba.getchannel('A'))
            image = image.resize((96, 96), Image.Resampling.NEAREST)
            ox, oy = (phase % 11)*128, (phase//11)*128
            sheet.paste(image, (ox, oy+24))
            draw.text((ox+2, oy+2), f'{phase}: -{row["missing"]} +{row["extra"]}', fill='black')
        destination = args.output/f'animation-{index:02}'
        destination.mkdir()
        sheet.save(destination/'own-source-phases.png')
        report = dict(status='PRIVATE_INITIAL_POSE_SEEDS_NOT_REVIEW_READY',
            animation_index=index, plan_sha256=ref.sha(PLAN),
            pose_template_sha256=ref.sha(TEMPLATE), recipe_sha256=ref.sha(Path(__file__)),
            cycle_ticks=198, frame_duration_ticks=2,
            geometry_template=fixed_geometry(), fixed_uniform_scale=scale,
            scale_basis='Ratio of own and template 95th percentile alpha extents; inferred anatomy, not approval.',
            material_rgb_own_source_median=np.median(colors, axis=0).tolist(),
            rows=rows, missing_pixels=sum(r['missing'] for r in rows),
            bright_missing=sum(r['bright_missing'] for r in rows),
            extra_pixels=sum(r['extra'] for r in rows),
            source_pixels=sum(r['source_pixels'] for r in rows),
            limitations=['Pose bank initialization only; independent own-art optimization and anatomy review remain.',
                'No borrowed texture pixels. Own RGB observations have not yet been projected to surfaces.',
                'Depth, body attitude and shared rest anatomy are inferred; no collision or exported playback proof.',
                'No geometry approval or publication eligibility is implied.'])
        (destination/'fit.json').write_text(json.dumps(report, indent=2)+'\n')
        summary = {key:report[key] for key in ('animation_index','fixed_uniform_scale','missing_pixels','bright_missing','extra_pixels','source_pixels')}
        summary['fit_sha256'] = ref.sha(destination/'fit.json')
        summaries.append(summary)
        print(json.dumps(summary), flush=True)
    (args.output/'batch.json').write_text(json.dumps(dict(status='SIX_OWN_SOURCE_INITIALIZATIONS', sequences=summaries), indent=2)+'\n')


if __name__ == '__main__':
    main()
