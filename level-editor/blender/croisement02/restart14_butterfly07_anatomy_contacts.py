"""CPU receiver contacts for fixed butterfly07 poses and bounded rigid motion."""
import collections,json,math,importlib.util
from pathlib import Path
import numpy as np
from scipy.spatial import ConvexHull
from scipy.spatial.transform import Rotation,Slerp
import restart14_butterfly_canopy22_audit as reader
from restart14_butterfly07_prism_contacts import alpha_maximum
from restart14_butterfly07_pose_fit import geometry,WING

from scipy.interpolate import CubicSpline

B=reader.B;OUT=B/'butterfly07-anatomy-contacts-v2';EPS=1e-8

def clip_planes(poly,planes):
    for plane in planes:
        if not len(poly):break
        result=[];previous=poly[-1];old=float(previous[:3]@plane[:3]+plane[3])
        for current in poly:
            value=float(current[:3]@plane[:3]+plane[3])
            if (old<=EPS)!=(value<=EPS):
                result.append(previous+(EPS-old)/(value-old)*(current-previous))
            if value<=EPS:result.append(current)
            previous,old=current,value
        poly=np.asarray(result).reshape(-1,5)
    return poly

def triangle_planes(v):
    normal=np.cross(v[1]-v[0],v[2]-v[0]);normal/=np.linalg.norm(normal)
    planes=[np.r_[normal,-normal@v[0]],np.r_[-normal,normal@v[0]]]
    for a,b,c in [(v[0],v[1],v[2]),(v[1],v[2],v[0]),(v[2],v[0],v[1])]:
        n=np.cross(b-a,normal);n/=np.linalg.norm(n)
        if n@(c-a)>0:n=-n
        planes.append(np.r_[n,-n@a])
    return np.array(planes)

def validate_rest_shape(fit, expected=None):
    expected=expected or dict(wing_outline=WING.tolist(),body_radii=[.4,2.7,.45],wing_hinge_offsets=[-.22,.22])
    for key in ('wing_outline','body_radii','wing_hinge_offsets'):
        assert np.array_equal(np.asarray(fit['fixed_geometry'][key]),np.asarray(expected[key])), f'Fit {key} does not match the audited geometry helper'


def geometry_binding(fit, helper_path=None, helper_sha256=None):
    if helper_path is None:
        assert helper_sha256 is None
        helper=Path(__file__).with_name('restart14_butterfly07_pose_fit.py')
        expected=dict(wing_outline=WING.tolist(),body_radii=[.4,2.7,.45],wing_hinge_offsets=[-.22,.22])
        function=geometry
    else:
        helper=Path(helper_path)
        assert helper_sha256 and reader.sha(helper)==helper_sha256, 'Geometry helper hash mismatch'
        spec=importlib.util.spec_from_file_location('bound_butterfly_geometry',helper)
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
        expected=module.fixed_geometry();function=module.geometry
    if fit.get('geometry_helper_sha256') is not None:
        assert fit['geometry_helper_sha256']==reader.sha(helper), 'Fit was authored with a different geometry helper'
    validate_rest_shape(fit,expected)
    radii=dict(body=float(max(expected['body_radii'])),wing=float(np.linalg.norm(expected['wing_outline'],axis=1).max()+max(abs(v) for v in expected['wing_hinge_offsets'])))
    return function,radii,dict(path=str(helper),sha256=reader.sha(helper),fixed_geometry=expected)


def curve_acceleration_bound(curve, start, end):
    # A cubic spline has linear second derivative within each knot interval.
    times=np.r_[start,curve.x[(curve.x>start)&(curve.x<end)],end]
    return float(np.abs(curve(times,2)).max())


