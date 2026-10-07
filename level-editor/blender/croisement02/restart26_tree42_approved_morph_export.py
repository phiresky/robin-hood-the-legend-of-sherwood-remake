"""Queued private Tree42 morph delivery; exact static GLB appearance is retained."""
import hashlib,json,math,struct,sys
from pathlib import Path
import numpy as np
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement02-refinement'
BASE=OUT/'restart14-canopy-animation/tree42-motion-v5'
DEST=BASE/'approved-morph-export-v1'
MODEL=BASE/'prototype.blend'
MODEL_SHA='b569c53628404fd640c582402306ba265fead93a032fd43d1c07d62e03c6eb9b'
GLB=ROOT/'level-editor/library/3d-assets/croisement02/croisement02-tree-42/model.glb'
GLB_SHA='7e716b81b82dcc28d0aac0d874c9488ff22e48622833f881d89a503a13c27b21'
MAP=ROOT/'level-editor/library/scenes/croisement02.rhlos-map.json'
MAP_SHA='616734426ba0bb56f145790cfa9013e05703ac8ab4df341dc27f4b0cc01e63ab'
APPROVAL=OUT/'restart3-review-batches/pending-v17-v23-plus-two-hub-v1/user-approval.json'
APPROVAL_SHA='ca25ba9362b26dfb8ac1239f7acd7b56929463498b125bed0f42dcd98ec628f4'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()

def verify_inputs():
    for p,h in [(MODEL,MODEL_SHA),(GLB,GLB_SHA),(MAP,MAP_SHA),(APPROVAL,APPROVAL_SHA)]:assert sha(p)==h,str(p)

def phase_weights():
    weights=np.zeros((15,13),np.float32)
    for phase in range(1,14):weights[phase,phase-1]=1
    return np.arange(15,dtype=np.float32)*4/25,weights

def map_deltas(base_world,source_world,source_delta,linear,tolerance=1e-4):
    from scipy.spatial import cKDTree
    tree=cKDTree(source_world);distance,ids=tree.query(base_world)
    assert np.max(distance)<=tolerance,('Static basis correspondence failed',float(max(distance)))
    for point,index in zip(base_world,ids):
        coincident=tree.query_ball_point(source_world[index],1e-7)
        assert np.max(abs(source_delta[:,coincident]-source_delta[:,index:index+1]))<=1e-6,'Coincident vertices have ambiguous motion'
    return source_delta[:,ids]@np.linalg.inv(linear).T,float(max(distance))

