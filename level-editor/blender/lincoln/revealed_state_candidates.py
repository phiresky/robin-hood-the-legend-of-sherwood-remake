"""Write state-review candidates into cloned Lincoln workspaces (system Python).

python3 level-editor/blender/lincoln/revealed_state_candidates.py [--overrides <workspace map>] <spec.json> [...]

Review text (summary, state names, limitations) lives beside each spec in
<asset>.review.json, so wording edits do not invalidate rendered packets.

Each spec's asset has a clone of its approved workspace in
state-review/assets/<asset> (model, input and modified packets byte-identical,
so the recorded geometry approval still binds the covered model). Rendered state
packets live in states/<state>/{revealed,input} (and states/covered/covered when
the covered display itself changes). This adds the collector's state fields to
the clone's candidate.json and a "Revealed and animated states" review.md
section. The collector then binds the packets into state_bundle_sha256, so the
existing geometry approval no longer hides the card until the user approves the
state bundle. It never touches the approved workspace.
"""
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / 'work/needs-review/lincoln-refinement'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    args = sys.argv[1:]
    overrides_path = WORK / 'workspace-overrides-v6.json'
    if args[:1] == ['--overrides']:
        overrides_path, args = Path(args[1]), args[2:]
    overrides = json.loads(overrides_path.read_text())['assets']
    for spec_path in map(Path, args):
        spec = json.loads(spec_path.read_text())
        asset = spec['asset_id']
        review = json.loads(spec_path.with_suffix('.review.json').read_text())
        approved = Path(overrides[asset])
        clone = WORK / 'state-review/assets' / asset
        for name in ('model.blend', 'modified/views.json', 'input/views.json', 'workspace.json'):
            if sha(approved / name) != sha(clone / name):
                raise ValueError(f'{asset}: clone differs from approved workspace: {name}')
        candidate = json.loads((approved / 'candidate.json').read_text())
        geometry = json.loads((WORK / 'state-review/models' / asset / 'state-geometry.json').read_text())
        masks = json.loads((WORK / 'state-review/masks' / asset / 'source-masks.json').read_text())
        main_state = review['main_state']
        for state in spec['states']:
            for mode in (('revealed', 'input') if state == main_state else ('revealed',)):
                binding = json.loads((clone / 'states' / state / mode / 'state-binding.json').read_text())
                if binding['state_model_sha256'] != geometry['state_model_sha256']:
                    raise ValueError(f'{asset}/{state}/{mode}: packet was rendered from another state model')
                if binding['spec_sha256'] != sha(spec_path):
                    raise ValueError(f'{asset}/{state}/{mode}: packet was rendered from another spec')
        base = f'states/{main_state}/revealed/'
        candidate.update(
            revealed_solid=base + 'solid.png', revealed_textured=base + 'textured.png',
            revealed_context=base + 'context.png', revealed_input=f'states/{main_state}/input')
        others = [s for s in spec['states'] if s != main_state]
        if others:
            candidate['animation_states'] = [
                {'id': state, 'directory': f'states/{state}/revealed',
                 'name': review['states'][state]['name'], 'description': review['states'][state]['description']}
                for state in others]
        if review.get('covered_changed'):
            folder = 'states/covered/covered/'
            binding = json.loads((clone / folder / 'state-binding.json').read_text())
            if binding['state_model_sha256'] != geometry['state_model_sha256']:
                raise ValueError(f'{asset}: covered packet was rendered from another state model')
            candidate.update(covered_solid=folder + 'solid.png', covered_textured=folder + 'textured.png',
                             covered_context=folder + 'context.png')
        candidate['status'] = 'ready-for-user'
        candidate['limitations'] = list(candidate.get('limitations', [])) + review.get('limitations', [])
        candidate['state_review'] = {
            'version': 1, 'summary': review['summary'],
            'states': {state: {'applied_patches': patches, **review['states'][state]}
                       for state, patches in spec['states'].items()},
            'main_state': main_state,
            'spec': str(spec_path.resolve()), 'spec_sha256': sha(spec_path),
            'state_model': str(WORK / 'state-review/models' / asset / 'state-model.blend'),
            'state_model_sha256': geometry['state_model_sha256'],
            'approved_model_sha256': geometry['source_blend_sha256'],
            'approved_geometry_unchanged': geometry['original_objects_unchanged'],
            'state_geometry': str(WORK / 'state-review/models' / asset / 'state-geometry.json'),
            'state_masks': str(WORK / 'state-review/masks' / asset / 'source-masks.json'),
            'state_mask_review': masks['review'],
            'editor_visibility': geometry['state_properties'],
            'variants': [{k: v[k] for k in ('object', 'variant_of', 'component', 'patches', 'hide_patches', 'note')}
                         for v in geometry['variants']],
            'additions': [{k: v[k] for k in ('object', 'source_node', 'component', 'patches', 'hide_patches', 'note')}
                          for v in geometry['additions']],
            'decision_scope': ('Approving this card approves the state bundle (revealed/animated packets, state '
                               'geometry and masks). The covered geometry approval is unchanged.'),
        }
        (clone / 'candidate.json').unlink()
        (clone / 'candidate.json').write_text(json.dumps(candidate, indent=2) + '\n')
        text = (approved / 'review.md').read_text().rstrip() + '\n\n## Revealed and animated states\n\n'
        text += review['summary'] + '\n\n'
        for state, patches in spec['states'].items():
            entry = review['states'][state]
            text += f"- **{entry['name']}** (`{state}`: {', '.join(patches)}): {entry['description']}\n"
        text += '\nState geometry (approved covered objects untouched; whole-object visibility per patch):\n\n'
        for row in geometry['variants']:
            text += (f"- `{row['object']}`: shown when {', '.join(row['patches'])} applied; "
                     f"replaces `{row['variant_of']}`. {row['note']}\n")
        for row in geometry['additions']:
            text += f"- `{row['object']}` (new, {row['source_node']}): shown when {', '.join(row['patches'])} applied. {row['note']}\n"
        for node, names in geometry['hide'].items():
            text += f"- `{node}` hidden when its patch is applied ({len(names)} object(s)).\n"
        for node, name in geometry['show'].items():
            text += f"- `{name}` shown only when its patch is applied.\n"
        if review.get('limitations'):
            text += '\nState limitations:\n\n' + ''.join(f'- {row}\n' for row in review['limitations'])
        (clone / 'review.md').unlink()
        (clone / 'review.md').write_text(text)
        print('CANDIDATE', asset, main_state, others)


if __name__ == '__main__':
    main()
