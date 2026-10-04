"""Append the reviewed 28 native leaf pixels without rebuilding existing leaves."""
import copy
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image

HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
import prepare_central_shrubs as prep
from central_support_geometry import leaf_signature
from tree_geometry import SIN,COS,RAY,material,one_sided
from opacity_bounds import measure
from evidence_io import sha,write_json

OLD=prep.OUT/'understory-candidates/native-75-split-v19'
WORKER=OLD/'assets/croisement02-shrub-75'
EXPECTED='cfc00374a4755dedd2d614807bd3ba07e65354309f27834052da315774371e4a'


def build(obj,packet):
    if sha(WORKER/'model.blend')!=EXPECTED:raise ValueError('Prior reviewed75 changed')
    audit=json.loads((WORKER/'inspection/saved-model-audit.json').read_text())
    names=[r['object'] for r in audit['objects']]
    if len(names)!=1:raise ValueError('Expected one preserved shrub')
    with bpy.data.libraries.load(str(WORKER/'model.blend'),link=False) as (_,dst):dst.objects=names
    prior=dst.objects[0]
    old=prior.data;nv,nf,nl=len(old.vertices),len(old.polygons),len(old.loops)
    before=leaf_signature(old,nv,nf,nl)
    vertices=[tuple(v.co) for v in old.vertices];faces=[tuple(p.vertices) for p in old.polygons]
    slots=[p.material_index for p in old.polygons]
    uvs=[tuple(v.uv) for v in old.uv_layers['Foliage UV'].data]
    colors=[tuple(v.color) for v in old.color_attributes['Source ownership'].data]
    mats=list(old.materials);directory=Path(packet['directory'])
    mask_path=prep.OUT/'missing-fence-candidates/boundary-roles95-v1/foliage75.png'
    extra=np.asarray(Image.open(mask_path).convert('L'))>0
    if int(extra.sum())!=28:raise ValueError('Wrong additive boundary')
    x0,y0,w,h=packet['native_bbox'];source=np.asarray(Image.open(directory/'observed-source.png').convert('RGBA')).copy()
    source[:,:,3]=np.where(extra[y0:y0+h,x0:x0+w],source[:,:,3],0)
    Image.fromarray(source).save(directory/'added-native-leaves.png')
    start=len(mats)
    mats.extend([material(obj.name+' added observed boundary',directory/'added-native-leaves.png',True),material(obj.name+' inferred boundary backs',directory/'added-native-leaves.png',False)])
    for mat in mats[-2:]:one_sided(mat)
    known_vertices=sorted({v for p in old.polygons if old.materials[p.material_index].get('foliage_observed') for v in p.vertices})
    points=np.array([vertices[i] for i in known_vertices]);screen=np.column_stack((points[:,0],-points[:,1]*SIN-points[:,2]*COS))
    right=np.array([1.,0,0]);down=np.array([0.,-SIN,-COS]);ray=np.asarray(RAY)
    for sy,sx in zip(*np.nonzero(extra)):
        target=np.array([sx+.5,sy+.5]);near=int(np.argmin(np.sum((screen-target)**2,axis=1)))
        center=points[near]+right*(target[0]-screen[near,0])+down*(target[1]-screen[near,1])
        corners=[center+right*dx+down*dy for dx,dy in [(-.5,-.5),(.5,-.5),(.5,.5),(-.5,.5)]]
        if np.dot(np.cross(corners[1]-corners[0],corners[2]-corners[0]),ray)<0:corners.reverse()
        for back in (False,True):
            pts=list(reversed(corners)) if back else corners
            offset=len(vertices);vertices.extend(tuple(p) for p in pts);faces.append(tuple(range(offset,offset+4)));slots.append(start+back)
            uvs.extend(((p[0]-x0)/w,1-(-p[1]*SIN-p[2]*COS-y0)/h) for p in pts)
            colors.extend([(0. if back else 1.,1.,1.,1.)]*4)
    mesh=bpy.data.meshes.new(obj.name+' exact boundary addition');mesh.from_pydata(vertices,[],faces);mesh.update()
    for mat in mats:mesh.materials.append(mat)
    layer=mesh.uv_layers.new(name='Foliage UV');ownership=mesh.color_attributes.new(name='Source ownership',type='FLOAT_COLOR',domain='CORNER');mesh.color_attributes.active_color=ownership
    for p,slot in zip(mesh.polygons,slots):p.material_index=slot
    for i,(uv,color) in enumerate(zip(uvs,colors)):layer.data[i].uv=uv;ownership.data[i].color=color
    if leaf_signature(mesh,nv,nf,nl)!=before:raise ValueError('Existing leaf prefix changed')
    obj.data=mesh;obj.matrix_world=prior.matrix_world.copy()
    result=copy.deepcopy(json.loads((WORKER/'inspection/refinement.json').read_text())['crown'])
    support=json.loads((OLD/'shrub-75/support.json').read_text());write_json(directory/'support.json',support)
    proof=dict(prior_worker=str(WORKER),prior_model_sha256=EXPECTED,original_vertices=nv,original_faces=nf,original_loops=nl,original_prefix_signature=before,added_pixels=28,added_faces=56,added_mask_sha256=sha(mask_path),prior_materials_unchanged=True,depth_rule='Closest existing observed leaf vertex depth; exact source projection retained',user_approval=None)
    write_json(directory/'boundary-preservation.json',proof)
    result.update(opacity_bounds=measure(obj),boundary_addition=proof)
    bpy.data.objects.remove(prior,do_unlink=True)
    if sha(WORKER/'model.blend')!=EXPECTED:raise ValueError('Prior source modified')
    return result


if __name__=='__main__':
    prep.CHOSEN={75:[]};prep.FIRST_DOMAIN=487
    prep.DIRECTORY=prep.OUT/'understory-candidates/native-75-boundary-add-v22'
    prep.MIXED_SOURCE_REVIEW=prep.OUT/'understory-candidates/mixed75-91-source-v3/source-review.json'
    prep.build=build
    prep.acquire()
    try:prep.main()
    finally:prep.release()
