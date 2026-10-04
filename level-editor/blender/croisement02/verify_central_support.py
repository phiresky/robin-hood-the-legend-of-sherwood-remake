"""Reopen a support derivative and prove its preexisting leaf loops unchanged."""
import argparse
import sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from central_support_geometry import leaf_signature
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release


def verify(version):
    old=OUT/'understory-candidates/native-71-scoped-v1/assets/croisement02-shrub-71/model.blend'
    new=OUT/f'understory-candidates/native-71-{version}/assets/croisement02-shrub-71/model.blend'
    hashes=[sha(old),sha(new)]
    def target():
        matches=[o for o in bpy.data.objects if o.type=='MESH' and o.get('source_node')=='foliage-shrub-071']
        if len(matches)!=1:raise ValueError('Expected one native71 target')
        return matches[0]
    bpy.ops.wm.open_mainfile(filepath=str(old));obj=target();counts=(len(obj.data.vertices),len(obj.data.polygons),len(obj.data.loops))
    before=leaf_signature(obj.data,*counts);matrix=[list(row) for row in obj.matrix_world]
    bpy.ops.wm.open_mainfile(filepath=str(new));obj=target();after=leaf_signature(obj.data,*counts)
    if before!=after or matrix!=[list(row) for row in obj.matrix_world]:raise ValueError('Existing leaf mesh or placement changed')
    if hashes!=[sha(old),sha(new)]:raise ValueError('Input changed during reopen audit')
    report=dict(status='PASS',old_model=str(old),old_model_sha256=hashes[0],model_sha256=hashes[1],
                preserved_leaf_vertices=counts[0],preserved_leaf_faces=counts[1],preserved_leaf_loops=counts[2],
                exact_leaf_signature=before,additional_vertices=len(obj.data.vertices)-counts[0],
                additional_faces=len(obj.data.polygons)-counts[1],world_transform_unchanged=True,
                limitation='Support is inferred; signature proves old leaf geometry/UV/ownership/material indices unchanged, not observed roots')
    write_json(new.parent/'inspection/support-preservation.json',report);print(report)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--version',required=True)
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);acquire()
    try:verify(args.version)
    finally:release()
