"""Measure source-camera physical crown coverage across the clipped map edge."""
import json
from pathlib import Path
import sys

import bpy
import numpy as np
from PIL import Image
from mathutils import Vector

ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_review import _tree
from tree_geometry import SIN,RAY


def main():
    experiment=OUT/'texture-fill-round-1/croisement02-tree-40/complete-native-front-preparation/experiment'
    output=OUT/'texture-fill-round-1/croisement02-tree-40/boundary-density-audit-v1'
    if output.exists():raise FileExistsError(output)
    before=sha(experiment/'approved-model.blend')
    manifest=json.loads((experiment/'views.json').read_text())
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(experiment/'approved-model.blend'))
        scene=bpy.data.scenes[manifest['scene_name']]
        bpy.context.window.scene=scene
        objects=[scene.objects[name] for name in manifest['object_names'] if any(
            material and material.get('foliage_physical_opacity') for material in scene.objects[name].data.materials)]
        tree,_,_=_tree(objects)
        pixels=np.zeros((152,226),bool)
        ray=Vector(RAY)
        for yy in range(152):
            for xx in range(226):
                x,y=1679+xx+.5,319+yy+.5
                point=Vector((x,-y/SIN,0))+ray*10000
                hit,_,_,_=tree.ray_cast(point,-ray)
                pixels[yy,xx]=hit is not None
        left=pixels[:,:113][:,::-1]
        right=pixels[:,113:]
        rows=[]
        for start,end in [(0,1),(1,4),(4,16),(16,32),(32,60),(60,113)]:
            a,b=left[:,start:end],right[:,start:end]
            rows.append(dict(edge_distance=[start,end],left_coverage=float(a.mean()),
                right_coverage=float(b.mean()),right_fills_mirrored_left_gap=int((b&~a).sum()),
                right_missing_mirrored_left_leaf=int((a&~b).sum())))
        output.mkdir()
        Image.fromarray(pixels.astype('uint8')*255).save(output/'actual-physical-coverage.png')
        write_json(output/'report.json',dict(status='read-only physical density evidence',
            model_sha256=before,source_box=[1679,319,1905,471],bands=rows,
            interpretation='Mirrored comparison is diagnostic only, not inferred-alpha authority. Unequal coverage can reflect real silhouette differences; inspect shape and native source together.',
            geometry_saved=False,physical_alpha_changed=False))
        if sha(experiment/'approved-model.blend')!=before:raise ValueError('Frozen model changed')
    finally:release()


if __name__=='__main__':main()