def capture():
    # Run only when assigned a Blender processing slot. This never saves a blend.
    import bpy
    verify_inputs();assert not DEST.exists();DEST.mkdir()
    bpy.ops.wm.open_mainfile(filepath=str(MODEL));scene=bpy.context.scene;scene.frame_set(1);bpy.context.view_layer.update()
    objects=[o for o in scene.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-tree-42']
    crown=next(o for o in objects if o.get('projection_component')=='crown')
    keys=crown.data.shape_keys.key_blocks;assert len(keys)==14
    matrix=np.array(crown.matrix_world);basis=np.array([v.co[:]for v in keys[0].data]);basis_world=(basis@matrix[:3,:3].T)+matrix[:3,3]
    deltas=np.array([np.array([v.co[:]for v in key.data])-basis for key in keys[1:]])@matrix[:3,:3].T
    # Blender world Z-up to delivered world Y-up; native placement is kept separately.
    basis_yup=basis_world[:,[0,2,1]]*np.array([1,1,-1]);delta_yup=deltas[:,:,[0,2,1]]*np.array([1,1,-1])
    wood={o.name:(np.array(o.matrix_world),np.array([v.co[:] for v in o.data.vertices])) for o in objects if o!=crown}
    evaluated=[]
    for phase in range(15):
        scene.frame_set(1+phase*4);bpy.context.view_layer.update()
        assert np.array_equal(np.array(crown.matrix_world),matrix),'Crown parent transform changed'
        for o in objects:
            if o!=crown:
                transform,points=wood[o.name]
                assert np.array_equal(np.array(o.matrix_world),transform) and np.array_equal(np.array([v.co[:] for v in o.data.vertices]),points),'Wood changed during crown cycle'
        expected=np.zeros(13);index=phase%14
        if index:expected[index-1]=1
        assert np.array_equal([k.value for k in keys[1:]],expected)
        mesh=crown.evaluated_get(bpy.context.evaluated_depsgraph_get()).to_mesh()
        points=np.array([v.co[:]for v in mesh.vertices]);assert points.shape==basis.shape
        wanted=basis if index==0 else np.array([v.co[:]for v in keys[index].data])
        assert np.max(abs(points-wanted))<1e-5
        crown.evaluated_get(bpy.context.evaluated_depsgraph_get()).to_mesh_clear();evaluated.append(phase)
    for o in objects:
        if o!=crown:assert o.data.shape_keys is None and o.animation_data is None,'Wood unexpectedly animated'
    np.savez_compressed(DEST/'approved-crown.npz',basis_world_yup=basis_yup,delta_world_yup=delta_yup)
    receipt=dict(status='PRIVATE_APPROVED_POSE_CAPTURE_NOT_RUNTIME_APPROVAL',source_model_sha256=MODEL_SHA,approval_sha256=APPROVAL_SHA,canonical_glb_sha256=GLB_SHA,map_sha256=MAP_SHA,crown_name=crown.name,vertices=len(basis),shape_keys=[k.name for k in keys],evaluated_phases=evaluated,arrays_sha256=sha(DEST/'approved-crown.npz'))
    (DEST/'capture.json').write_text(json.dumps(receipt,indent=2)+'\n');verify_inputs()

def augment():
    from scipy.spatial.transform import Rotation
    verify_inputs();receipt=json.loads((DEST/'capture.json').read_text());assert receipt['source_model_sha256']==MODEL_SHA
    assert receipt['arrays_sha256']==sha(DEST/'approved-crown.npz')
    target=DEST/'tree42-approved-motion.glb';assert not target.exists()
    raw=GLB.read_bytes();length=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+length]);offset=28+length
    assert not doc.get('animations') and not doc.get('skins')
    buffers=[raw[offset:offset+doc['buffers'][0]['byteLength']]]+[ (GLB.parent/b['uri']).read_bytes() for b in doc['buffers'][1:]]
    data=bytearray(buffers[0]);external_hashes=[]
    for index,buffer in enumerate(buffers[1:],1):
        data.extend(b'\0'*(-len(data)%4));start=len(data);data.extend(buffer);external_hashes.append(hashlib.sha256(buffer).hexdigest())
        for view in doc['bufferViews']:
            if view.get('buffer',0)==index:view['buffer']=0;view['byteOffset']=view.get('byteOffset',0)+start
    doc['buffers']=[{'byteLength':len(data)}]
    packet=np.load(DEST/'approved-crown.npz');basis=packet['basis_world_yup'];delta=packet['delta_world_yup']
    def accessor(index):
        a=doc['accessors'][index];v=doc['bufferViews'][a['bufferView']];assert a['componentType']==5126 and 'sparse'not in a
        width={'VEC3':3,'SCALAR':1}[a['type']]
        return np.ndarray((a['count'],width),dtype='<f4',buffer=data,offset=v.get('byteOffset',0)+a.get('byteOffset',0),strides=(v.get('byteStride',4*width),4)).copy()
    def append(array,kind):
        array=np.asarray(array,dtype='<f4');data.extend(b'\0'*(-len(data)%4));start=len(data);payload=array.tobytes();data.extend(payload)
        vi=len(doc['bufferViews']);doc['bufferViews'].append(dict(buffer=0,byteOffset=start,byteLength=len(payload)))
        width=3 if kind=='VEC3' else 1;values=array.reshape(-1,width)
        ai=len(doc['accessors']);doc['accessors'].append(dict(bufferView=vi,componentType=5126,count=len(values),type=kind,min=values.min(0).tolist(),max=values.max(0).tolist()));return ai
    placement=next(p for p in json.loads(MAP.read_text())['placements'] if p['id']=='croisement02-tree-42')['transform'];assert placement['rot_deg']==0
    placed=np.eye(4);placed[:3,3]=[placement['dx'],placement['dz'],placement['dy']/math.sin(math.radians(35))]
    nodes=[]
    def visit(index,parent):
        node=doc['nodes'][index];local=np.array(node['matrix']).reshape(4,4).T if 'matrix'in node else np.eye(4)
        if 'matrix'not in node:
            local[:3,:3]=Rotation.from_quat(node.get('rotation',[0,0,0,1])).as_matrix()@np.diag(node.get('scale',[1,1,1]));local[:3,3]=node.get('translation',[0,0,0])
        world=parent@local
        if node.get('name')==receipt['crown_name']:nodes.append((index,world))
        for child in node.get('children',[]):visit(child,world)
    for index in doc['scenes'][doc.get('scene',0)]['nodes']:visit(index,placed)
    assert len(nodes)==1;index,world=nodes[0];node=doc['nodes'][index];mesh=doc['meshes'][node['mesh']];proof=[]
    for primitive in mesh['primitives']:
        assert not primitive.get('targets');base=accessor(primitive['attributes']['POSITION']);world_points=base@world[:3,:3].T+world[:3,3]
        local_delta,error=map_deltas(world_points,basis,delta,world[:3,:3])
        primitive['targets']=[{'POSITION':append(d,'VEC3')} for d in local_delta]
        proof.append(dict(vertices=len(base),maximum_basis_correspondence_error=error))
    mesh['weights']=[0]*13;mesh.setdefault('extras',{})['targetNames']=receipt['shape_keys'][1:]
    times,weights=phase_weights();ia=append(times,'SCALAR');oa=append(weights,'SCALAR')
    doc['animations']=[dict(name='Tree42 approved physical crown cycle',samplers=[dict(input=ia,output=oa,interpolation='STEP')],channels=[dict(sampler=0,target=dict(node=index,path='weights'))])]
    doc['buffers'][0]['byteLength']=len(data);data.extend(b'\0'*(-len(data)%4))
    encoded=json.dumps(doc,separators=(',',':')).encode();encoded+=b' '*(-len(encoded)%4)
    payload=struct.pack('<III',0x46546c67,2,28+len(encoded)+len(data))+struct.pack('<II',len(encoded),0x4e4f534a)+encoded+struct.pack('<II',len(data),0x004e4942)+data
    assert len(payload)<64*1024**2;target.write_bytes(payload)
    result=dict(status='PRIVATE_MORPH_DERIVATIVE_REQUIRES_REOPEN_AND_RUNTIME_REVIEW',source_glb_sha256=GLB_SHA,capture_sha256=sha(DEST/'capture.json'),model_sha256=sha(target),external_buffers_sha256=external_hashes,primitives=proof,static_binary_prefix_exact=bytes(data[:len(buffers[0])])==buffers[0],clip=dict(phases=14,targets=13,hold_ticks=4,cycle_ticks=56,seconds=2.24,interpolation='STEP'),limits=['No canonical publication or propagation to other trees.','Material/UV/static buffer bytes preserved; actual saved appearance and shadow behavior still need review.','Morph vertex correspondence tolerance1e-4 accounts for float32 export; mismatch fails rather than changing approved motion.'])
    assert result['static_binary_prefix_exact'];(DEST/'export.json').write_text(json.dumps(result,indent=2)+'\n');verify_inputs()

if __name__=='__main__':
    assert '--capture' in sys.argv or '--augment' in sys.argv
    if '--capture' in sys.argv:
        sys.path.insert(0,str(ROOT/'level-editor/refinement'))
        from render_slots import acquire,release
        acquire()
        try:capture()
        finally:release()
    else:augment()
