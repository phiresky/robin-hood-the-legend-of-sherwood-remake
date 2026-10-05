"""Read frozen cart surfaces at surveyed native panel coordinates."""
import json
import sys
from pathlib import Path
import bpy
from mathutils.bvhtree import BVHTree

HERE = Path(__file__).resolve().parent
sys.path[:0] = [str(HERE), str(HERE.parents[1] / 'refinement'),
                str(HERE.parents[1] / 'refinement/blender')]
from catalog import OUT
from tree_geometry import RAY
from log_trap_state_candidate import point
from evidence_io import sha, write_json
from render_slots import acquire, release


def main():
    source = OUT / 'restart2-state/south-cart-wreck-solid-v3/worker.blend'
    assert sha(source) == '192fbfa9e2f9806aae828f50ac25247e9fa0f5150384e9d14fff2868fb84362c'
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(source))
        meshes = [o for o in bpy.context.scene.objects if o.type == 'MESH']
        trees = {o.name: BVHTree.FromPolygons([o.matrix_world @ v.co for v in o.data.vertices],
                  [p.vertices[:] for p in o.data.polygons]) for o in meshes}
        coords = [(94,31),(103,37),(96,45),(85,39),(95,28),(106,36),(97,49),(81,40),
                  (72,28),(96,14),(129,19),(146,42),(138,72),(107,112),(65,98),
                  (54,64),(56,0),(78,2),(171,35),(188,44),(187,62),(170,91),(157,114)]
        rows = []
        for x,y in coords:
            start = point(953+x,844+y,0) + RAY*3000
            hits = []
            for name,tree in trees.items():
                hit,normal,index,distance = tree.ray_cast(start,-RAY,6000)
                if hit is not None: hits.append(dict(object=name,world=list(hit),ray_depth=hit.dot(RAY),distance=distance))
            rows.append(dict(pixel=[x,y],hits=sorted(hits,key=lambda r:r['distance'])))
        write_json(OUT/'restart3-south-cart/surface-survey-v1.json',dict(source=str(source),model_sha256=sha(source),rows=rows))
    finally:
        release()


if __name__ == '__main__':
    main()
