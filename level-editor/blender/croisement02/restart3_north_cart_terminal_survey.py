"""Freeze a source-only role survey for the north cart's terminal artwork."""
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw
from scipy.ndimage import label, find_objects

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / 'work/croisement02-refinement'
DEST = OUT / 'restart3-north-cart/terminal-source-survey-v1'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    manifest = OUT / 'state-target-evidence/north-cart/manifest.json'
    source = json.loads(manifest.read_text())
    main_part, actors = source['parts']
    terminal = main_part['frames'][-1]
    pixels = np.array(Image.open(terminal['image']).convert('RGBA'))
    labels, _ = label(pixels[:, :, 3] > 0)
    components = []
    for i, bounds in enumerate(find_objects(labels), 1):
        components.append(dict(component=i, opaque_pixels=int((labels == i).sum()),
                               local_box=[bounds[1].start, bounds[0].start,
                                          bounds[1].stop, bounds[0].stop]))
    assert sorted(x['opaque_pixels'] for x in components) == [928, 11259]
    assert actors['frames'][-1]['opaque_pixels'] == 0
    # Broad visual regions are deliberately not authoritative per-material masks.
    roles = [
        dict(name='remaining striped roof and hanging side drapes', box=[78, 0, 166, 94],
             inference='Sagged/torn front-left roof margin and folded side drapes; hidden rear shell and thickness unknown.'),
        dict(name='broken upright front panel and foreground timbers', box=[0, 18, 93, 118],
             inference='Several disconnected-looking projected boards overlap the body; alpha alone cannot establish their joints or depth.'),
        dict(name='retained bed and dark under-cabin structure', box=[25, 79, 126, 120],
             inference='Separate opaque timber from dark shading before deriving support; no ground plane inferred from darkness.'),
        dict(name='near upright visible wheel', box=[99, 99, 134, 142],
             inference='Projected center approximately(117,121), elliptical spoked ring. Ground/contact must be solved against current receivers.'),
        dict(name='right lowered or displaced wheel', box=[157, 88, 177, 114],
             inference='Nearly horizontal projected ring around(166,100); physical axle attachment and settled orientation unproven.'),
        dict(name='separate box-like cargo and dark base', box=[220, 102, 252, 137],
             inference='Distinct disconnected opaque component; visible slatted top supports finite box hypothesis, dark base may include shadow.')]
    canvas = Image.fromarray(pixels).resize((1008, 568), Image.Resampling.NEAREST).convert('RGB')
    draw = ImageDraw.Draw(canvas)
    for i, role in enumerate(roles, 1):
        box = [v * 4 for v in role['box']]
        draw.rectangle(box, outline=(255, 80, 80), width=2)
        draw.text((box[0]+3, box[1]+3), str(i), fill='white', stroke_width=1, stroke_fill='black')
    DEST.mkdir(parents=True, exist_ok=True)
    canvas.save(DEST / 'terminal-role-regions.png')
    frames = [main_part['frames'][i] for i in [55, 60, 65, 70, 75, 80, 85, 90, 95, 99]]
    frames += [actors['frames'][i] for i in [0, 8, 16, 24, 28, 32]]
    files = [manifest, Path(__file__).resolve()]
    for frame in frames:
        assert sha(frame['image']) == frame['image_sha256']
        files.append(Path(frame['image']))
    record = dict(status='Source-only survey complete; no terminal physical candidate or motion approval',
                  source_manifest=str(manifest), terminal_frame=terminal,
                  global_top_left=[1202, 220], alpha_components=components,
                  roles=roles,
                  departing_sequence=dict(profile=actors['profile_id'],
                    visual_role='Horses and harness with trailing equipment; separate from persistent terminal wreck',
                    terminal_opaque_pixels=0, terminal_image_sha256=actors['frames'][-1]['image_sha256']),
                  observations=[
                      'The initial cabin candidate is not a valid terminal wreck: front timber breakage, sagged roof, wheel placement and detached cargo differ.',
                      'Main terminal artwork contains exactly two alpha components. This is not a physical object count: many wreck parts overlap in projection.',
                      'Boxes overlap intentionally and are visual survey regions, not exclusive source ownership masks.',
                      'No rigid motion, timing parity, world-space depth, contact, or hidden geometry is asserted by this source-only survey.',
                      'Initial front-closure-v4 and all south-cart frozen geometry remain unchanged.'
                  ],
                  proposed_next_checks=['Fit terminal source roof rim, front boards and two visible wheel rings independently.',
                    'Construct broken openings and displacements as geometry; preserve native RGB.',
                    'Prove contact against evaluated bank69ecb7 and ground16c638be; inspect native and all8 before root review.',
                    'Keep detached box and any shadow ambiguity separately scoped.'],
                  evidence_sha256={str(p):sha(p) for p in files})
    for name in ['terminal-role-regions.png','terminal-grid.png','part1-phases.png','breakup-source-phases.png']:
        p=DEST/name
        if p.exists(): record['evidence_sha256'][str(p)]=sha(p)
    path=DEST/'survey.json'
    assert not path.exists(), path
    path.write_text(json.dumps(record, indent=2)+'\n')
    print(path, sha(path))


if __name__ == '__main__':
    main()
