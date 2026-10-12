"""Freeze mission endpoint artwork and construction inputs; never publish assets."""
import hashlib
import html
import json
import os
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[2]
REPO = ROOT.parent
BASE = ROOT / 'work/croisement03-refinement'
OUT = BASE / 'mission-endpoint-preparation-oct12-v1'
DATA = REPO / 'datadirs/fullgame_gog_hackable/Data'
LIB = ROOT / 'library/game-data/Data'
PINS = {}
CACHE = {}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def pin(path):
    PINS[str(path.relative_to(REPO))] = sha(path)
    return PINS[str(path.relative_to(REPO))]


def read(path):
    pin(path)
    return json.loads(path.read_text())


def image(path, legacy=False):
    digest = pin(path)
    key = (digest, legacy)
    if key in CACHE:
        return CACHE[key]
    dest = OUT / 'raw' / (digest + '.png')
    if not dest.exists():
        os.link(path, dest)
    im = Image.open(path).convert('RGBA')
    a = np.array(im)
    packed = ((a[:, :, 0].astype('uint16') >> 3) << 11) | ((a[:, :, 1].astype('uint16') >> 2) << 5) | (a[:, :, 2].astype('uint16') >> 3)
    keyed = (packed == 0x07C0) if legacy else np.zeros(a.shape[:2], dtype=bool)
    shadow = (packed == 0x001F) & (a[:, :, 3] >= 128) if legacy else np.zeros(a.shape[:2], dtype=bool)
    a[keyed | (a[:, :, 3] < 128), 3] = 0
    display = OUT / 'display' / (digest + ('-keyed' if legacy else '') + '.png')
    Image.fromarray(a).save(display)
    row = {'source_path': str(path.relative_to(REPO)), 'raw_path': str(dest.relative_to(OUT)), 'sha256': digest,
           'display_path': str(display.relative_to(OUT)), 'display_sha256': sha(display), 'size': list(im.size),
           'raw_alpha_bbox': im.getchannel('A').getbbox(), 'effective_alpha_bbox': Image.fromarray(a[:, :, 3]).getbbox(),
           'legacy_color_keys': legacy, 'keyed_pixels': int(keyed.sum()), 'shadow_key_pixels': int(shadow.sum()),
           'normalization': 'Only alpha for RGB565 0x07c0 and alpha<128 is cleared; all RGB bytes retained. Blue shadow sentinel remains labelled, never treated as solid anatomy.'}
    CACHE[key] = row
    return row


def profile(bank, name):
    folder = DATA / 'Animations/Day' / (bank + '.rhs.d')
    manifest = read(folder / 'manifest.json')
    p = next(p for p in manifest['profiles'] if p['name'] == name)
    rows = []
    for r in p['rows']:
        frames = []
        for f in r['frames']:
            path = folder / name / r['path'] / f['file']
            if not path.exists() and len(manifest['profiles']) == 1:
                path = folder / r['path'] / f['file']
            if not path.exists():
                raise FileNotFoundError(path)
            frames.append(dict(f, **image(path, manifest.get('pixel_format') == 'legacy_color_keys')))
        rows.append(dict(r, frames=frames))
    initial = LIB / 'Animations/Day' / (bank + '.rhs.d/manifest.json')
    ld = read(initial) if initial.exists() else {}
    lp = next((x for x in ld.get('profiles', []) if x['name'] == name), {})
    signature_data = [{k: v for k, v in r.items() if k != 'frames'} | {'frames': [{k: f[k] for k in ['sha256', 'offset_x', 'offset_y', 'delay', 'sound_id']} for f in r['frames']]} for r in rows]
    return {'bank': bank, 'name': name, 'profile_metadata': {k: v for k, v in p.items() if k != 'rows'},
            'rows': rows, 'full_actions': [r['action_id'] for r in rows],
            'library_actions': [r['action_id'] for r in lp.get('rows', [])],
            'art_signature': hashlib.sha256(json.dumps(signature_data, sort_keys=True).encode()).hexdigest()}


