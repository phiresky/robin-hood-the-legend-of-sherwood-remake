"""Distinguish opaque Workbench diagnostics from physical leaf opacity."""
import json
import math
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT / 'level-editor/refinement'), str(ROOT / 'level-editor/refinement/blender')]
from evidence_io import sha, write_json
from render_slots import acquire, release
from render_views import render_views

B = ROOT / 'level-editor/work/croisement03-refinement/restart2'


def main():
    candidate = B / 'tree11-crown-prototype-v2'
    out = candidate / 'physical-opacity-audit'
    assert not out.exists()
    out.mkdir()
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(candidate / 'worker.blend'))
        scene = bpy.data.scenes['Tree13 isolated wood']
        leaf = next(o for o in scene.objects if o.get('asset_group') == 'croisement03-arbre06-fragment-tree11-provisional')
        mesh = leaf.data
        count = json.loads((candidate / 'receipt.json').read_text())['native_faces']
        source_faces = list(mesh.polygons)[:count]
        assert count == 911 and all(len(f.vertices) == 4 for f in source_faces)
        depths, widths, heights = [], [], []
        sin, cos = math.sin(math.radians(35)), math.cos(math.radians(35))
        for face in source_faces:
            vs = [mesh.vertices[i].co for i in face.vertices]
            depths.append(float(sum(v.y for v in vs) / 4))
            widths.append(float(max(v.x for v in vs) - min(v.x for v in vs)))
            projected = [-v.y * sin - v.z * cos for v in vs]
            heights.append(float(max(projected) - min(projected)))
        assert max(widths) <= 3.001 and max(heights) <= 3.001
        assert len({tuple(f.vertices) for f in source_faces}) == count
        assert len({i for f in source_faces for i in f.vertices}) == count * 4
        material = mesh.materials[0]
        tex = next(n for n in material.node_tree.nodes if n.type == 'TEX_IMAGE')
        mix = next(n for n in material.node_tree.nodes if n.type == 'MIX_SHADER')
        assert mix.inputs[0].links[0].from_node == tex
        assert mix.inputs[0].links[0].from_socket.name == 'Alpha'
        assert mix.inputs[1].links[0].from_node.type == 'BSDF_TRANSPARENT'
        emission = mix.inputs[2].links[0].from_node
        assert emission.type == 'EMISSION'
        material.node_tree.links.remove(emission.inputs['Color'].links[0])
        emission.inputs['Color'].default_value = (.55, .55, .55, 1)
        # Only RGB changes in this diagnostic. Original physical alpha still drives transparency.
        render_views(scene.name, {'view-0': 'Tree13 view0', 'view-1': 'Tree13 view1', 'view-4': 'Tree13 view4'}, out, modes=('textured',), width=768)
        source = np.array(Image.open(B / 'tree11-canopy-fragment-source-v1/000.png'))
        write_json(out / 'receipt.json', dict(status='PASS disconnected source cells, no continuous opaque source panel',
            model_sha256=sha(candidate / 'worker.blend'), source_faces=count,
            native_face_max_projected_width=max(widths), native_face_max_projected_height=max(heights),
            native_cell_depth_min=min(depths), native_cell_depth_max=max(depths),
            independently_vertexed_quads=True, actual_total_leaf_faces=len(mesh.polygons),
            actual_inferred_leaf_faces=len(mesh.polygons)-count,
            frame0_opaque_pixels=int(np.count_nonzero(source[:, :, 3])), frame0_total_pixels=int(source.shape[0]*source.shape[1]),
            original_alpha_controls_transparent_shader=True,
            diagnostic='Shared solid mode uses BLENDER_WORKBENCH, which renders every card opaque. New gray RGB Cycles renders preserve the original alpha, geometry and UV without saving a modified worker.',
            receipt_correction='The original construction receipt inferred_leaf_faces counter was overwritten by a temporary support-branch face list. The mesh counts above are authoritative; no geometry change.',
            views={str(i): sha(out / f'view-{i}-textured.png') for i in (0, 1, 4)}))
    finally:
        release()


if __name__ == '__main__':
    main()
