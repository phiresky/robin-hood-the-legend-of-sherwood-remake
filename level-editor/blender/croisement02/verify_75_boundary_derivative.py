"""Reopen and compare the unchanged original leaf geometry and packed materials."""
import hashlib
import json
import sys
from pathlib import Path
import bpy
import numpy as np
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
sys.path.insert(0,str(HERE.parents[1]/'refinement'))
sys.path.insert(0,str(HERE.parents[1]/'refinement/blender'))
from central_support_geometry import leaf_signature
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release


def snapshot(model,counts=None):
    bpy.ops.wm.open_mainfile(filepath=str(model))
    objects=[o for o in bpy.data.objects if o.type=='MESH' and o.get('source_node')=='foliage-shrub-075']
    if len(objects)!=1:raise ValueError('Ambiguous leaf object')
    obj=objects[0];mesh=obj.data
    if counts is None:counts=(len(mesh.vertices),len(mesh.polygons),len(mesh.loops),len(mesh.materials))
    nv,nf,nl,nm=counts;materials=[]
    for mat in list(mesh.materials)[:nm]:
        images=[]
        for node in mat.node_tree.nodes:
            if node.type=='TEX_IMAGE' and node.image:
                pixels=np.empty(len(node.image.pixels),dtype=np.float32);node.image.pixels.foreach_get(pixels)
                images.append(dict(size=list(node.image.size),rgba_sha256=hashlib.sha256(pixels.tobytes()).hexdigest(),interpolation=node.interpolation))
        materials.append(dict(images=images,culling=mat.use_backface_culling,properties=dict(mat.items())))
    return dict(counts=counts,prefix=leaf_signature(mesh,nv,nf,nl),smooth=[p.use_smooth for p in mesh.polygons[:nf]],materials=materials,transform=[list(row) for row in obj.matrix_world])


def main():
    old=OUT/'understory-candidates/native-75-split-v19/assets/croisement02-shrub-75/model.blend'
    batch=OUT/'understory-candidates/native-75-boundary-add-v23';worker=batch/'assets/croisement02-shrub-75'
    before=snapshot(old);after=snapshot(worker/'model.blend',before['counts'])
    if before!=after:raise ValueError('Reopened original leaf geometry/material preservation failed')
    result=dict(status='PASS',model_sha256=sha(worker/'model.blend'),prior_model_sha256=sha(old),original_counts=before['counts'],prefix_signature=before['prefix'],old_geometry_uv_ownership_materials_and_transform_unchanged=True)
    write_json(worker/'inspection/boundary-preservation-reopen.json',result);print(json.dumps(result))


if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
