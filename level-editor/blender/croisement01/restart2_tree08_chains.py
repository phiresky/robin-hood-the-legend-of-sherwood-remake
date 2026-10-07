"""Continuous source-chain sweeps and explicitly owned missing wood support."""
import argparse,hashlib,json,math
from pathlib import Path
import numpy as np
from scipy.ndimage import gaussian_filter1d
from scipy.spatial import cKDTree
from PIL import Image
from restart2_tree08_transport import curvature_limited_sweep,strip_orientation
from restart2_tree08_transport_audit import coverage
R=Path(__file__).resolve().parents[3]/'level-editor/work/croisement01-refinement/restart2'
S,C=math.sin(math.radians(35)),math.cos(math.radians(35));RAY=np.array([0,-C,S]);DOWN=np.array([0,-S,-C]);RIGHT=np.array([1.,0,0])


def project(v):
    return np.column_stack((v[:,0],-v[:,1]*S-v[:,2]*C))


def triangle_faces(rings,n=16):
    faces=[]
    for j in range(rings-1):
        for k in range(n):
            a=j*n+k;b=j*n+(k+1)%n;faces.extend([(a,b,b+n),(a,b+n,a+n)])
    for k in range(1,n-1):faces.extend([(0,k+1,k),((rings-1)*n,(rings-1)*n+k,(rings-1)*n+k+1)])
    return np.asarray(faces,dtype=np.int32)


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--revision',type=int,default=3);parser.add_argument('--repairs',type=int,default=6);args=parser.parse_args()
    out=R/f'tree08-v12-chain-cpu-v{args.revision}';out.mkdir(exist_ok=False)
    source=R/'tree08-v11-curvature-cpu-v7';old=json.loads((source/'report.json').read_text());archive=np.load(source/'mesh.npz');lookup={s['trace_id']:dict(vertices=archive[f"vertices_{s['index']}"],faces=archive[f"faces_{s['index']}"],held_crossing=s['held_crossing']) for s in old['mesh_sections']}
    review=json.loads((R/'tree08-wood-prototype-v11/self-review.json').read_text());chains=review['next_cpu_construction']['degree_two_chains'];joined_ids={s['trace_id'] for ch in chains for s in ch['sections']};sections=[];records=[]
    for trace_id,section in lookup.items():
        if trace_id not in joined_ids:sections.append(dict(section,trace_id=trace_id,source_traces=[trace_id]))
    for chain in chains:
        raw=[];members=[];joins=[]
        for item in chain['sections']:
            trace_id=item['trace_id'];members.append(trace_id);rings=lookup[trace_id]['vertices'].reshape(-1,16,3).copy()
            if item['reverse']:rings=rings[::-1]
            if raw:
                prior=raw[-1];center=prior.mean(0);nextcenter=rings[0].mean(0)
                assert np.linalg.norm(center-nextcenter)<1e-6,'Projected coincidence is not a physical join'
                radius=max(np.linalg.norm(prior[0]-center),np.linalg.norm(rings[0,0]-nextcenter));direction=prior-center;raw[-1]=center+direction*radius/np.linalg.norm(direction[0]);joins.append(len(raw)-1);raw.extend(rings[1:])
            else:raw.extend(rings)
        raw=np.asarray(raw);centers=raw.mean(1);radial=raw-centers[:,None,:];radii=np.linalg.norm(radial[:,0],axis=1)
        # Smooth the former endpoint radius steps across their shared samples.
        smooth=gaussian_filter1d(radii,2,mode='nearest');raw=centers[:,None,:]+radial*(smooth/radii)[:,None,None]
        vertices,audit=curvature_limited_sweep(raw.reshape(-1,3))
        record=dict(trace_id='chain-'+ '-'.join(map(str,members)),source_traces=members,removed_internal_cap_pairs=len(members)-1,former_join_rings=joins,**audit);records.append(record)
        sections.append(dict(trace_id=record['trace_id'],source_traces=members,vertices=vertices,faces=triangle_faces(len(vertices)//16),held_crossing=any(lookup[t]['held_crossing'] for t in members)))
    # The omitted source traces are disconnected in the native skeleton. Keep
    # them independent; infer only ray depth from their closest retained limb.
    source_traces=json.loads((R/'tree08-source-trace-v2/trace.json').read_text())['polylines'];allcenters=np.concatenate([s['vertices'].reshape(-1,16,3).mean(1) for s in sections]);tree=cKDTree(project(allcenters));missing_routes=json.loads((R/'tree08-wood-prototype-v11/cpu-junction-review/report.json').read_text())['native_misses']
    for trace_id in [12,167,'native-bark-fragment-542-186']:
        if isinstance(trace_id,int):path=np.array(source_traces[trace_id],dtype=float)
        else:path=np.array([[542.5,185.5,1.5],[542.5,186.5,1.5],[543.2,187.5,1.5],[543.5,188.5,1.5],[544.5,189.5,1.5],[545.5,190.5,1.5]])
        if trace_id==12:
            tangent=path[-1,:2]-path[-2,:2];tangent/=np.linalg.norm(tangent);path=np.vstack((path,np.r_[path[-1,:2]+tangent,1.5]))
        if trace_id==167:
            # Explicit short continuation to the five owned upper-edge pixels.
            # This does not connect the fragment to an unrelated nearby bough.
            path=np.vstack(([644,223,1.4],[643,223,1.4],[642,223,1.4],[641,223,1.4],[641,224,1.4],[641,225,1.4],[640,226,1.4],path))
        _,index=tree.query(path[:,:2].mean(0));anchor=allcenters[index];anchor_native=project(anchor[None,:])[0];centers=np.array([anchor+RIGHT*(p[0]-anchor_native[0])+DOWN*(p[1]-anchor_native[1]) for p in path]);radii=np.maximum(path[:,2]*1.5,1.5)
        raw=np.array([center+r*(RIGHT*math.cos(a)+RAY*math.sin(a)) for center,r in zip(centers,radii) for a in np.arange(16)*math.tau/16]);vertices,audit=curvature_limited_sweep(raw)
        sections.append(dict(trace_id=trace_id,source_traces=[trace_id],vertices=vertices,faces=triangle_faces(len(vertices)//16),held_crossing=True));records.append(dict(trace_id=trace_id,disconnected_native_trace=True,inferred_depth_anchor_native=anchor_native.tolist(),**audit))
    checks=[dict(trace_id=s['trace_id'],**strip_orientation(s['vertices'],16)) for s in sections]
    bad=sum(c['nonoutward'] for c in checks);reduced=[r['trace_id'] for r in records if r.get('max_radius_reduction',0)>1e-6]
    before=coverage(list(lookup.values()));core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0
    hit_cache=[coverage([section]) for section in sections];hit=np.logical_or.reduce(hit_cache);repairs=[]
    owner={trace_id:index for index,section in enumerate(sections) for trace_id in section['source_traces']}
    assigned={tuple(r['native']):r['nearest_source_trace'] for r in missing_routes}
    for native in [(542,186),(543,186),(542,187),(544,187),(542,188),(543,189),(545,189)]:assigned[native]='native-bark-fragment-542-186'
    samples=[];sample_owner=[]
    for trace_id in owner:
        if not isinstance(trace_id,int) or trace_id>=len(source_traces):continue
        for point in source_traces[trace_id]:samples.append(point[:2]);sample_owner.append(trace_id)
    sample_tree=cKDTree(samples)
    for iteration in range(args.repairs):
        misses=np.argwhere(core&~hit)
        if not len(misses):break
        changes={};routed=[]
        for yy,xx in misses:
            native=(int(xx)+331,int(yy)+11);target=np.array(native)+.5
            trace_id=assigned.get(native)
            if trace_id is None:
                _,index=sample_tree.query(target);trace_id=sample_owner[index]
            section_index=owner[trace_id];section=sections[section_index];rings=section['vertices'].reshape(-1,16,3);centers=rings.mean(1);ring=int(np.argmin(np.linalg.norm(project(centers)-target,axis=1)));changes.setdefault(section_index,set()).add(ring);routed.append(dict(native=native,trace_id=trace_id,ring=ring))
        accepted=[];rejected=[]
        for index,targets in changes.items():
            section=sections[index];rings=section['vertices'].reshape(-1,16,3);centers=rings.mean(1);radial=rings-centers[:,None,:];radii=np.linalg.norm(radial[:,0],axis=1)
            for width in [6,12,24,48]:
                amount=np.max([.6*np.exp(-((np.arange(len(rings))-j)/width)**2) for j in targets],axis=0);candidate=centers[:,None,:]+radial*(1+amount/radii)[:,None,None];audit=strip_orientation(candidate.reshape(-1,3),16)
                if not audit['nonoutward']:
                    section['vertices']=candidate.reshape(-1,3);hit_cache[index]=coverage([section]);accepted.append(dict(trace_id=section['trace_id'],width=width,max_radius_increment=.6));break
            else:rejected.append(section['trace_id'])
        repairs.append(dict(iteration=iteration,misses_before=len(misses),routing=routed,accepted=accepted,rejected=rejected));hit=np.logical_or.reduce(hit_cache);print('Source repair',iteration,'remaining',int((core&~hit).sum()),'rejected',rejected,flush=True)
        if not accepted:break
    checks=[dict(trace_id=section['trace_id'],**strip_orientation(section['vertices'],16)) for section in sections];bad=sum(check['nonoutward'] for check in checks)

    payload={};index=[]
    for i,section in enumerate(sections):
        payload[f'vertices_{i}']=section['vertices'];payload[f'faces_{i}']=section['faces'];index.append({k:v for k,v in dict(section,index=i).items() if k not in ['vertices','faces']})
    np.savez_compressed(out/'mesh.npz',**payload)
    result=dict(status='CPU TRIAL HOLD; no model/render authorization implied',mesh_sha256=hashlib.sha256((out/'mesh.npz').read_bytes()).hexdigest(),parent_mesh_sha256=old['mesh_sha256'],mesh_sections=index,source_repairs=repairs,chains=records,sections=checks,nonoutward=bad,curvature_radius_reduced_sections=reduced,remaining_core_misses=int((core&~hit).sum()),miss_native_pixels=[[int(x)+331,int(y)+11] for y,x in np.argwhere(core&~hit)],lost_prior_core_pixels=[[int(x)+331,int(y)+11] for y,x in np.argwhere(core&before&~hit)],removed_internal_cap_pairs=sum(len(c['sections'])-1 for c in chains),limitations=['True forks remain overlapping closed sections; local explicit junction construction is still required.','Two disconnected skeleton traces and one directly traced native bark fragment remain independent; hidden anatomical connections are not invented.','No saved model or ground receiver proof.'])
    (out/'report.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({k:v for k,v in result.items() if k not in ['chains','sections','mesh_sections','miss_native_pixels','lost_prior_core_pixels','source_repairs']},indent=2));print('new misses',result['lost_prior_core_pixels'])

if __name__=='__main__':main()
