"""Inventory cumulative ground gaps and prepare review-only, source-locked fill inputs."""
import json
from pathlib import Path
import sys

import numpy as np
from PIL import Image, ImageDraw

sys.path[:0] = [str(Path(__file__).parent), str(Path(__file__).resolve().parents[2] / 'refinement/blender')]
from catalog import OUT
from evidence_io import sha, write_json


def main():
    base = OUT / 'ground-receiver-review-v5/reference'
    current = OUT / 'restart2-ground38/cumulative848-v1'
    output = OUT / 'restart2-ground-completion/preparation-v1'
    if output.exists():
        raise FileExistsError(output)
    output.mkdir(parents=True)
    load = lambda p: np.asarray(Image.open(p).convert('L')) > 0
    known = load(current / 'ground-observed-domain.png')
    first = load(base / 'ground-first-hit.png')
    states = load(base / 'mission-frame-reservations.png')
    source = np.asarray(Image.open(base / 'source.png').convert('RGB'))
    atlas = np.asarray(Image.open(current / 'observed-neutral.png').convert('RGB'))
    if not np.array_equal(atlas[known], source[known]) or (known & ~first).any():
        raise ValueError('Known native ground or first-hit authority changed')
    hidden = first & ~known
    deferred = hidden & states
    editable = hidden & ~states
    relief = ~first
    domains = dict(known=known, eligible_hidden_floor=editable, deferred_state_floor=deferred,
                   separate_relief=relief, all_unknown=~known, state_reservation=states)
    if not np.all(known.astype(int) + editable + deferred + relief == 1):
        raise ValueError('Ground partition must be complete and disjoint')
    for name, data in domains.items():
        Image.fromarray(data.astype('uint8') * 255).save(output / (name + '.png'))
    Image.fromarray(atlas).convert('RGBA').save(output / 'input.png')
    mask = np.full((*known.shape, 4), 255, dtype='uint8')
    mask[editable, 3] = 0
    Image.fromarray(mask).save(output / 'mask.png')
    Image.new('RGBA', (1792, 1152), (127, 127, 127, 255)).save(output / 'solid.png')
    overlay = np.asarray(atlas, dtype=float) * .35
    for data, color in [(known, (20, 200, 200)), (editable, (230, 165, 30)),
                        (deferred, (195, 55, 180)), (relief, (65, 80, 110))]:
        overlay[data] += np.asarray(color) * .65
    Image.fromarray(overlay.astype('uint8')).save(output / 'domain-review.png')
    labels = [('forest-floor', (292, 458, 48, 48)), ('dirt-path', (410, 956, 64, 64)),
              ('meadow-grass', (736, 1050, 64, 64)), ('shaded-floor', (1650, 324, 32, 32))]
    refs = []
    sheet = Image.new('RGB', (1024, 292), (45, 45, 45))
    draw = ImageDraw.Draw(sheet)
    (output / 'references').mkdir()
    for i, (role, (x, y, w, h)) in enumerate(labels):
        if not (known & ~states)[y:y+h, x:x+w].all():
            raise ValueError('Supplementary crop contains unknown or state-reserved pixels')
        image = Image.fromarray(source[y:y+h, x:x+w]).convert('RGBA')
        path = output / 'references' / (role + '.png')
        image.save(path)
        refs.append(dict(source='input', file=str(path), sha256=sha(path),
                         crop=dict(left=x, top=y, width=w, height=h), scale=1, role=role,
                         native_pixels=w*h, all_known=True, state_overlap=0))
        sheet.paste(image.resize((256, 256), Image.Resampling.NEAREST), (i*256, 36))
        draw.text((i*256+8, 12), role + ' (native pixels)', fill='white')
    sheet.save(output / 'reference-review.png')
    write_json(output / 'auxiliary-references.json', dict(version=1,
        input_sha256=sha(output / 'input.png'), lighting_sha256=sha(output / 'solid.png'), references=refs))
    state_proof = base / 'state-source-preservation.json'
    state_data = json.loads(state_proof.read_text())
    frame_hashes = {}
    for row in state_data['frames']:
        path = Path(row['image'])
        if str(path) not in frame_hashes:
            frame_hashes[str(path)] = sha(path)
        if frame_hashes[str(path)] != row['sha256']:
            raise ValueError('Mission-state frame evidence changed')
    if sha(base / 'mission-frame-reservations.png') != state_data['reservation_sha256']:
        raise ValueError('Mission-state reservation changed')
    geometry = json.loads((current / 'geometry-before.json').read_text())
    matrix = np.asarray(geometry['matrix'])
    world = (matrix @ np.column_stack((geometry['vertices'], np.ones(4))).T).T[:, :3]
    sin, cos = np.sin(np.deg2rad(35)), np.cos(np.deg2rad(35))
    projected = np.column_stack((world[:, 0], -world[:, 1]*sin-world[:, 2]*cos))
    if not np.allclose(projected.min(axis=0), (0, 0), atol=.001) or not np.allclose(projected.max(axis=0), (1792, 1152), atol=.001):
        raise ValueError('Existing ground footprint is not the exact in-map rectangle')
    feedback = json.loads((OUT / 'user-feedback.json').read_text())['records']
    approvals = [r for r in feedback if r.get('asset_id') == 'croisement02-ground-receiver'
                 and r.get('decision') == 'approved']
    manifest = dict(version=1, status='review-only preparation; no API approval',
        model=str(current / 'model.blend'), model_sha256=sha(current / 'model.blend'),
        native_source=str(base / 'source.png'), source_sha256=sha(base / 'source.png'),
        cumulative_source_domain=str(current / 'source-domain-manifest.json'),
        cumulative_source_domain_sha256=sha(current / 'source-domain-manifest.json'),
        dimensions=[1792, 1152], projection_kind='planar-atlas',
        counts={name:int(data.sum()) for name,data in domains.items()},
        out_of_map_receiver_pixels=0, projected_receiver_corners=projected.tolist(),
        interpretation={
            'eligible_hidden_floor':'Proposed fill scope only: unseen in-map ground, outside all state reservations. Requires exact input/geometry/domain approval before API.',
            'deferred_state_floor':'Unknown base floor overlapping mission frame reservations; retain for separate state-aware review.',
            'separate_relief':'Under separately owned bank/relief first hits, not this initial floor fill request.',
            'out_of_map':'No out-of-map geometry exists in this flat receiver. Out-of-map crown/rock continuations remain their own asset responsibilities.'},
        state_source=dict(path=str(state_proof),sha256=sha(state_proof),records=len(state_data['frames']),
                          verified_image_files=len(frame_hashes), native_patch_records=len(state_data['native_patches']),
                          terminal_ground=state_data['barrier_ground_requirement'], all_images_untouched=True),
        prerequisites=dict(ground_geometry_user_approval_records=approvals, exact_atlas_user_approval=False,
                           texture_api_eligible=False, root_source_restoration_review='PASS; does not imply user texture-input approval',
                           current_limits=['Current first-hit mask covers bank and ground only; final assembled-scene source ownership audit remains pending.',
                                           'Canonical source-role refresh is in progress; this packet binds cumulative848 rather than silently following moving selectors.']),
        invariants=dict(known_rgb_exact=True, editable_known_overlap=0, editable_state_overlap=0,
                        editable_relief_overlap=0, geometry_changed=False, api_calls=0),
        files={p.name:sha(p) for p in output.glob('*.png')}, supplementary_references=refs)
    write_json(output / 'inventory.json', manifest)
    (output / 'material-prompt.txt').write_text(
        'Complete only the editable unseen in-map ground in this single planar atlas. Continue the native forest floor, soil paths and meadow at exact pixel scale using the supplied own-map material crops. Preserve all known source RGB including painted ground shadows. Preserve state-reserved regions and separately owned bank/relief areas. Do not reconstruct removed trees, shrubs, grasses as standing plants, logs, fences, buildings, rocks or props. Do not extend the map or invent new raised relief. Keep1792x1152 dimensions and exact coordinates. This is a preparation draft, not authorization to generate.\n')
    print(json.dumps(dict(output=str(output),counts=manifest['counts'],api_eligible=False)))


if __name__ == '__main__':
    main()
