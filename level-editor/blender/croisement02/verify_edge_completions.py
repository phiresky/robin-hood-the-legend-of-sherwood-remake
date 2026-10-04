"""Verify added crown geometry preserves the entire previous asset appearance."""
import hashlib
import json
import sys
from pathlib import Path

import bpy

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent))
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from complete_northern_caps import mesh_prefix
from evidence_io import sha, write_json
from render_slots import acquire, release


def images(material):
    return sorted(hashlib.sha256(node.image.packed_file.data).hexdigest()
        for node in material.node_tree.nodes if node.type=='TEX_IMAGE' and node.image)


def objects(asset):
    return [o for o in bpy.data.collections['Croisement02 Working'].all_objects
            if o.type=='MESH' and o.get('asset_group')==asset]


def main():
    receipts=sorted((OUT/'forest-v4-round-3/assets').glob('*/inspection/northern-cap-revision.json'))
    rows=[]
    acquire()
    try:
        for path in receipts:
            receipt=json.loads(path.read_text())
            worker=path.parents[1]
            old=Path(receipt['previous_worker'])
            if sha(old/'model.blend')!=receipt['previous_model_sha256']:
                raise ValueError('Previous worker changed: '+str(old))
            if sha(worker/'model.blend')!=receipt['model_sha256']:
                raise ValueError('Candidate changed: '+str(worker))
            bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'))
            before={o.name:dict(counts=(len(o.data.vertices),len(o.data.polygons),len(o.data.loops)),
                mesh=mesh_prefix(o.data),transform=[list(r) for r in o.matrix_world],
                materials=[images(m) for m in o.data.materials]) for o in objects(worker.name)}
            bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
            after=objects(worker.name)
            if set(before)!={o.name for o in after}:raise ValueError('Asset components changed')
            for obj in after:
                prior=before[obj.name]
                if mesh_prefix(obj.data,*prior['counts'])!=prior['mesh']:
                    raise ValueError('Existing geometry or UV changed: '+obj.name)
                if [list(r) for r in obj.matrix_world]!=prior['transform']:
                    raise ValueError('Transform changed: '+obj.name)
                if [images(m) for m in list(obj.data.materials)[:len(prior['materials'])]]!=prior['materials']:
                    raise ValueError('Existing packed appearance changed: '+obj.name)
                if obj.get('projection_component')!='crown' and prior['counts']!=(len(obj.data.vertices),len(obj.data.polygons),len(obj.data.loops)):
                    raise ValueError('Wood changed: '+obj.name)
            rows.append(dict(asset_id=worker.name,model_sha256=receipt['model_sha256'],
                previous_model_sha256=receipt['previous_model_sha256'],status='PASS',
                preserved=['existing vertices and faces','UVs','source ownership','transforms','packed source and inferred appearance'],
                limitation='Preservation check only; new geometry still needs visual and integrated review.'))
        write_json(OUT/'map-edge-review/preservation.json',dict(status='PASS',assets=rows))
        print('Preserved previous geometry and appearance:',len(rows))
    finally:
        release()


if __name__=='__main__':main()
