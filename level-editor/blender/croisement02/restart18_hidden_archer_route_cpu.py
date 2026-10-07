"""CPU geometric visibility audit for continuous exterior climbing supports.

Geometric rock occlusion is a planning filter, not a material-opacity certificate.
Saved-model first-hit and finite-radius clearance checks remain mandatory.
"""
import json, math, hashlib
from pathlib import Path
import numpy as np
from scipy.spatial import cKDTree
from scipy.ndimage import gaussian_filter1d
from PIL import Image
BASE=Path(__file__).resolve().parents[2]/'work/croisement02-refinement/restart14-hidden-archer'
DEST=BASE/'climbing-v17/exterior-routing-cpu-v1'
SIN=math.sin(math.radians(35)); COS=math.cos(math.radians(35)); RAY=np.array([0,-COS,SIN])
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def screen(p):return np.stack((p[...,0],-p[...,1]*SIN-p[...,2]*COS),axis=-1)
class RockDepth:
    def __init__(self,tri):
        self.tri=tri; self.xy=screen(tri);self.centers=self.xy.mean(1)
        self.radius=np.linalg.norm(self.xy-self.centers[:,None,:],axis=2).max(1)
        self.tree=cKDTree(self.centers);self.depth=tri@RAY
    def front(self,points):
        result=np.full(len(points),-np.inf)
        for i,(point,ids) in enumerate(zip(points,self.tree.query_ball_point(points,float(self.radius.max())+1e-6))):
            if not ids:continue
            ids=np.array(ids); t=self.xy[ids];a=t[:,1]-t[:,0];b=t[:,2]-t[:,0];p=point-t[:,0];den=a[:,0]*b[:,1]-a[:,1]*b[:,0]
            valid=np.abs(den)>1e-10
            u=np.divide(p[:,0]*b[:,1]-p[:,1]*b[:,0],den,out=np.zeros_like(den),where=valid)
            v=np.divide(a[:,0]*p[:,1]-a[:,1]*p[:,0],den,out=np.zeros_like(den),where=valid)
            valid &= (u>=-1e-7)&(v>=-1e-7)&(u+v<=1+1e-7)
            if valid.any():result[i]=(self.depth[ids,0]+u*(self.depth[ids,1]-self.depth[ids,0])+v*(self.depth[ids,2]-self.depth[ids,0]))[valid].max()
        return result

def main():
    assert not DEST.exists(); DEST.mkdir();inputs={}
    def read(p):inputs[str(p)]=sha(p);return json.loads(p.read_text())
    sp=BASE/'surface-v8/surfaces.npz';inputs[str(sp)]=sha(sp);data=np.load(sp);rv=data['vertices0'];rt=data['triangles0'];tri=rv[rt];depth=RockDepth(tri)
    normals=np.zeros_like(rv);fn=np.cross(tri[:,1]-tri[:,0],tri[:,2]-tri[:,0])
    for k in range(3):np.add.at(normals,rt[:,k],fn)
    normals/=np.maximum(np.linalg.norm(normals,axis=1)[:,None],1e-12);tree=cKDTree(rv)
    attachment=read(BASE/'skeleton-v9-cpu/root-attachment-final-centers.json'); paths=read(BASE/'geodesic-v8-cpu/report.json')['paths'];guides=[]
    for k,path in enumerate(paths):
        p=np.array(path['points']);n=gaussian_filter1d(normals[tree.query(p)[1]],1.5,axis=0);n/=np.maximum(np.linalg.norm(n,axis=1)[:,None],1e-12);guides.append(np.vstack([attachment['states'][0]['rock_path_roots'][k],p+n*4]))
    records=[]
    for state in ['initial','applied']:
        plan=read(BASE/f'skeleton-v9-cpu/{state}-plan.json');arr=read(BASE/f'lobes-v16-cpu/{state}-arrangement.json');source=Path(plan['source']);inputs[str(source)]=sha(source);rgba=np.array(Image.open(source).convert('RGBA'));h,w=rgba.shape[:2];ox,oy=plan['source_top_left'];a=next(r for r in attachment['states'] if r['state']==state);front=np.array(plan['front'])-RAY*.6
        chains=guides+[front[c] for c in plan['segments']]+[np.array(a['climber_join']),np.array(a['right_branch'])];core=len(chains);chains += [np.array([l['twig_base'],l['twig_tip']]) for l in arr['lobes']+arr['offmap_lobes']]
        for ci,chain in enumerate(chains):
            if ci==20:continue # Independent bank stem removed by the reviewed proposal.
            points=np.concatenate([p+(q-p)*np.linspace(0,1,max(2,math.ceil(np.linalg.norm(q-p)/.5)+1))[:,None] for p,q in zip(chain[:-1],chain[1:])]);xy=screen(points);ix=np.floor(xy[:,0]-ox).astype(int);iy=np.floor(xy[:,1]-oy).astype(int);inside=(ix>=0)&(ix<w)&(iy>=0)&(iy<h);native=np.zeros(len(points),bool);native[inside]=rgba[iy[inside],ix[inside],3]>=128;offmap=xy[:,1]<0;rock=depth.front(xy);hidden=rock>points@RAY+.8
            unsupported=~native&~offmap&~hidden
            records.append(dict(state=state,chain=ci,samples=len(points),native_projected=int(native.sum()),geometrically_rock_occluded=int(hidden.sum()),offmap=int(offmap.sum()),exposed_native_empty=int(unsupported.sum()),exposed_examples=points[unsupported][:4].tolist()))
    # Exterior surface candidates: evaluate actual triangle normals, without
    # claiming offset edges clear at a concave join or through texture holes.
    centers=tri.mean(1);norm=fn/np.maximum(np.linalg.norm(fn,axis=1)[:,None],1e-12);eligible=(centers[:,0]>=95)&(centers[:,2]>=42);ids=np.flatnonzero(eligible);candidate=centers[ids]+norm[ids]*1.5;candidate_front=depth.front(screen(candidate));hidden=candidate_front>candidate@RAY+.8
    np.savez_compressed(DEST/'surface-candidates.npz',triangle_ids=ids,candidate=candidate,geometrically_hidden=hidden)
    result=dict(status='PLANNING HOLD; no saved geometry mutation',inputs=inputs,chain_centerline_visibility=records,exterior_surface_candidates=dict(total=len(ids),geometrically_hidden=int(hidden.sum()),offset_world=1.5),limitations=['Centerline classification alone does not certify the tube radius or native leaf first-hit depth.','Rock geometric occlusion may cross material apertures; alpha-aware saved-model proof is required.','Candidate surface offsets require exact triangle clearance along every routed edge before construction.'],next_step='Build a connected exterior route through rock-occluded candidate faces, joining native-covered branch endpoints without unsupported leaf lobes.')
    (DEST/'report.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result['exterior_surface_candidates']));print(json.dumps({s:sum(r['exposed_native_empty'] for r in records if r['state']==s) for s in ['initial','applied']}))
if __name__=='__main__':main()
