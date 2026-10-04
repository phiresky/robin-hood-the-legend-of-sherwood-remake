"""Reopen a saved ground receiver and check geometry and observed atlas ownership."""
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from prepare_ground_receiver import ASSET,COLLECTION
from evidence_io import digest,sha,write_json
from refinement_workspace import validate


def signature(obj):
    return digest({'vertices':[list(v.co) for v in obj.data.vertices], 'faces':[list(p.vertices) for p in obj.data.polygons], 'matrix':[list(r) for r in obj.matrix_world]})


def main():
    worker=Path(sys.argv[sys.argv.index('--')+1]).resolve()
    config=json.loads((worker/'workspace.json').read_text())
    bpy.ops.wm.open_mainfile(filepath=str(worker/'baseline.blend'))
    original,=[o for o in bpy.data.collections[COLLECTION].all_objects if o.type=='MESH' and o.get('asset_group')==ASSET]
    baseline=signature(original)
    bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
    validation=validate(worker)
    obj,=[o for o in bpy.data.collections[COLLECTION].all_objects if o.type=='MESH' and o.get('asset_group')==ASSET]
    if signature(obj)!=baseline:raise ValueError('Native ground geometry changed')
    if any(abs((obj.matrix_world@v.co).z)>.001 for v in obj.data.vertices):raise ValueError('Ground is not the native zero plane')
    images=[node.image for mat in obj.data.materials for node in mat.node_tree.nodes if node.type=='TEX_IMAGE' and node.image]
    if len(images)!=1 or not images[0].packed_file:raise ValueError('Ground source image must be packed')
    atlas=worker/'reference/observed-neutral.png'
    import hashlib
    if hashlib.sha256(images[0].packed_file.data).hexdigest()!=sha(atlas):raise ValueError('Stored atlas differs from ownership proposal')
    source=np.asarray(Image.open(worker/'reference/source.png').convert('RGB'));known=np.asarray(Image.open(worker/'reference/ground-observed-domain.png'))>0
    observed=np.asarray(Image.open(atlas).convert('RGB'))
    if np.any(source[known]!=observed[known]):raise ValueError('Known pixels changed')
    if np.any(observed[~known]!=127):raise ValueError('Unknown pixels are not neutral')
    write_json(worker/'inspection/saved-model-audit.json',dict(status='PASS',model_sha256=sha(worker/'model.blend'),baseline_geometry_signature=baseline,geometry_unchanged=True,world_z_range=[min((obj.matrix_world@v.co).z for v in obj.data.vertices),max((obj.matrix_world@v.co).z for v in obj.data.vertices)],vertices=len(obj.data.vertices),faces=len(obj.data.polygons),source_node=obj['source_node'],known_pixels=int(known.sum()),known_rgb_differences=0,atlas_sha256=sha(atlas),outside_ownership_validation=validation,approval='pending',limitations=['Catalog78 is an ownership snapshot. The frozen geometry context contains native proxies plus reviewed bank candidate, not a complete78-group assembly.','Full-scene final first-hit and state application checks remain integration work.']))


if __name__=='__main__':main()
