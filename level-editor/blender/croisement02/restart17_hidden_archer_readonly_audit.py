"""Read-only saved endpoint role and planned support audit; no model or render writes."""
import json,sys,math
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from scipy.spatial import cKDTree
from scipy.ndimage import gaussian_filter1d
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import RAY,SIN
from refinement_review import _tree
from render_slots import acquire,release
BASE=OUT/'restart14-hidden-archer';DEST=BASE/'climbing-v17/readonly-role-support-v1'

def main():
    assert not DEST.exists();DEST.mkdir();inputs={};records=[]
    def read(path):
        inputs[str(path)]=sha(path);return json.loads(path.read_text())
    attachment=read(BASE/'skeleton-v9-cpu/root-attachment-final-centers.json');paths=read(BASE/'geodesic-v8-cpu/report.json')['paths']
    surface=BASE/'surface-v8/surfaces.npz';inputs[str(surface)]=sha(surface);data=np.load(surface);rv=data['vertices0'];rt=data['triangles0'];q=rv[rt];normals=np.zeros_like(rv);fn=np.cross(q[:,1]-q[:,0],q[:,2]-q[:,0])
    for k in range(3):np.add.at(normals,rt[:,k],fn)
    normals/=np.maximum(np.linalg.norm(normals,axis=1)[:,None],1e-12);tree=cKDTree(rv);guides=[]
    for k,path in enumerate(paths):
        p=np.array(path['points']);n=gaussian_filter1d(normals[tree.query(p)[1]],1.5,axis=0);n/=np.maximum(np.linalg.norm(n,axis=1)[:,None],1e-12);guides.append(np.vstack([attachment['states'][0]['rock_path_roots'][k],p+n*4]))
    for state in ['initial','applied']:
        folder=BASE/f'climbing-v17/profile-05-{state}';construction=read(folder/'construction.json');model=folder/'model.blend';inputs[str(model)]=sha(model);assert inputs[str(model)]==construction['model_sha256'];plan=read(BASE/f'skeleton-v9-cpu/{state}-plan.json');arrangement=read(BASE/f'lobes-v16-cpu/{state}-arrangement.json');a=next(r for r in attachment['states'] if r['state']==state)
        front=np.array(plan['front'])-np.array(RAY)*.6;chains=guides+[front[c] for c in plan['segments']]+[np.array(a['climber_join']),np.array(a['right_branch'])];core_count=len(chains);lobes=arrangement['lobes']+arrangement['offmap_lobes'];chains += [np.array([r['twig_base'],r['twig_tip']]) for r in lobes]
        primary={0,1,2,core_count-1};segments=[]
        for ci,c in enumerate(chains):
            radius=.65 if ci in primary else .18 if ci<core_count else .12
            for si,(p,q) in enumerate(zip(c[:-1],c[1:])):
                if np.linalg.norm(q-p)>1e-7:segments.append((ci,p,q,radius*(1-.35*si/max(1,len(c)-1))))
        starts=np.array([s[1] for s in segments]);ends=np.array([s[2] for s in segments]);dirs=ends-starts;lens=np.einsum('ij,ij->i',dirs,dirs);owners=np.array([s[0] for s in segments]);radii=np.array([s[3] for s in segments]);edges=set();endpoint_rows=[]
        for ci,c in enumerate(chains):
            radius=.65 if ci in primary else .18 if ci<core_count else .12
            for end,p in [('base',c[0]),('tip',c[-1])]:
                t=np.clip(np.einsum('ij,ij->i',p-starts,dirs)/lens,0,1);distance=np.linalg.norm(starts+t[:,None]*dirs-p,axis=1);distance[owners==ci]=np.inf;gap=distance-radii-radius;hits=np.where(gap<=1e-5)[0]
                for j in hits:edges.add(tuple(sorted((ci,int(owners[j])))))
                j=int(np.argmin(gap));endpoint_rows.append(dict(chain=ci,end=end,nearest_chain=int(owners[j]),centerline_distance=float(distance[j]),conservative_radius_gap=float(gap[j])))
        adjacency={i:set() for i in range(len(chains))}
        for i,j in edges:adjacency[i].add(j);adjacency[j].add(i)
        reachable=set(range(len(guides)));stack=list(reachable)
        while stack:
            for j in adjacency[stack.pop()]-reachable:reachable.add(j);stack.append(j)
        bpy.ops.wm.open_mainfile(filepath=str(model));objects=[o for o in bpy.context.scene.objects if o.type=='MESH'];assert len(objects)==1 and not objects[0].modifiers;obj=objects[0];mesh=obj.data;mesh.calc_loop_triangles();bvh,_,_=_tree(objects)
        rgba=np.asarray(Image.open(construction['source']).convert('RGBA'));h,w=rgba.shape[:2];ox,oy=construction['source_top_left'];native_count=int((rgba[:,:,3]>=128).sum());native_faces=native_count*2;stem_faces=10*len(segments);exceptions=[];missing=[]
        for y,x in np.argwhere(rgba[:,:,3]>=128):
            origin=Vector((ox+float(x)+.5,-(oy+float(y)+.5)/SIN,0))+RAY*6000;hit,_,index,_=bvh.ray_cast(origin,-RAY)
            if hit is None:missing.append([int(ox+x),int(oy+y)]);continue
            tri=mesh.loop_triangles[index];mat=mesh.materials[tri.material_index]
            if mat.get('foliage_observed'):continue
            polygon=mesh.polygons[tri.polygon_index];uv=mesh.uv_layers['Foliage UV'];p0,p1,p2=[obj.matrix_world@mesh.vertices[v].co for v in tri.vertices];e,f,v=p1-p0,p2-p0,hit-p0;ee,ef,ff=e.dot(e),e.dot(f),f.dot(f);den=ee*ff-ef*ef;b=(ff*v.dot(e)-ef*v.dot(f))/den;c=(ee*v.dot(f)-ef*v.dot(e))/den;u=uv.data[tri.loops[0]].uv*(1-b-c)+uv.data[tri.loops[1]].uv*b+uv.data[tri.loops[2]].uv*c;sample=[math.floor(u.x*w),h-1-math.floor(u.y*h)]
            role='native paired back' if polygon.index<native_faces else 'inferred supporting stem' if polygon.index<native_faces+stem_faces else 'inferred side leaf'
            exceptions.append(dict(pixel=[int(ox+x),int(oy+y)],world=list(hit),triangle=index,polygon=polygon.index,material=mat.name,material_slot=tri.material_index,construction_role=role,source_sample_local=sample,expected_sample_local=[int(x),int(y)],exact_source_sample=sample==[int(x),int(y)],source_ownership=[float(mesh.color_attributes['Source ownership'].data[k].color[0]) for k in tri.loops]))
        records.append(dict(state=state,model_sha256=inputs[str(model)],native_pixels=native_count,missing=missing,nonobserved_first_hits=exceptions,planned_support_graph=dict(core_chains=core_count,lobe_twigs=len(lobes),chains=len(chains),stem_segments=len(segments),edges=[list(e) for e in sorted(edges)],root_guide_chains=list(range(len(guides))),reachable_from_root_guides=sorted(reachable),unreached_chains=sorted(set(adjacency)-reachable),endpoints=endpoint_rows),limitations=['Endpoint-to-segment radius proximity tests planned tubes, not actual triangle intersection or opaque material continuity. Interior/interior crossings may add contacts not recorded.','Gray detached-looking leaf packets require actual twig-to-leaf and rock/bank context review; an ellipsoid envelope is not a physical support surface.']))
    for path,digest in inputs.items():assert sha(Path(path))==digest
    result=dict(status='READ_ONLY_AUDIT; geometry readiness remains held',inputs=inputs,records=records,model_saved=False,rendered=False,output_cap_bytes=2*1024**2)
    encoded=json.dumps(result,indent=2)+'\n';assert len(encoded.encode())<2*1024**2;(DEST/'report.json').write_text(encoded);print(DEST/'report.json',flush=True)
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
