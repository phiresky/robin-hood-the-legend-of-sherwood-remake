"""Enumerate native canopy coverage of both net body alternatives before attachment inference."""
import json
from pathlib import Path
import numpy as np
from PIL import Image
from catalog import OUT
from native_sign_foreground_reference import behind, placed_frame
from native_log_foreground_reference import sha


def main():
    source = OUT / 'source-states'
    layers_path = source / 'layers.json'
    layers = json.loads(layers_path.read_text())
    animation_path = OUT / 'animation-references/manifest.json'
    animations = json.loads(animation_path.read_text())['animations']
    profile_path = Path(animations[0]['frames'][0]['source']).parents[3] / 'Trapcr02.rhs.d/manifest.json'
    profiles = json.loads(profile_path.read_text())['profiles']
    targets = json.loads((OUT / 'state-target-evidence/manifest.json').read_text())['instances']
    patches = [p for p in layers['mission_patches'] if p['mission'] == 'Emb05_FoB_MP' and p['name'].endswith(('piege01e', 'piege01i', 'piege03e', 'piege03i'))]
    assert len(patches) == 4
    ordered = [dict(kind='animation', index=a['index'], poly=a['display_polyline']) for a in animations if a['display_polyline']]
    ordered += [dict(kind='target', index=t['target_index'], poly=t['target']['polyline']) for t in targets if t['mission'] == 'Emb05_FoB_MP' and t['target']['polyline']]
    ordered += [dict(kind='patch', index=p['id'], poly=p['state']['element_fx']['display_polyline']) for p in layers['patches'] + layers['mission_patches'] if (p.get('mission') in (None, 'Emb05_FoB_MP')) and p['state']['element_fx']['display_polyline']]
    ordered.sort(key=lambda a: min(p[1] for p in a['poly']))
    records = []
    for patch in patches:
        fx = patch['state']['element_fx']
        assert fx['sprite']['elevation'] == 1 and not fx['display_polyline']
        sprite = fx['sprite']
        profile = next(p for p in profiles if p['name'] == sprite['profile_name'])
        point = [sprite['position_x'] + profile['center_x'], sprite['position_y'] + profile['center_y']]
        insertion = next((i for i, a in enumerate(ordered) if behind(a['poly'], point)), len(ordered))
        later = {a['index'] for a in ordered[insertion:] if a['kind'] == 'animation'}
        rows = []
        for state, data in patch['states'].items():
            for index, frame in enumerate(data['frames']):
                x, y, w, h = frame['bbox']
                box = (x, y, x+w, y+h)
                path = source / frame['image']
                alpha = np.asarray(Image.open(path).convert('RGBA'))[:, :, 3] > 127
                overlays = []
                for animation in animations:
                    if animation['index'] not in later:
                        continue
                    counts = []
                    for other in animation['frames']:
                        mask = np.asarray(placed_frame(other, box))[:, :, 3] > 127
                        counts.append(int((mask & alpha).sum()))
                    if max(counts):
                        overlays.append(dict(index=animation['index'], profile=animation['profile'], counts=counts))
                rows.append(dict(state=state, frame=index, sha256=sha(path), source_pixels=int(alpha.sum()), later_overlay_coverage=overlays))
        records.append(dict(patch=patch['id'], point=point, profile_center=[profile['center_x'], profile['center_y']], insertion=insertion, frames=rows))
    dest = OUT / 'net-native-order-v1'
    dest.mkdir(exist_ok=False)
    result = dict(status='Source ordering diagnostic; no physical attachment decision', layer_sha256=sha(layers_path), animation_sha256=sha(animation_path), profile_sha256=sha(profile_path), order=ordered, records=records, semantics=['Body patches have elevation 1 and empty polylines, so merge as non-animations using sprite position plus profile center.', 'Force display controls visibility eligibility, not foreground priority.', 'The elevation-zero leaf effect uses background animation presentation separately.'], limitations=['Coverage is enumerated separately per overlay; joint overlay unions are not reported.', 'Moving actors and active target imagery are excluded.', 'This diagnostic does not establish physical crown occlusion or attachment identity.'])
    (dest / 'manifest.json').write_text(json.dumps(result, indent=2)+'\n')
    print([(r['patch'], r['point'], sorted({a['index'] for f in r['frames'] for a in f['later_overlay_coverage']})) for r in records])


if __name__ == '__main__':
    main()
