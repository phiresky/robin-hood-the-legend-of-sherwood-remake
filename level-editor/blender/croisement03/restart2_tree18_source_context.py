"""CPU-only Tree18 source audit and provisional wood construction constraints."""
import argparse
import hashlib
import json
import math
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/croisement03-refinement'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    out = args.output
    out.mkdir(parents=True, exist_ok=False)
    pins = {}

    def pin(path):
        pins[str(path)] = sha(path)
        return path

    level = json.loads(pin(BASE / 'baseline/Croisement03.rhp.json').read_text())
    inventory = json.loads(pin(BASE / 'source-inventory.json').read_text())
    catalog = json.loads(pin(BASE / 'catalog.json').read_text())
    geometry_inventory = json.loads(pin(BASE / 'inventory/inventory.json').read_text())
    source = Image.open(pin(BASE / 'baseline/covered.png')).convert('RGBA')
    animation = next(a for a in inventory['animations'] if a['index'] == 4)
    assert animation['profile'] == 'Croisement03 - Arbre05'
    assert len(animation['frames']) == 14
    group = next(g for g in catalog['groups'] if g['id'] == 'croisement03-tree-18')
    assert group['wood_mask'] == 18
    assert {p['obstacle'] for p in group['parts']} == {33, 34}

    def mask(index):
        im = Image.open(pin(BASE / f'baseline/masks/{index:06}.png'))
        result = Image.new('L', source.size)
        result.paste(im, tuple(level['masks'][index]['box_top_left']))
        return np.array(result) > 0

    wood, occ0, occ1 = (mask(i) for i in [18, 120, 121])
    bank = mask(96) | mask(108) | mask(109)
    union = np.zeros_like(wood)
    frames = []
    crop = (220, 170, 610, 575)
    sheet = Image.new('RGB', (390 * 7, 430 * 2), '#303030')
    draw = ImageDraw.Draw(sheet)
    for phase, row in enumerate(animation['frames']):
        rawpath, path = Path(row['source']), Path(row['image'])
        assert sha(pin(rawpath)) == row['sha256']
        raw = np.array(Image.open(rawpath).convert('RGBA'))
        key = np.all(raw[:, :, :3] == [0, 251, 0], axis=2)
        expected = raw.copy()
        expected[key] = [0, 0, 0, 0]
        normalized = Image.open(pin(path)).convert('RGBA')
        assert np.array_equal(expected, np.array(normalized)), f'RGBA mismatch phase {phase}'
        assert list(normalized.size) == row['bbox'][2:]
        canvas = Image.new('RGBA', source.size)
        canvas.paste(normalized, tuple(row['bbox'][:2]))
        alpha = np.array(canvas)[:, :, 3] > 0
        union |= alpha
        frames.append({'phase': phase, 'bbox_xywh': row['bbox'], 'delay': row['delay'],
                       'raw': str(rawpath), 'raw_sha256': sha(rawpath),
                       'normalized': str(path), 'normalized_sha256': sha(path),
                       'rgba_exact': True, 'key_pixels_removed': int(key.sum()),
                       'alpha_pixels': int(alpha.sum()),
                       'outside_occupancy120': int((alpha & ~occ0).sum()),
                       'woodmask_overlap': int((alpha & wood).sum()),
                       'bank_overlap': int((alpha & bank).sum())})
        # This is an explicit composite state, not a claim that covered.png is phase zero.
        composite = Image.alpha_composite(source, canvas).convert('RGB').crop(crop)
        x, y = (phase % 7) * 390, (phase // 7) * 430
        sheet.paste(composite, (x, y + 25))
        draw.text((x + 4, y + 5), f'Native phase {phase:02}: base + exact RGBA', fill='white')
    sheet.save(out / 'all14-explicit-composites.png')
    Image.fromarray(union.astype('uint8') * 255).save(out / 'native-alpha-union.png')
    source.convert('RGB').crop(crop).resize((780, 810), Image.Resampling.NEAREST).save(out / 'source-context-2x.png')

    # Source cross-sections are geometry proposals, never bark ownership masks.
    # Branch sections are oblique to their local projected axes.
    proposals = [
        ('main_lower', 33, [((421, 474), (460, 474)), ((418, 454), (460, 454)),
                           ((418, 433), (458, 433)), ((422, 413), (455, 413)),
                           ((417, 395), (455, 395))], 3),
        ('main_upper', 34, [((430, 378), (454, 378)), ((438, 355), (458, 355)),
                           ((440, 332), (459, 332)), ((441, 312), (460, 312)),
                           ((443, 292), (460, 292)), ((447, 275), (461, 275))], 3),
        ('left_fork', None, [((422, 382), (418, 397)), ((396, 376), (392, 390)),
                            ((369, 362), (360, 374)), ((351, 342), (343, 354)),
                            ((321, 336), (317, 344)), ((291, 330), (289, 335))], 4),
        ('right_fork', None, [((452, 367), (468, 376)), ((463, 346), (474, 354)),
                             ((482, 324), (491, 333)), ((505, 305), (511, 312)),
                             ((526, 290), (531, 296))], 4),
    ]
    annotated = source.convert('RGB')
    painter = ImageDraw.Draw(annotated)
    colors = ['#00ffff', '#ff55ff', '#55aaff', '#ffffff']
    records = []
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
    depth_reference = sum(p['y'] for p in level['sight_obstacles'][33]['points']) / 4
    for (name, obstacle, samples, uncertainty), color in zip(proposals, colors):
        sections = []
        for number, (a, b) in enumerate(samples):
            center = [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2]
            width = math.dist(a, b)
            world = [center[0], -depth_reference / sine, (depth_reference - center[1]) / cosine]
            assert abs(-world[1] * sine - world[2] * cosine - center[1]) < 1e-9
            sections.append({'endpoints_source_xy': [a, b], 'center_source_xy': center,
                             'center_world_hypothesis': world, 'projected_width_pixels': width,
                             'initial_cross_section_depth': width, 'uncertainty_pixels': uncertainty,
                             'source_ownership_assigned': False})
            painter.line([a, b], fill=color, width=1)
            painter.text((b[0] + 2, b[1]), f'{name[0]}{number}', fill=color)
        records.append({'track': name, 'native_obstacle_context': obstacle, 'sections': sections,
                        'status': 'MANUAL_APPROXIMATE_TRACE_REQUIRES_SAVED_CAMERA_REVIEW'})
    annotated.crop((275, 265, 545, 555)).resize((810, 870), Image.Resampling.NEAREST).save(out / 'proposed-wood-sections-3x.png')

    references = []
    for name in ['leicester-southeast-cottage-tree', 'leicester-moat-bank-tree']:
        path = pin(ROOT / f'level-editor/work/leicester-refinement/parallel-wave-2/audit-images/{name}.jpg')
        references.append({'path': str(path), 'sha256': sha(path), 'use': 'Construction and crown articulation only; no RGB copied.'})
    canonical = ROOT / 'level-editor/library/3d-assets/croisement03/croisement03-tree-18/asset.json'
    if canonical.exists():
        pin(canonical)
    inventory_rows = [o for o in geometry_inventory['objects'] if o.get('source_node') in ['building-033', 'building-034']]
    assert len(inventory_rows) == 2
    report = {
        'status': 'HOLD_PRIVATE_CPU_SOURCE_AND_CONSTRUCTION_PLAN', 'pins': pins,
        'catalog_group': group, 'native_sprite': animation['sprite'],
        'native_display_polyline': animation['display_polyline'], 'frames': frames,
        'native_alpha_union_pixels': int(union.sum()),
        'native_alpha_union_bank_overlap': int((union & bank).sum()),
        'woodmask_bank_overlap': int((wood & bank).sum()),
        'occupancy_bank_overlap': int(((occ0 | occ1) & bank).sum()),
        'occupancy_layers_identical': bool(np.array_equal(occ0, occ1)),
        'native_alpha_union_outside_occupancy120': int((union & ~occ0).sum()),
        'woodmask_pixels': int(wood.sum()), 'inventory_obstacles': inventory_rows,
        'gameplay_obstacles': {str(i): level['sight_obstacles'][i] for i in [33, 34]},
        'canonical_tree18_descriptor_exists': canonical.exists(),
        'source_roles': {
            'observed_animation': 'One existing owner for all fourteen exact keyed frames; preserve foreground precedence.',
            'woodmask18': 'Gameplay occupancy and catalog association, not complete material ownership.',
            'bark': 'Visible trunk and branch proposals only; exclude leaf crossings with exact phase provenance before any projection.',
            'ivy': 'Green material on lower-left trunk and roots is visibly distinct; ownership and depth remain unresolved.',
            'bank': 'Layered gray/brown surfaces left of trunk remain bank context; never flatten woodmask into bark RGB.',
            'static_foliage': 'No broad static foliage ownership assignment made.'},
        'wood_tracks': records, 'references': references,
        'geometry_recipe': {
            'projection': 'x=world.x; y=-world.y*sin(35deg)-world.z*cos(35deg)',
            'depth_reference_native_y': depth_reference,
            'depth_status': 'Obstacle mean is a starting hypothesis only; physical depth is unknown.',
            'construction': 'Build tapered closed sweeps along four tracks with initially round cross-sections. Attach fork junctions within main trunk using a reviewed union; retain source node provenance for native obstacles 33 and 34.',
            'budget': {'max_wood_vertices': 2500, 'max_wood_triangles': 5000, 'ring_sides': 12},
            'root': 'Do not terminate at y474, last reliable exposed bark. Infer continuation behind ivy to local ground; native ground anchor y~500 is only gameplay context. Exposed left root-like diagonal around x350-397/y495-536 needs a separate trace and contact review.',
            'branches': 'Upper caps continue behind leaves rather than ending at last sampled section; fork endpoints are uncertain. Branch cross-sections use local perpendicular frames, not horizontal sheets.',
            'crown': 'Preserve full animation under one owner. Hidden crown volume and depth must be at least width absent stronger evidence; keep inferred geometry behind observed rays. No static pixel reassignment or inferred texture yet.'},
        'next_builder': [
            'Build private closed wood sweeps from these provisional sections; treat world-depth values as adjustable along native rays.',
            'Reopen candidate and original baseline together; verify exact transform, source projection, native obstacle provenance and local ground/root contact.',
            'Render actual saved eight views and native crop with current bank52/53/54 and plateaus94-97, whole native animation4 and adjacent observed layers.',
            'Audit all fourteen phases independently: foreign bank/tree source hits, coverage holes, duplicate ownership and transparent-surface convergence.',
            'Inspect source-only bark and ivy domains separately; no barycentric/nearest-receiver reassignment of misses.',
            'Review complete inferred crown and root continuation before geometry approval or texture fill.'],
        'limits': ['No Blender scene, API, browser, canonical catalog, source texture or gameplay data changed.',
                   'No saved model produced; these manual traces are not validated geometry or material masks.',
                   'All fourteen composite images are explicit base-plus-frame states, not identification of the phase baked into covered artwork.',
                   'Native mask overlap is not physical contact or material ownership proof.',
                   'The only external tree references inspected are the two authorized Leicester trees.']}
    (out / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: report[k] for k in ['status', 'native_alpha_union_pixels', 'native_alpha_union_bank_overlap', 'woodmask_bank_overlap', 'occupancy_bank_overlap']}))


if __name__ == '__main__':
    main()