def main():
    for d in ['raw', 'display', 'scripts', 'sheets']:
        (OUT / d).mkdir(parents=True, exist_ok=True)
    scope = read(BASE / 'state-scope.json')
    source = read(BASE / 'source-inventory.json')
    level = read(LIB / 'Levels/Croisement03.rhp.json')
    missions = sorted({r['mission'] for records in scope['profile_families'].values() for r in records})
    md = {m: read(LIB / 'Levels' / (m + '.rhm.json')) for m in missions}
    families = []
    for name, records in scope['profile_families'].items():
        bank = records[0]['sprite']['frame_profile_name']
        p = profile(bank, name)
        p['instances'] = []
        for r in records:
            i = int(r['id'].rsplit('-', 1)[1])
            raw = md[r['mission']]['mission_patches'][i]
            assert raw['element_fx']['sprite'] == r['sprite']
            # Exact raw gameplay and source placement remain per occurrence.
            p['instances'].append({'id': r['id'], 'mission': r['mission'], 'mission_patch_index': i,
                                   'runtime_patch_index': r['runtime_patch_index'], 'source': raw,
                                   'extracted_states': r['states']})
            for state in r['states'].values():
                for f in state['frames']:
                    path = BASE / 'source-states' / f['image']
                    assert pin(path) == f['sha256']
        families.append(p)
    targets = {}
    script_records = []
    for mission, m in md.items():
        for suffix in ['scb.ts', 'scb.json']:
            path = DATA / 'Levels' / (mission + '.' + suffix)
            digest = pin(path)
            dest = OUT / 'scripts' / path.name
            if not dest.exists():
                os.link(path, dest)
            script_records.append({'mission': mission, 'path': str(dest.relative_to(OUT)), 'sha256': digest})
        text = (DATA / 'Levels' / (mission + '.scb.ts')).read_text()
        calls = [{'line': i + 1, 'text': line.strip()} for i, line in enumerate(text.splitlines())
                 if any(word in line for word in ['ApplyPatch(', 'UnApplyPatch(', 'RecordPlayAnim', 'RecordTimer(', 'GetActorScript(', 'GetPatchScript('])]
        (OUT / 'scripts' / (mission + '.calls.json')).write_text(json.dumps(calls, indent=2) + '\n')
        for i, target in enumerate(m['targets']):
            key = target['filename'] + ':' + target['profile_name']
            if key not in targets:
                targets[key] = profile(target['filename'], target['profile_name'])
                targets[key]['instances'] = []
            targets[key]['instances'].append({'mission': mission, 'target_index': i, 'source': target,
                                              'actor_index_binding': 'Resolve against exact mission element order; target_index is not GetActorScript index.'})
    ambient = []
    for a in source['animations']:
        ambient.append({'index': a['index'], 'kind': a['kind'], 'sprite': a['sprite'], 'frame_count': len(a['frames']),
                        'scope': 'Ambient inventory only; independent static-tree ownership retained.',
                        'frames': [dict(f, source_pin=pin(Path(f['source']))) for f in a['frames']]})
    report = {'status': 'SOURCE_PREPARATION_ONLY_NO_GEOMETRY_APPROVAL', 'map': 'Croisement03', 'missions': missions,
              'patch_occurrences': sum(len(f['instances']) for f in families), 'patch_families': families,
              'targets': list(targets.values()), 'ambient': ambient, 'native_map_patches': level['patches'],
              'native_scripts': script_records, 'source_pins': PINS,
              'semantics': ['RGB565 0x07c0 is transparent even when extracted PNG alpha is255; raw pixels remain frozen.',
                            'Shadow-key pixels are separate rendering metadata, not opaque mesh anatomy.',
                            'Applied background integration uses last transition frame; an absent final row does not imply absent integrated terrain.',
                            'Script actor and patch indices are separate from local target/mission-patch indices; raw order and complete script files retained.',
                            'Action160 rows are collected without inventing coupling or approval. Nine map patches remain distinct gameplay records.']}
    assert report['patch_occurrences'] == 101 and len(ambient) == 15 and len(level['patches']) == 9
    for path, digest in PINS.items():
        assert sha(REPO / path) == digest
    (OUT / 'inventory.json').write_text(json.dumps(report, indent=2) + '\n')
    cards = []
    for number, p in enumerate(families + list(targets.values())):
        samples = [(r, i, r['frames'][i]) for r in p['rows'] for i in sorted({0, len(r['frames']) // 2, len(r['frames']) - 1}) if r['frames']]
        width = 1000
        sheet = Image.new('RGB', (width, 60 + 280 * ((len(samples) + 3) // 4)), '#303030')
        draw = ImageDraw.Draw(sheet)
        draw.text((12, 12), p['name'] + ' | exact native direction | source-only, unapproved', fill='white')
        for n, (row, index, f) in enumerate(samples):
            x, y = (n % 4) * 250, 50 + (n // 4) * 280
            im = Image.open(OUT / f['display_path']).convert('RGBA')
            im.thumbnail((240, 225), Image.Resampling.NEAREST)
            sheet.paste(im, (x + 5, y + 36), im)
            draw.text((x + 5, y), f"action{row['action_id']} f{index} delay{f['delay']}", fill='white')
            draw.text((x + 5, y + 16), str([f['offset_x'], f['offset_y']]), fill='#cccccc')
        path = OUT / 'sheets' / f'{number:02d}.jpg'
        sheet.save(path, quality=92)
        p['review_sheet'] = str(path.relative_to(OUT))
        cards.append('<section><h2>' + html.escape(p['name']) + '</h2><p>Full actions ' + str(p['full_actions']) + '; library actions ' + str(p['library_actions']) + '</p><img src="' + p['review_sheet'] + '"></section>')
    (OUT / 'inventory.json').write_text(json.dumps(report, indent=2) + '\n')
    (OUT / 'index.html').write_text('<!doctype html><meta charset="utf-8"><title>Crossings03 endpoint sources</title><style>body{background:#222;color:white;font:16px sans-serif}img{max-width:100%}section{border-top:1px solid #888;padding:16px}</style><h1>Crossings03 endpoint sources</h1><p>Source artwork only, no geometry approval. Raw PNGs and all action rows retained. Display copies normalize only transparent color-key alpha; shadow keys remain metadata, not physical surfaces.</p>' + ''.join(cards))
    make_recipes(report)
    print(json.dumps({'patches': report['patch_occurrences'], 'patch_families': len(families), 'target_profiles': len(targets), 'source_pins': len(PINS), 'images': len(CACHE), 'inventory_sha256': sha(OUT / 'inventory.json')}))


def make_recipes(report):
    definitions = [
        ('log-trap', 1, ['buches'], 'Build the action0 b1 stacked logs first, then a separate b3 terminal spread. b2 is an intermediate handoff, not a permanent second pile.',
         ['Trace every visible log axis/end in b1 action0; count and fit the source stack rather than transferring the approved C2 log count.',
          'Keep b1, b2 and b3 target origins independent. b1 action160 has9 frames and ends empty; b2 has7 frames, starts with23-delay empty frame and ends empty; b3 has18 frames, starts with38-delay empty frame and ends visible.',
          'Reconstruct the final split pile from b3 frame17; do not combine visible b1 and b3 simultaneously at the endpoint.',
          'Separate buches_ombre receiver from solid logs. Preserve its30-frame transition and integration flag per mission.'],
         'level-editor/library/mission-states/croisement02/contracts/Tac19_FoB_EC-log-trap.json'),
        ('net01', 2, ['piege01'], 'Build the initial leaf-covered ground net and suspension, then distinct empty/occupied hanging envelopes with counterweight.',
         ['Target piege01h initial ground net disappears in its single empty action160 frame; do not retain an initial billboard under the raised net.',
          'Patch e and i have34 transition and14 final frames; construct separate occupied and empty envelope silhouettes. The visible cylindrical counterweight and rope remain separate solids.',
          'Target position[1203,392] minus center[120,180] equals patch origin[1083,212]; use this exact common art origin before adding depth.',
          'Ground g has33 transition frames plus one final; retain leaf debris and receiver contact separately from the hanging body.'],
         'level-editor/library/mission-states/croisement02/contracts/Tac02_FoB_EC-net-01-empty.json'),
        ('net02', 3, ['piege02'], 'Reconstruct the second installation independently; its net and counterweight silhouettes differ from net01.',
         ['Target position[859,570] minus center[70,140] equals patch origin[789,430].',
          'Preserve h disappearing action160, e/i34 transition+14 final frames, and g33 transition+1 final. The g terminal and final frames have different effective widths; do not alias by appearance.',
          'Use own rope/counterweight/net contour; never scale net01 geometry and assume source parity.'],
         'level-editor/library/mission-states/croisement02/contracts/Tac02_FoB_EC-net-01-empty.json'),
        ('rock-trap', 4, ['rocher01'], 'Model the initial cluster and terminal separated stones from target action160, with a distinct ground receiver.',
         ['Two target records per mission can share the same profile and position while retaining different script classes; keep both native records until timing ownership is resolved.',
          'Target action160 has45 frames; reconstruct initial and last visible frame44. Ground receiver has44 transition frames and a final frame, so clocks are not interchangeable.',
          'Align target and shadow receiver through their own centers and offsets. Preserve jagged rock silhouettes and infer only unseen depth.'],
         'level-editor/library/mission-states/croisement02/contracts/rock-trap.json'),
        ('holes', 5, [' - hole'], 'One reusable rim/leaf construction with eight independently fitted terrain sites,19 mission occurrences.',
         ['Trace normalized initial leaf footprint and30-frame transition; no final action row exists.',
          'Compute aperture and replacement terrain per source placement, including receiver slope. Last transition can integrate into background; absent final animation is not absent terrain.',
          'Keep per-instance masks, flags and layer changes; never union restoration rectangles across unrelated controls.'],
         'level-editor/work/croisement02-refinement/restart47-explicit-rgb-sampling-v1/holes/integration-focused.json'),
        ('hiding-mounds', 6, ['hiding Pc'], 'Reusable leaf-clump construction with25 source origins and27 mission occurrences; fit source-visible silhouette per site.',
         ['Initial compact mound becomes scattered leaves over15 transition frames, not a character body.',
          'Keep raw delay0 in first transition frame; timing normalization is a runtime concern.',
          'Several sprite origins are outside the image; evaluate actual offset bounds before classifying geometry as off-map. Infer missing physical continuation only where clipped source requires it.'],
         'level-editor/work/croisement02-refinement/restart47-explicit-rgb-sampling-v1/mounds/reader-config.json'),
        ('archer-covers', 7, ['hidden archer'], 'Four distinct ivy/brush cover variants, not four archer bodies.',
         ['Reconstruct source initial cover and lowered three-frame transition shapes at each fixed origin.',
          'Maintain01/02/03/04 distinct shapes and offsets. Locate associated static stump/log ownership before removing or replacing any existing geometry.',
          'The revealed central opening must remain open; do not fill it with inferred foliage or duplicate native cover.'],
         'level-editor/work/croisement02-refinement/restart52-climbing-browser-v1/handoff.json'),
        ('cart05', 8, ['chariot05'], 'Build the striped covered cart body, wheels and terminal wreck as separate source-owned parts.',
         ['Main action160 begins with moving cart and horse and ends empty; b2 retains collapsed canopy/body and b3 retains detached wheel/block.',
          'Action0 is empty except b4 residual source fragment; do not invent a persistent initial complete cart.',
          'Keep the horse and source shadows separate from reusable cart geometry. Resolve script sequencing before combining100/73/36/1 frame rows.'],
         'level-editor/work/croisement02-refinement/restart3-north-cart/terminal-source-inputs-v1/handoff.json'),
        ('cart04', 9, ['chariot04'], 'Reconstruct own wreck/body and suspended log mechanism; it is not a material variant of cart05.',
         ['Main action160 has100 frames with a different terminal wreck. b2 has33 frames including rope/log and debris; b3 retains a hanging log through30 frames; b4 has81 frames and ends empty.',
          'All initial action0 rows are empty. Preserve own action offsets; do not place each image at the target anchor without subtracting center.',
          'Split horse, rope, suspended log, cart body and transient debris by source ownership; no runtime motion inferred merely from shared action160.'],
         'level-editor/work/croisement02-refinement/restart3-north-cart/terminal-source-inputs-v1/handoff.json'),
    ]
    recipes = []
    for name, priority, terms, next_geometry, steps, donor in definitions:
        selected = [p for p in report['patch_families'] + report['targets'] if any(t in p['name'] for t in terms)]
        donor_path = REPO / donor
        if not donor_path.exists():
            raise FileNotFoundError(donor_path)
        sources = []
        for p in selected:
            instances = []
            for i in p['instances']:
                raw = i['source']
                if 'element_fx' in raw:
                    sprite = raw['element_fx']['sprite']
                    origin = [sprite['position_x'], sprite['position_y']]
                else:
                    origin = [raw['position_x'] - p['profile_metadata']['center_x'], raw['position_y'] - p['profile_metadata']['center_y']]
                bounds = []
                for row in p['rows']:
                    for f in row['frames']:
                        box = f['effective_alpha_bbox']
                        if box:
                            bounds.append([origin[0] + f['offset_x'] + box[0], origin[1] + f['offset_y'] + box[1], origin[0] + f['offset_x'] + box[2], origin[1] + f['offset_y'] + box[3]])
                instances.append({'mission': i['mission'], 'source_id': i.get('id', i.get('target_index')), 'art_origin': origin,
                                  'effective_union_bounds': [min(b[0] for b in bounds), min(b[1] for b in bounds), max(b[2] for b in bounds), max(b[3] for b in bounds)] if bounds else None})
            sources.append({'profile': p['name'], 'art_signature': p['art_signature'], 'review_sheet': p['review_sheet'], 'full_actions': p['full_actions'], 'instances': instances})
        recipes.append({'family': name, 'priority': priority, 'next_geometry': next_geometry, 'construction_steps': steps,
                        'sources': sources, 'candidate_donor': {'path': donor, 'sha256': pin(donor_path), 'scope': 'Construction workflow only. No geometry, texture, source ownership or approval transfers to C3.'},
                        'required_before_integration': ['Own source geometry review and approval', 'Per-site contact/ownership checks', 'Full script-index/timing binding', 'Texture review', 'Actual loader/editor proof']})
    (OUT / 'reconstruction-recipes.json').write_text(json.dumps({'status': 'CONCRETE_SOURCE_RECIPES_NOT_GEOMETRY', 'families': recipes,
        'nonphysical_targets': 'Bow Target is a UI/action marker; three MerryManStaff profiles include character/rappel animation and remain separate actor-native work, not static map props.',
        'source_pins': PINS}, indent=2) + '\n')


if __name__ == '__main__':
    main()
