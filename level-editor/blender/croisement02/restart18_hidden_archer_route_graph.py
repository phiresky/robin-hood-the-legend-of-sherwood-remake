"""Test exterior rock-route connectivity without saving or rendering models."""
import json
import numpy as np
from scipy.spatial import cKDTree
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import dijkstra
from PIL import Image
from restart18_hidden_archer_route_cpu import BASE,screen,sha
DEST=BASE/'climbing-v17/exterior-routing-graph-v1'
def main():
    assert not DEST.exists();DEST.mkdir()
    sp=BASE/'surface-v8/surfaces.npz';cp=BASE/'climbing-v17/exterior-routing-cpu-v1/surface-candidates.npz';d=np.load(sp);c=np.load(cp);ids=c['triangle_ids'];points=c['candidate'];hidden=c['geometrically_hidden'];tri=d['triangles0'][ids];vertices=d['vertices0']
    _,weld=np.unique(np.round(vertices,5),axis=0,return_inverse=True);tri=weld[tri];edges=np.sort(np.concatenate([tri[:,[0,1]],tri[:,[1,2]],tri[:,[2,0]]]),axis=1);owners=np.tile(np.arange(len(tri)),3);order=np.lexsort((edges[:,1],edges[:,0]));edges=edges[order];owners=owners[order];paired=np.flatnonzero((edges[1:]==edges[:-1]).all(1));a=owners[paired];b=owners[paired+1];length=np.linalg.norm(points[a]-points[b],axis=1)
    xy=screen(points);reports=[]
    rootdoc=json.loads((BASE/'skeleton-v9-cpu/root-attachment-final-centers.json').read_text());guide=json.loads((BASE/'geodesic-v8-cpu/report.json').read_text())['paths']
    for state in ['initial','applied']:
        plan=json.loads((BASE/f'skeleton-v9-cpu/{state}-plan.json').read_text());rgba=np.array(Image.open(plan['source']).convert('RGBA'));h,w=rgba.shape[:2];ix=np.floor(xy[:,0]-plan['source_top_left'][0]).astype(int);iy=np.floor(xy[:,1]-plan['source_top_left'][1]).astype(int);inside=(ix>=0)&(ix<w)&(iy>=0)&(iy<h);native=np.zeros(len(points),bool);native[inside]=rgba[iy[inside],ix[inside],3]>=128;allowed=hidden|native|(xy[:,1]<0);valid=allowed[a]&allowed[b]
        graph=coo_matrix((np.r_[length[valid],length[valid]],(np.r_[a[valid],b[valid]],np.r_[b[valid],a[valid]])),shape=(len(points),len(points))).tocsr();allowedids=np.flatnonzero(allowed);tree=cKDTree(points[allowedids]);r=next(x for x in rootdoc['states'] if x['state']==state);paths=[]
        for k,g in enumerate(guide):
            start=np.array(r['rock_path_roots'][k]);end=np.array(g['points'][-1]);sd,si=tree.query(start);ed,ei=tree.query(end);si=int(allowedids[si]);ei=int(allowedids[ei]);dist,previous=dijkstra(graph,indices=si,return_predecessors=True);route=[]
            if np.isfinite(dist[ei]):
                cursor=ei
                while cursor!=si:route.append(cursor);cursor=int(previous[cursor]);assert cursor>=0
                route.append(si);route.reverse()
            paths.append(dict(guide=k,connected=bool(route),start_connector_distance=float(sd),end_connector_distance=float(ed),route_length=float(dist[ei]) if route else None,points=points[route].tolist(),geometrically_hidden_nodes=int(hidden[route].sum()) if route else 0))
        reports.append(dict(state=state,allowed_nodes=int(allowed.sum()),paths=paths))
    result=dict(status='CPU route hypothesis only; HOLD',inputs={str(sp):sha(sp),str(cp):sha(cp)},states=reports,limitations=['Native-alpha node inclusion does not certify support behind saved observed planes.','Every edge and endpoint connector still needs finite-radius clearance and alpha-aware native preservation.','Sparse triangle offsets may cross concave geometry; no candidate is construction-ready.'])
    (DEST/'report.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps([{**x,'paths':[{k:v for k,v in p.items() if k!='points'} for p in x['paths']]} for x in reports]))
if __name__=='__main__':main()
