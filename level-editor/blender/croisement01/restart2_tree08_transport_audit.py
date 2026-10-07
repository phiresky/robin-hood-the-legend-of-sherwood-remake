"""CPU-only sweep trial with independent projected pixel-center coverage."""
import argparse,hashlib,json,sys
from pathlib import Path
import numpy as np
from PIL import Image
from restart2_tree08_hierarchy import build_hierarchy_sections
from restart2_tree08_transport import curvature_limited_sweep,strip_orientation
ROOT=Path(__file__).resolve().parents[3]
R=ROOT/'level-editor/work/croisement01-refinement/restart2'


def coverage(sections):
    hit=np.zeros((461,446),dtype=bool)
    s,c=np.sin(np.radians(35)),np.cos(np.radians(35))
    for section in sections:
        v=np.asarray(section['vertices']);xy=np.column_stack((v[:,0]-331,-v[:,1]*s-v[:,2]*c-11))
        for face in section['faces']:
            for i in range(1,len(face)-1):
                a,b,d=xy[[face[0],face[i],face[i+1]]]
                lo=np.maximum(0,np.ceil(np.minimum(np.minimum(a,b),d)-.5).astype(int));hi=np.minimum([445,460],np.floor(np.maximum(np.maximum(a,b),d)-.5).astype(int))
                if np.any(lo>hi):continue
                denominator=(b[0]-a[0])*(d[1]-a[1])-(b[1]-a[1])*(d[0]-a[0])
                if abs(denominator)<1e-10:continue
                yy,xx=np.mgrid[lo[1]:hi[1]+1,lo[0]:hi[0]+1];x=xx+.5-a[0];y=yy+.5-a[1]
                u=(x*(d[1]-a[1])-y*(d[0]-a[0]))/denominator;w=((b[0]-a[0])*y-(b[1]-a[1])*x)/denominator
                hit[yy,xx]|=(u>=-1e-8)&(w>=-1e-8)&(u+w<=1+1e-8)
    return hit


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--revision',type=int,default=5);parser.add_argument('--repair-iterations',type=int,default=8);args=parser.parse_args()
    out=R/f'tree08-v11-curvature-cpu-v{args.revision}';out.mkdir(exist_ok=False)
    traces=json.loads((R/'tree08-source-trace-v2/trace.json').read_text())['polylines']
    selected=[p['trace_id'] for p in json.loads((R/'tree08-wood-prototype-v8/construction.json').read_text())['sections'] if isinstance(p['trace_id'],int)]
    root=json.loads((R/'tree08-topology-plan-v1/plan.json').read_text())['root_native']
    misses=json.loads((R/'tree08-wood-prototype-v8/coverage.json').read_text())['miss_native_pixels']
    core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0
    support=[[int(x)+331,int(y)+11] for y,x in np.argwhere(core)]
    sections,_,_=build_hierarchy_sections(traces,selected,root,misses,support,True,True,True)
    old_hit=coverage(sections);records=[]
    for section in sections:
        section['vertices'],record=curvature_limited_sweep(section['vertices'])
        records.append(dict(trace_id=section['trace_id'],**record))
    hit=coverage(sections)
    source_repairs=[]
    for iteration in range(args.repair_iterations):
        lost=np.argwhere(core&old_hit&~hit)
        if not len(lost):break
        projected=[];owners=[]
        for section_index,section in enumerate(sections):
            v=np.asarray(section['vertices']);sine,cosine=np.sin(np.radians(35)),np.cos(np.radians(35))
            projected.extend(np.column_stack((v[:,0]-331,-v[:,1]*sine-v[:,2]*cosine-11)))
            owners.extend((section_index,j//16) for j in range(len(v)))
        from scipy.spatial import cKDTree
        distance,nearest=cKDTree(projected).query(lost[:,::-1]+.5)
        adjustments={}
        for d,index in zip(distance,nearest):
            if d>2.5:continue
            section_index,ring=owners[index];adjustments.setdefault(section_index,set()).add(ring)
        accepted=0
        for section_index,targets in adjustments.items():
            section=sections[section_index];rings=np.asarray(section['vertices']).reshape(-1,16,3);center=rings.mean(1);radial=rings-center[:,None,:];radius=np.linalg.norm(radial[:,0],axis=1)
            amount=np.max([.3*np.exp(-((np.arange(len(rings))-j)/5)**2) for j in targets],axis=0)
            candidate=center[:,None,:]+radial*(1+amount/radius)[:,None,None]
            check=strip_orientation(candidate.reshape(-1,3),16)
            if check['nonoutward']:continue
            section['vertices']=candidate.reshape(-1,3);records[section_index].update(check);accepted+=1
        source_repairs.append(dict(iteration=iteration,lost_pixels=len(lost),accepted_sections=accepted,max_increment=.3))
        if not accepted:break
        hit=coverage(sections)
    report=dict(status='HOLD: CPU construction trial, not saved-model validation',sections=records,
        source_repairs=source_repairs,nonoutward=sum(r['nonoutward'] for r in records),tested=sum(r['tested'] for r in records),
        core_pixels=int(core.sum()),baseline_core_misses=int((core&~old_hit).sum()),candidate_core_misses=int((core&~hit).sum()),
        lost_core_pixels=int((core&old_hit&~hit).sum()),gained_core_pixels=int((core&~old_hit&hit).sum()),
        source_silhouette_removed_pixels=int((old_hit&~hit).sum()),source_silhouette_added_pixels=int((~old_hit&hit).sum()),
        max_source_center_displacement=max(r['max_source_center_displacement'] for r in records),
        duplicate_basal_removed=True,global_remesh=False,
        recipes={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in [Path(__file__),Path(__file__).with_name('restart2_tree08_transport.py'),Path(__file__).with_name('restart2_tree08_hierarchy.py')]},
        limitations=['Coverage compares CPU triangle projections, not saved model or material alpha.','Two bends use oblique parallel sections; actual-material, solid, and contact review remain mandatory.','Junctions remain intersecting separate sections; no welded-joint or terrain proof.'])
    report['lost_native_pixels']=[[int(x)+331,int(y)+11] for y,x in np.argwhere(core&old_hit&~hit)]
    report['miss_native_pixels']=[[int(x)+331,int(y)+11] for y,x in np.argwhere(core&~hit)]
    payload={};section_index=[]
    for index,section in enumerate(sections):
        payload[f'vertices_{index}']=np.asarray(section['vertices'])
        # Triangulate explicitly so Blender imports the same audited diagonal.
        payload[f'faces_{index}']=np.asarray([(face[0],face[j],face[j+1]) for face in section['faces'] for j in range(1,len(face)-1)],dtype=np.int32)
        section_index.append(dict(index=index,trace_id=section['trace_id'],held_crossing=section['held_crossing']))
    np.savez_compressed(out/'mesh.npz',**payload)
    report['mesh_sha256']=hashlib.sha256((out/'mesh.npz').read_bytes()).hexdigest()
    report['mesh_sections']=section_index
    (out/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    rgb=np.zeros((*core.shape,3),dtype=np.uint8);rgb[core]=[50,160,70];rgb[core&~hit]=[255,30,160];rgb[~core&hit]=[70,80,120]
    Image.fromarray(rgb).save(out/'source-coverage.png')
    print(json.dumps({k:v for k,v in report.items() if k not in ['sections','recipes','mesh_sections','miss_native_pixels','lost_native_pixels']},indent=2))

if __name__=='__main__':main()