def main(fit_path=None, midpoint_hinge_offsets=None, transition_candidates=None, registration_controls=None, depth_trials=None, continuous_height_curve=None, sweep_subdivisions=8, dense_pose_steps=None, geometry_helper=None, geometry_helper_sha256=None, receiver_asset_ids=('croisement02-tree-01','croisement02-tree-02')):
    assert not OUT.exists(),'Preserve prior evidence'
    fitp=Path(fit_path) if fit_path else B/'butterfly07-pose-fit-v1/fit.json';fit=json.loads(fitp.read_text());rows={r['phase']:r for r in fit['rows']};bound_geometry,radii,binding=geometry_binding(fit,geometry_helper,geometry_helper_sha256)
    curve=None if continuous_height_curve is None else CubicSpline(continuous_height_curve['times'],continuous_height_curve['heights'],bc_type='periodic')
    phase_intervals=[phase for phase in sorted(rows) if (phase+1)%99 in rows]
    units=[]
    def vertices(row,mirror=False,t=None,other=None,hinge_offset=None,registration_offset=None,depth_delta=0.):
        p=np.array(row['parameters']);anchor=np.array(row['fixed_path_anchor_zup'])
        if t is not None:
            q=np.array(other['parameters']);rotation=Slerp([0,1],Rotation.from_euler('xyz',[p[:3],q[:3]],degrees=True))([t])[0]
            p[:3]=rotation.as_euler('xyz',degrees=True);p[3:]=(1-t)*p[3:]+t*q[3:];anchor=(1-t)*anchor+t*np.array(other['fixed_path_anchor_zup'])
        if curve is not None:depth_delta+=float(curve(row['phase']+(0 if t is None else t)))-anchor[2]
        anchor+=np.array([0.,-reader.COS/reader.SIN*depth_delta,depth_delta])
        if registration_offset is not None:p[5:7]+=registration_offset
        if hinge_offset is not None:p[3:5]+=hinge_offset
        if mirror:p[[0,1,3,4]]*=-1
        body,wings=bound_geometry(p)
        def world(v):
            x=v[:,0]+p[5];y=v[:,1]+p[6];d=v[:,2]
            return anchor+np.c_[x,-reader.SIN*y-reader.COS*d,-reader.COS*y+reader.SIN*d]
        return [world(body),*[world(w) for w in wings]]
    def parts(v):
        yield 'body',v[0],True
        for k,w in enumerate(v[1:]):
            for i in range(1,len(w)-1):yield f'wing{k}',w[[0,i,i+1]],False
    def add(key,label,v,planes,pad=0.):
        units.append(dict(key=key,part=label,low=v.min(0)-pad-EPS,high=v.max(0)+pad+EPS,planes=planes,contacts=collections.Counter(),witnesses=[]))
    for mirrored in (False,True):
        if (transition_candidates is not None or registration_controls is not None or depth_trials is not None) and mirrored:continue
        branch='mirrored' if mirrored else 'selected'
        for phase,row in rows.items():
            for name,v,solid in parts(vertices(row,mirrored)):
                add(f'{branch}:pose:{phase}',name,v,ConvexHull(v).equations if solid else triangle_planes(v))
        for label,depths in (depth_trials or {}).items():
            for phase,row in rows.items():
                for name,v,solid in parts(vertices(row,depth_delta=depths[phase])):
                    add(f'depth-trial:{label}:pose:{phase}',name,v,ConvexHull(v).equations if solid else triangle_planes(v))
        for phase in phase_intervals:
            a,b=rows[phase],rows[(phase+1)%99]
            if dense_pose_steps:
                for sample in range(1,dense_pose_steps):
                    t=sample/dense_pose_steps
                    for name,v,solid in parts(vertices(a,mirrored,t,b)):
                        add(f'{branch}:dense-pose:{phase}:{t}',name,v,ConvexHull(v).equations if solid else triangle_planes(v))
            for name,v,solid in parts(vertices(a,mirrored,.5,b)):
                add(f'{branch}:midpoint:{phase}-{phase+1}',name,v,ConvexHull(v).equations if solid else triangle_planes(v))
            for offset in (midpoint_hinge_offsets or {}).get(phase,[]):
                for name,v,solid in parts(vertices(a,mirrored,.5,b,offset)):
                    add(f'{branch}:inferred-hinge-midpoint:{phase}-{phase+1}:{offset}',name,v,ConvexHull(v).equations if solid else triangle_planes(v))
            for ia,ib,aa,bb in (transition_candidates or {}).get(phase,[]):
                for t in [.25,.5,.75]:
                    for name,v,solid in parts(vertices(aa,False,t,bb)):
                        add(f'selected:edge:{phase}:{ia}-{ib}:{t}',name,v,ConvexHull(v).equations if solid else triangle_planes(v))
            for offset in (registration_controls or {}).get(phase,[]):
                for t in [.25,.5,.75]:
                    bow=np.array(offset)*math.sin(math.pi*t)
                    for name,v,solid in parts(vertices(a,False,t,b,registration_offset=bow)):
                        add(f'selected:registration-bow:{phase}:{offset}:{t}',name,v,ConvexHull(v).equations if solid else triangle_planes(v))
            for label,depths in (depth_trials or {}).items():
                for t in [.25,.5,.75]:
                    dz=(1-t)*depths[phase]+t*depths[(phase+1)%99]
                    for name,v,solid in parts(vertices(a,False,t,b,depth_delta=dz)):
                        add(f'depth-trial:{label}:edge:{phase}:{t}',name,v,ConvexHull(v).equations if solid else triangle_planes(v))
            ra=Rotation.from_euler('xyz',a['parameters'][:3],degrees=True);rb=Rotation.from_euler('xyz',b['parameters'][:3],degrees=True)
            angle=(ra.inv()*rb).magnitude();hinges=np.abs(np.deg2rad(np.array(a['parameters'][3:5])-b['parameters'][3:5]));n=sweep_subdivisions
            for step in range(n):
                start=list(parts(vertices(a,mirrored,step/n,b)));end=list(parts(vertices(a,mirrored,(step+1)/n,b)))
                for (name,v,solid),(_,w,_) in zip(start,end):
                    radius=radii['body'] if solid else radii['wing']
                    angular=angle if solid else angle+hinges[int(name[-1])]
                    # Linear chord error <= max||v''|| dt²/8; translations are linear.
                    pad=float(radius*angular**2/(8*n*n))+1e-7
                    if curve is not None:pad+=curve_acceleration_bound(curve,phase+step/n,phase+(step+1)/n)/(reader.SIN*8*n*n)
                    joined=np.vstack([v,w]);hull=ConvexHull(joined,qhull_options='QJ');planes=hull.equations.copy();planes[:,3]-=pad
                    assert np.max(joined@planes[:,:3].T+planes[:,3])<=EPS
                    add(f'{branch}:sweep:{phase}-{phase+1}',name,joined,planes,pad)
    queries=[{'screen':r['source']['alpha_centroid_display'],'hits':[]} for r in rows.values()]
    def inspect(placed,node,ni,pi,tri,uvs,mat,alpha,texture_record):
        world=np.stack([tri[:,:,0],-tri[:,:,2],tri[:,:,1]],axis=2)
        uv=np.zeros((*tri.shape[:2],2)) if uvs is None else uvs
        attrs=np.concatenate([world,uv],axis=2);low=world.min(1);high=world.max(1)
        opaque=mat.get('alphaMode','OPAQUE')=='OPAQUE';image,sampler,factor=(None,{},1.) if opaque else texture_record(mat)
        cutoff=mat.get('alphaCutoff',.5) if mat.get('alphaMode')=='MASK' else .01
        for unit in units:
            indexes=np.flatnonzero(np.all(high>=unit['low'],axis=1)&np.all(low<=unit['high'],axis=1))
            for idx in indexes:
                poly=clip_planes(attrs[idx],unit['planes'])
                if not len(poly):continue
                value,witness,_=alpha_maximum(poly,image,sampler,factor)
                if value<cutoff:continue
                assert opaque or abs(value-alpha(mat,witness[3:5],True))<1e-6
                unit['contacts'][placed['id']]+=1
                if len(unit['witnesses'])<1:unit['witnesses'].append({'receiver':placed['id'],'node':node.get('name'),'triangle':int(idx),'primitive':pi,'material':mat.get('name'),'world_zup':witness[:3].tolist(),'alpha':value,'uv':witness[3:5].tolist()})
    def finish(rays,assets,map_path):
        results={}
        for u in units:
            row=results.setdefault(u['key'],{'contact_counts':{},'parts':{},'witnesses':[]})
            for owner,count in u['contacts'].items():row['contact_counts'][owner]=row['contact_counts'].get(owner,0)+count;row['parts'][u['part']]=row['parts'].get(u['part'],0)+count
            if len(row['witnesses'])<5:row['witnesses']+=u['witnesses']
        report={'status':'PRIVATE_ANATOMY_AND_CONSERVATIVE_SWEEP_DIAGNOSTIC','fit_sha256':reader.sha(fitp),'map_sha256':reader.sha(map_path),'recipe_sha256':reader.sha(Path(__file__)),'assets':assets,'results':results,'method':'Exact transformed receiver triangles clipped to closed triangulated ellipsoid body or zero-thickness wing fan triangles; bilinear level0 alpha maximized on intersections. Both depth branches tested only without a candidate trial pool; pools test the selected branch. Recorded bounded rotational subintervals per adjacent pose; slerped body rotation and linear hinges, anchors and registration. Each endpoint convex hull expanded by rigorous second-derivative chord-error bound. Swept contacts are conservative, not proof of an actual intermediate intersection.','limits':[f"{sum(row['missing'] for row in fit['rows'])} own-source pixels remain outside fitted anatomy. This tests inferred geometry, not final butterfly approval.",f'Tested {len(rows)} poses / {len(phase_intervals)} adjacent intervals; full 99-phase cycle requires 99 poses and 99 intervals.','Wing fans are inferred solid surfaces; UV/material pattern unbuilt.','Opposite-depth poses project identically; source cannot select them.','Numerical closed-contact tolerance1e-8 world units; level0 alpha only.','No path lifting, canopy edit, render or library write.']}
        report.update(geometry_binding=binding,receiver_asset_ids=None if receiver_asset_ids is None else sorted(receiver_asset_ids),tested_pose_count=len(rows),tested_phase_intervals=phase_intervals,sweep_subdivisions=sweep_subdivisions,dense_pose_steps=dense_pose_steps,continuous_height_curve=continuous_height_curve,depth_trials=depth_trials,midpoint_hinge_offsets=midpoint_hinge_offsets,registration_controls=registration_controls,transition_candidate_counts={k:len(v)for k,v in (transition_candidates or {}).items()})
        report['limits'].append('Control and graph samples are inferred between observed frames; sampled clearance is not swept clearance.')
        OUT.mkdir();payload=json.dumps(report,indent=2)+'\n';assert len(payload)<2*1024**2;(OUT/'report.json').write_text(payload)
        print(json.dumps({k:v['contact_counts'] for k,v in results.items()}),flush=True)
        return report
    reader.main(ray_records=queries,postprocess=finish,output=OUT,asset_ids=receiver_asset_ids,triangle_callback=inspect,query_margin=max(15.,radii['wing']+max(np.linalg.norm(row['parameters'][5:7]) for row in rows.values())),output_limit_bytes=2*1024**2)

if __name__=='__main__':main()
