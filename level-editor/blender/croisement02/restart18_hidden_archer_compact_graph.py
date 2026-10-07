"""Build a private CPU local-support forest beneath exact saved leaf faces.

This is a candidate graph, never an accepted geometry or clearance receipt.
"""
import base64,hashlib,json,zlib
import numpy as np
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import dijkstra,connected_components
from scipy.spatial import cKDTree
from restart18_hidden_archer_route_cpu import BASE,RockDepth,RAY,screen,sha
DEST=BASE/'climbing-v17/compact-support-cpu-v2'
def unpack(record):
    dtype=np.dtype(record['dtype']);b=np.frombuffer(zlib.decompress(base64.b64decode(record['data'])),dtype='u1').reshape(dtype.itemsize,-1).T.copy().tobytes();assert hashlib.sha256(b).hexdigest()==record['raw_sha256'];return np.frombuffer(b,dtype=dtype).reshape(record['shape'])
def main():
    assert not DEST.exists();DEST.mkdir();p=BASE/'climbing-v17/exact-geometry-readonly-v2/report.json';report=json.loads(p.read_text());sp=BASE/'surface-v8/surfaces.npz';data=np.load(sp);rock=RockDepth(data['vertices0'][data['triangles0']]);states=[]
    for state in report['states']:
        a={k:unpack(v) for k,v in state['arrays'].items()};v=a['world_vertices'];loops=a['polygon_loop_vertices'];offset=a['polygon_offsets'];hits=a['native_first_hits'];count=state['native_pixels'];centers=[]
        for k in range(count):
            polygon=2*k;assert a['polygon_ownership'][polygon]==1;centers.append(v[loops[offset[polygon]:offset[polygon+1]]].mean(0))
        centers=np.array(centers);support=centers-RAY*.5;xy=screen(centers);pixel=np.floor(xy).astype(int);lookup={tuple(px):i for i,px in enumerate(pixel)};assert len(lookup)==count
        edges=[]
        for i,(x,y) in enumerate(pixel):
            for delta in [(1,0),(0,1)]:
                j=lookup.get((x+delta[0],y+delta[1]))
                if j is not None:edges.append((i,j))
        edges=np.array(edges);length=np.linalg.norm(support[edges[:,0]]-support[edges[:,1]],axis=1);graph=coo_matrix((np.r_[length,length],(np.r_[edges[:,0],edges[:,1]],np.r_[edges[:,1],edges[:,0]])),shape=(count,count)).tocsr();n,component=connected_components(graph,directed=False);depth=rock.front(xy);gap=centers@RAY-depth;eligible=np.isfinite(depth)&(gap>1.8)&(gap<12);arr=json.loads(__import__('pathlib').Path(state['lobe_envelopes']).read_text());seed_pixels=np.array([l['seed_pixel'] for l in arr['lobes']]);seed_indices=cKDTree(pixel).query(seed_pixels)[1];anchor_indices=[];used_edges=set();routes=[];unanchored=[];rejected_detours=[]
        for seed in seed_indices:
            dist,pred=dijkstra(graph,indices=int(seed),return_predecessors=True);possible=np.flatnonzero(eligible&(component==component[seed]));existing=np.array([i for i in anchor_indices if component[i]==component[seed]],dtype=int)
            if len(existing) and dist[existing].min()<=12:anchor=int(existing[np.argmin(dist[existing])])
            elif len(possible):
                score=dist[possible]+gap[possible]*.25;anchor=int(possible[np.argmin(score)])
            else:unanchored.append(int(seed));continue
            if dist[anchor]>16:
                rejected_detours.append(dict(seed=int(seed),shortest_local_anchor_route=float(dist[anchor])));unanchored.append(int(seed));continue
            anchor_indices.append(anchor)
            route=[anchor];cursor=anchor
            while cursor!=seed:
                previous=int(pred[cursor]);assert previous>=0;used_edges.add(tuple(sorted((cursor,previous))));route.append(previous);cursor=previous
            routes.append(dict(seed=int(seed),anchor=anchor,branch_length=float(dist[anchor]),nodes=route[::-1]))
        anchors=[]
        for i in sorted(set(anchor_indices)):
            rockpoint=centers[i]+RAY*(depth[i]-centers[i]@RAY);anchors.append(dict(node=i,native_pixel=pixel[i].tolist(),support=support[i].tolist(),geometric_rock_hit=rockpoint.tolist(),attachment_length=float(np.linalg.norm(support[i]-rockpoint)),hypothesis='Local inferred rock-crevice attachment; not an observed root.'))
        selected=sorted({i for edge in used_edges for i in edge}|set(anchor_indices));states.append(dict(state=state['state'],native_pixels=count,four_connected_components=n,nodes=[dict(index=i,pixel=pixel[i].tolist(),world=support[i].tolist()) for i in selected],edges=[list(e) for e in sorted(used_edges)],candidate_radius=.12,anchors=anchors,lobe_routes=routes,unanchored_lobe_seeds=sorted(set(unanchored)),rejected_long_detours=rejected_detours,unanchored_component_pixels=[pixel[component==ci].tolist() for ci in range(n) if not eligible[component==ci].any()],maximum_local_branch_length=max((r['branch_length'] for r in routes),default=0),scope='Historical rock guides, climber join and independent bank stem are excluded. Only short local native-covered branch hypotheses remain.'))
    result=dict(status='CPU CANDIDATE HOLD',inputs={str(p):sha(p),str(sp):sha(sp)},states=states,remaining_guards=['Use exact leaf-plane first-hit depths along complete finite-radius tubes, not centerlines alone.','Certify triangles against rock/bank with intended endpoint-contact exceptions only.','Resolve tiny alpha-disconnected islands and off-map fans with compact explicit local attachment; no unsupported lobe may survive.','Prune or reposition only inferred hidden fans around the compact branches; preserve every observed leaf polygon/material/UV.','Validate paired saved candidates at all7073 native centers plus source extras, actual/solid8 and contact views.'])
    (DEST/'report.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps([dict(state=s['state'],anchors=len(s['anchors']),edges=len(s['edges']),max_branch=s['maximum_local_branch_length'],unanchored_lobes=len(s['unanchored_lobe_seeds'])) for s in states]))
if __name__=='__main__':main()
