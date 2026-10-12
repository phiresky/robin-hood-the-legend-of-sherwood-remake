"""Render a fixed-camera state packet for one Lincoln asset from its state model.

blender --background <state-model.blend> --threads 2 --python-exit-code 1 \
  --python level-editor/blender/lincoln/revealed_state_packets.py -- \
  <workspace> <spec.json> <state-id> <revealed|input|covered> <output-dir>

The packet reuses the workspace's frozen eight cameras, crop, tile size and
lighting (modified/views.json), so build_gallery.supplemental_packet accepts it.
Visibility follows the editor's patch display for the state's applied patches:
an object with reveal_hide_when_applied is hidden when any listed patch is
applied; an object with reveal_show_when_applied is shown only when one is.

- revealed: state geometry (variants/additions) with the state source artwork
  and the reviewed state-<id> masks from state-review/masks/<asset>/.
- input: the approved geometry with only the spec's whole-object covers hidden;
  replaced originals stay unmodified and no additions are shown. Same source and
  masks: the before-state showing where the revealed art lands without the
  reviewed state geometry.
- covered: the covered state (no patch applied) with covered.png and the
  workspace's covered masks; used where the covered display itself changes.

A state-binding.json inside the packet binds the state model, spec, masks and
source hashes, so the collector's state bundle hash covers them.
"""
import hashlib
import json
from pathlib import Path
import sys

import bpy

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / 'work/needs-review/lincoln-refinement'
sys.path.insert(0, str(Path(__file__).resolve().parent))
import render_slots  # noqa: E402


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def visible(obj, applied, mode, spec=None):
    hide = set(obj.get('reveal_hide_when_applied', []))
    show = obj.get('reveal_show_when_applied')
    if mode == 'input':
        # Approved geometry only: whole-object covers from the spec's hide list go,
        # replaced originals stay unmodified (their art lands on the covered shape).
        if obj.get('state_recipe'):
            return False
        hide = {p for node, patches in spec.get('hide', {}).items()
                if node == obj.get('source_node') for p in patches}
    if hide & applied:
        return False
    if show is not None:
        return bool(set(show) & applied)
    return True


def main():
    workspace, spec_path, state, mode, output = sys.argv[sys.argv.index('--') + 1:]
    workspace, spec_path, output = Path(workspace).resolve(), Path(spec_path).resolve(), Path(output).resolve()
    if mode not in ('revealed', 'input', 'covered'):
        raise ValueError('Mode must be revealed, input or covered')
    tooling = json.loads((WORK / 'tooling/current.json').read_text())['directory']
    sys.path.insert(0, tooling)
    from refinement_review import render_review
    render_slots.acquire()
    config = json.loads((workspace / 'workspace.json').read_text())
    spec = json.loads(spec_path.read_text())
    asset = config['asset_id']
    if spec['asset_id'] != asset:
        raise ValueError('Spec and workspace asset differ')
    geometry = json.loads((Path(bpy.data.filepath).parent / 'state-geometry.json').read_text())
    if geometry['state_model_sha256'] != sha(bpy.data.filepath) or geometry['spec_sha256'] != sha(spec_path):
        raise ValueError('State model is not the recorded build of this spec')
    applied = set() if mode == 'covered' else set(spec['states'][state])
    collection = bpy.data.collections[config['collection_name']]
    context_hide = {node for node, patches in spec.get('context_hide', {}).items() if set(patches) & applied}
    shown = []
    # Snapshot first: changing visibility invalidates the all_objects cache mid-iteration.
    for obj in list(collection.all_objects):
        if obj.type != 'MESH':
            continue
        if obj.get('asset_group') == asset:
            obj.hide_render = not visible(obj, applied, mode, spec)
            if not obj.hide_render:
                shown.append(obj.name)
        elif obj.get('state_context_recipe'):
            obj.hide_render = mode == 'covered' or not visible(obj, applied, 'revealed')
        elif obj.get('source_node') in context_hide or (
                set(obj.get('state_context_hide_when_applied', [])) & applied):
            obj.hide_render = True
    bpy.context.view_layer.update()
    baseline = json.loads((workspace / 'modified/views.json').read_text())
    if mode == 'covered':
        source = WORK / 'source-states/covered.png'
        masks = Path(config['source_mask_manifest']).resolve()
        label = 'exterior'
    else:
        record = json.loads((WORK / 'state-review/sources/manifest.json').read_text())['states'][state]
        source = Path(record['image'])
        if sha(source) != record['sha256']:
            raise ValueError('State source artwork changed')
        masks = (WORK / 'state-review/masks' / asset / 'source-masks.json').resolve()
        label = 'state-' + state
    frame = {**baseline, 'source_sha256': sha(source)}
    all_objects = [o for o in collection.all_objects if o is not None and o.type == 'MESH' and not o.hide_render]
    present = {o.get('source_node') for o in all_objects}
    receivers = sorted({o.get('source_node') for o in all_objects
                        if o.get('asset_group') == asset})
    layers = [{'source_path': str(source), 'projection_label': label,
               'receiver_nodes': receivers, 'occluder_nodes': sorted(present, key=str)}]
    manifest = render_review(output, scene_name=config['scene_name'], collection_name=config['collection_name'],
                             asset_id=asset, source_path=str(source), frame_manifest=frame,
                             lighting=config.get('lighting'), projection_layers=layers,
                             source_mask_manifest=str(masks), render_object_names=shown,
                             allow_projection_revision=True, allow_mask_revision=True)
    binding = {'version': 1, 'asset_id': asset, 'state': state, 'mode': mode,
               'applied_patches': sorted(applied), 'context_hidden_nodes': sorted(context_hide),
               'state_model': bpy.data.filepath, 'state_model_sha256': sha(bpy.data.filepath),
               'approved_model_sha256': geometry['source_blend_sha256'],
               'spec': str(spec_path), 'spec_sha256': sha(spec_path),
               'source': str(source), 'source_sha256': sha(source),
               'masks': str(masks), 'masks_sha256': sha(masks), 'projection_label': label,
               'frozen_views': str(workspace / 'modified/views.json'),
               'frozen_views_sha256': sha(workspace / 'modified/views.json'),
               'visible_objects': sorted(shown),
               'counts': [v['counts'] for v in manifest['views']]}
    (output / 'state-binding.json').write_text(json.dumps(binding, indent=2) + '\n')
    print('STATE-PACKET', asset, state, mode, json.dumps(binding['counts']))


if __name__ == '__main__':
    main()
