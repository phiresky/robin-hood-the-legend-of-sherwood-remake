"""Conservative continuous99phase clearance for fixed-size butterfly anatomy."""
import collections,json,math,itertools
import numpy as np
from PIL import Image
from scipy.interpolate import CubicSpline
from scipy.spatial import ConvexHull
import restart14_butterfly_canopy22_audit as reader
from restart14_butterfly07_anatomy_contacts import clip_planes
from restart14_butterfly07_prism_contacts import alpha_maximum
from restart14_butterfly07_pose_fit import WING,BODY
B=reader.B;OUT=B/'butterfly07-full99-envelope-v2'

def main(source_only=False):
    global OUT
    if source_only:OUT=B/'butterfly07-full99-source-depth-v1'
    assert not OUT.exists();curvep=B/'butterfly07-depth-curve-v1/curve.json';definition=json.loads(curvep.read_text())['curve'];curve=CubicSpline(definition['times'],definition['heights'],bc_type='periodic');plan=next(s for s in json.loads((B/'all7-context-plan-v1/plan.json').read_text())['sequences']if s['index']==14);screen=np.array([r['alpha_centroid_display']for r in plan['path']]);corners=np.array(list(itertools.product([-1.,1.],repeat=3)));radius=math.ceil(max(np.linalg.norm(BODY,axis=1).max(),np.linalg.norm(WING,axis=1).max()+.22)+math.sqrt(8));units=[];ray_sites=set();footprints=[]
    def anchor(t):
        i=int(t)%99;f=t-int(t);xy=(1-f)*screen[i]+f*screen[(i+1)%99];z=float(curve(t));return np.array([xy[0],-(xy[1]+reader.COS*z)/reader.SIN,z])
    for phase in range(99):
        frame=plan['path'][phase];assert reader.sha(__import__('pathlib').Path(frame['source']))==frame['sha256'];a=np.asarray(Image.open(frame['source']).convert('RGBA'));yy,xx=np.nonzero(a[:,:,3]);pixels=[(float(x+frame['bbox'][0]+.5),float(y+frame['bbox'][1]+.5))for x,y in zip(xx,yy)];ray_sites.update(pixels);footprints.append(pixels)
        if source_only:continue
        for step in range(8):
            ta=phase+step/8;tb=phase+(step+1)/8;points=np.vstack([anchor(ta)+corners*radius,anchor(tb)+corners*radius]);pad=max(abs(float(curve(ta,2))),abs(float(curve(tb,2))))/(reader.SIN*8*64)+1e-7;planes=ConvexHull(points,qhull_options='QJ').equations;planes[:,3]-=pad
            units.append({'phase':phase,'step':step,'low':points.min(0)-pad,'high':points.max(0)+pad,'planes':planes,'contacts':collections.Counter(),'witnesses':[]})
    def inspect(placed,node,ni,pi,tri,uvs,mat,alpha,texture_record):
        world=np.stack([tri[:,:,0],-tri[:,:,2],tri[:,:,1]],axis=2);uv=np.zeros((*tri.shape[:2],2))if uvs is None else uvs;attrs=np.concatenate([world,uv],axis=2);low=world.min(1);high=world.max(1);opaque=mat.get('alphaMode','OPAQUE')=='OPAQUE';image,sampler,factor=(None,{},1.)if opaque else texture_record(mat);cutoff=mat.get('alphaCutoff',.5)if mat.get('alphaMode')=='MASK'else .01
        for unit in units:
            if unit['contacts'].get(placed['id']):continue
            ids=np.flatnonzero(np.all(high>=unit['low'],axis=1)&np.all(low<=unit['high'],axis=1))
            for idx in ids:
                poly=clip_planes(attrs[idx],unit['planes'])
                if not len(poly):continue
                value,witness=0.,None
                for sample in [*poly,poly.mean(0)]:
                    observed=1. if opaque else alpha(mat,sample[3:5],True)
                    if observed>=cutoff:value,witness=observed,sample;break
                if witness is None:value,witness,_=alpha_maximum(poly,image,sampler,factor)
                if value<cutoff:continue
                unit['contacts'][placed['id']]+=1
                if len(unit['witnesses'])<1:unit['witnesses'].append({'asset':placed['id'],'node':node.get('name'),'part_material':mat.get('name'),'world_zup':witness[:3].tolist(),'alpha':value})
                break
    def finish(rays,assets,map_path):
        rows=[];lookup={tuple(r['screen']):r for r in rays}
        for phase in range(99):
            owner=collections.Counter();source_owner=collections.Counter();windows=[];examples=[]
            for unit in units[phase*8:phase*8+8]:
                owner.update(unit['contacts'])
                if unit['contacts']:windows.append(unit['step']);examples+=unit['witnesses'][:1]
            for pixel in footprints[phase]:
                for hit in lookup[pixel]['hits']:
                    if hit['passes_alpha_only'] and abs(hit['world_yup'][1]-float(curve(phase)))<=8:source_owner[hit['asset']]+=1
            rows.append({'phase':phase,'height':float(curve(phase)),'source_pixels':len(footprints[phase]),'source_pixel_center_plusminus8Z_potential_contacts':dict(source_owner),'continuous_fixed_anatomy_envelope_contacts':None if source_only else dict(owner),'subintervals_with_possible_contact':windows,'witnesses':examples[:2]})
        report={'status':'FULL99_SOURCE_PIXEL_CENTER_AUDIT_NO_CONTINUOUS_PROOF' if source_only else 'FULL99_CONSERVATIVE_ASSESSMENT_NOT_EXACT_ANATOMY_APPROVAL','curve_sha256':reader.sha(curvep),'map_sha256':reader.sha(map_path),'assets':assets,'world_radius':radius,'bound':'Cube enclosing all rigid orientations of fixed body/wing anatomy plus2px independent X/Y registration. Endpoint-hull slabs expanded by exact cubic depth second-derivative chord error;8subintervals each of99phases. No temporal pose assumptions needed within this size bound.','rows':rows,'clear_continuous_phase_intervals':None if source_only else [r['phase']for r in rows if not r['continuous_fixed_anatomy_envelope_contacts']],'limits':['Envelope fills empty space around wings; intersections are conservative possible contacts, not actual anatomy collisions.','Only8anatomical poses fitted;91remain unbuilt. Missing observed source pixels/filaments are outside completeness claims.','Source pixel checks sample centers with±8Zextent, not full source-pixel squares.','Native frame timing/artwork/order unchanged; inferred body depth is a private candidate.','Static map geometry only; state assets not incorporated.']};OUT.mkdir();payload=json.dumps(report,indent=2)+'\n';assert len(payload)<4*1024**2;(OUT/'report.json').write_text(payload);print('CLEAR_CONTINUOUS',None if source_only else len(report['clear_continuous_phase_intervals']),flush=True);return report
    reader.main(ray_records=[{'screen':list(p),'hits':[]}for p in sorted(ray_sites)],postprocess=finish,output=OUT,triangle_callback=None if source_only else inspect,query_margin=0. if source_only else 30.,output_limit_bytes=4*1024**2)

if __name__=='__main__':main()
