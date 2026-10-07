"""Bounded read-only exact climbing geometry and alpha-aware native hit extraction.

Run only with an explicitly granted FIFO lane. No save, render, or cache output.
The JSON contains lossless compressed arrays, not rounded geometry summaries.
"""
import base64,hashlib,json,sys,zlib
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha
from tree_geometry import RAY,SIN
from refinement_review import _tree
from render_slots import acquire,release
BASE=OUT/'restart14-hidden-archer';DEST=BASE/'climbing-v17/exact-geometry-readonly-v2';CAP=3*1024**2

def pack(value):
    a=np.ascontiguousarray(value);raw=a.tobytes();shuffled=a.view('u1').reshape(-1,a.dtype.itemsize).T.copy().tobytes();compressed=zlib.compress(shuffled,9)
    decoded=np.frombuffer(zlib.decompress(compressed),dtype='u1').reshape(a.dtype.itemsize,-1).T.copy().tobytes();assert decoded==raw
    return dict(dtype=a.dtype.str,shape=list(a.shape),raw_sha256=hashlib.sha256(raw).hexdigest(),encoding='base64-zlib-byteplanes',data=base64.b64encode(compressed).decode())

def main():
    assert not DEST.exists();bindings={};states=[]
    for state in ['initial','applied']:
        folder=BASE/f'climbing-v17/profile-05-{state}';model=folder/'model.blend';cp=folder/'construction.json';construction=json.loads(cp.read_text());bindings[str(cp)]=sha(cp);bindings[str(model)]=sha(model);assert bindings[str(model)]==construction['model_sha256'];src=Path(construction['source']);bindings[str(src)]=sha(src);assert bindings[str(src)]==construction['source_sha256'];arr=BASE/f'lobes-v16-cpu/{state}-arrangement.json';bindings[str(arr)]=sha(arr)
        bpy.ops.wm.open_mainfile(filepath=str(model));objs=[o for o in bpy.context.scene.objects if o.type=='MESH'];assert len(objs)==1 and not objs[0].modifiers;obj=objs[0];mesh=obj.data;mesh.calc_loop_triangles();assert np.allclose(np.array(obj.matrix_world),np.eye(4),rtol=0,atol=0),'Expected identity object transform for lossless local/world extraction'
        vertices=np.array([v.co[:] for v in mesh.vertices],dtype='<f4');unique,remap=np.unique(vertices,axis=0,return_inverse=True);loops=np.array([remap[l.vertex_index] for l in mesh.loops],dtype='<u4');uv=np.array([v.uv[:] for v in mesh.uv_layers['Foliage UV'].data],dtype='<f4');offsets=np.array([p.loop_start for p in mesh.polygons]+[len(mesh.loops)],dtype='<u4');slots=np.array([p.material_index for p in mesh.polygons],dtype='u1');ownership=np.array([mesh.color_attributes['Source ownership'].data[p.loop_start].color[0] for p in mesh.polygons],dtype='<f4');triangles=np.array([t.loops[:] for t in mesh.loop_triangles],dtype='<u4');triangle_polygons=np.array([t.polygon_index for t in mesh.loop_triangles],dtype='<u4')
        for p in mesh.polygons:assert all(mesh.color_attributes['Source ownership'].data[l].color[0]==ownership[p.index] for l in p.loop_indices)
        mats=[]
        for slot,mat in enumerate(mesh.materials):
            shader=next(n for n in mat.node_tree.nodes if n.type=='BSDF_PRINCIPLED');links=shader.inputs['Alpha'].links;assert len(links)==1;tex=links[0].from_node;assert tex.type=='TEX_IMAGE' and tex.image and tex.image.packed_file
            mats.append(dict(slot=slot,name=mat.name,observed=bool(mat.get('foliage_observed')),physical_opacity=bool(mat.get('foliage_physical_opacity')),semantics=mat.get('opacity_semantics'),cull=bool(mat.use_backface_culling or mat.get('foliage_card_sides')=='paired-one-sided'),interpolation=tex.interpolation,extension=tex.extension,image_size=list(tex.image.size),packed_image_sha256=hashlib.sha256(tex.image.packed_file.data).hexdigest()))
        rgba=np.array(Image.open(src).convert('RGBA'));h,w=rgba.shape[:2];ox,oy=construction['source_top_left'];bvh,_,_=_tree(objs);hits=[];failures=[]
        for y,x in np.argwhere(rgba[:,:,3]>=128):
            origin=Vector((ox+float(x)+.5,-(oy+float(y)+.5)/SIN,0))+RAY*6000;hit,normal,tid,_=bvh.ray_cast(origin,-RAY)
            if hit is None:failures.append(dict(pixel=[int(x),int(y)],reason='missing'));continue
            t=mesh.loop_triangles[tid];p0,p1,p2=[obj.matrix_world@mesh.vertices[v].co for v in t.vertices];a,b,c=p1-p0,p2-p0,hit-p0;aa,ab,bb=a.dot(a),a.dot(b),b.dot(b);den=aa*bb-ab*ab;u=(bb*c.dot(a)-ab*c.dot(b))/den;v=(aa*c.dot(b)-ab*c.dot(a))/den;sample_uv=uv[t.loops[0]]*(1-u-v)+uv[t.loops[1]]*u+uv[t.loops[2]]*v;sx=int(np.floor(sample_uv[0]*w));sy=h-1-int(np.floor(sample_uv[1]*h));exact=(sx==x and sy==y)
            if not exact:failures.append(dict(pixel=[int(x),int(y)],reason='different source sample',sample=[sx,sy]))
            hits.append([int(x),int(y),int(tid),int(t.polygon_index),*hit[:],float(sample_uv[0]),float(sample_uv[1])])
        assert not failures,failures[:8];assert len(hits)==construction['source_opaque_centers'];native_faces=len(hits)*2
        states.append(dict(state=state,model_sha256=bindings[str(model)],source=str(src),source_top_left=[ox,oy],native_pixels=len(hits),native_paired_polygon_range=[0,native_faces],materials=mats,arrays={name:pack(a) for name,a in dict(world_vertices=unique,polygon_loop_vertices=loops,loop_uv=uv,polygon_offsets=offsets,polygon_materials=slots,polygon_ownership=ownership,triangle_loops=triangles,triangle_polygons=triangle_polygons,native_first_hits=np.array(hits,dtype='<f8')).items()},native_first_hit_columns=['source_local_x','source_local_y','triangle','polygon','world_x','world_y','world_z','u','v'],native_alpha_aware_hit_status='PASS: every expected native pixel has exact own-source UV sample',lobe_envelopes=str(arr),limitations=['Exact saved geometry is extracted; finite-radius replacement support clearance has not yet been tested.','Lobe envelopes and planned twig associations are hash-bound separately; actual fan polygons are included in geometry arrays.']))
    for p,digest in bindings.items():assert sha(Path(p))==digest
    result=dict(status='READ_ONLY_EXACT_EXTRACTION; construction remains held',inputs=bindings,states=states,native_expected_total=7073,model_saved=False,rendered=False,output_cap_bytes=CAP);assert sum(s['native_pixels'] for s in states)==7073;encoded=(json.dumps(result,separators=(',',':'))+'\n').encode();assert len(encoded)<=CAP,f'Output exceeds bounded audit cap: {len(encoded)}';DEST.mkdir();(DEST/'report.json').write_bytes(encoded);print(json.dumps(dict(path=str(DEST/'report.json'),bytes=len(encoded),sha256=sha(DEST/'report.json'))),flush=True)
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
