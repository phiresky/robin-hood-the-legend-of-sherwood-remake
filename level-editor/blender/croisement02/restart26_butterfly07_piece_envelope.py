"""Per-piece continuous ray-depth exclusion, preserving gaps between receivers."""
import collections,json
from pathlib import Path
import numpy as np
from scipy.spatial import ConvexHull,QhullError
from scipy.spatial.transform import Rotation,Slerp
import restart14_butterfly_canopy22_audit as reader
import restart26_butterfly07_depth_refinement as pinned
from restart14_butterfly07_anatomy_contacts import geometry_binding,clip_planes
from restart14_butterfly07_prism_contacts import box_clip,alpha_maximum
OUT=reader.B/'butterfly07-v3-piece-depth-envelope-v1'
N=8;MARGIN_Z=.25

def merge_bands(bands):
    merged=[]
    for lo,hi in sorted(bands):
        if merged and lo<=merged[-1][1]+1e-9:merged[-1][1]=max(hi,merged[-1][1])
        else:merged.append([float(lo),float(hi)])
    return merged

def translation_interval(receiver,body,pad):
    # R-P contains exactly those translations for which two convex sets meet.
    difference=(receiver[:,None,:]-body[None,:,:]).reshape(-1,3)
    hull=ConvexHull(difference,qhull_options='QJ');planes=hull.equations.copy();planes[:,3]-=pad
    assert np.max(difference@planes[:,:3].T+planes[:,3])<1e-6
    lo=-np.inf;hi=np.inf
    for plane in planes:
        coefficient=plane[2];constant=plane[3]
        if abs(coefficient)<1e-12:
            if constant>1e-8:return None
        elif coefficient>0:hi=min(hi,-constant/coefficient)
        else:lo=max(lo,-constant/coefficient)
    if lo>hi+1e-8:return None
    assert np.isfinite(lo)and np.isfinite(hi)
    return [float(reader.SIN*lo-MARGIN_Z),float(reader.SIN*hi+MARGIN_Z)]

def main():
    assert not OUT.exists();assert reader.sha(pinned.FIT)==pinned.FIT_SHA
    fit=json.loads(pinned.FIT.read_text());rows={r['phase']:r for r in fit['rows']};assert set(rows)==set(range(99))
    geometry,radii,binding=geometry_binding(fit,pinned.HELPER,pinned.HELPER_SHA);units=[]
    for phase in range(99):
        a,b=rows[phase],rows[(phase+1)%99];p=np.array(a['parameters']);q=np.array(b['parameters']);rotations=Rotation.from_euler('xyz',[p[:3],q[:3]],degrees=True);slerp=Slerp([0,1],rotations)
        angle=(rotations[0].inv()*rotations[1]).magnitude();hinges=np.abs(np.deg2rad(q[3:5]-p[3:5]))
        def at(t):
            parameters=(1-t)*p+t*q;parameters[:3]=slerp([t])[0].as_euler('xyz',degrees=True);source=(1-t)*np.array(a['source']['alpha_centroid_display'])+t*np.array(b['source']['alpha_centroid_display']);shift=np.array([source[0]+parameters[5],source[1]+parameters[6],reader.COS*source[1]/reader.SIN]);body,wings=geometry(parameters)
            result=[('body',body+shift,radii['body']*angle**2)]
            for k,wing in enumerate(wings):
                for i in range(1,len(wing)-1):result.append((f'wing{k}-{i}',wing[[0,i,i+1]]+shift,radii['wing']*(angle+hinges[k])**2))
            return result
        for step in range(N):
            for (name,v,curvature),(_,w,_)in zip(at(step/N),at((step+1)/N)):
                pad=curvature/(8*N*N)+1e-6;points=np.vstack([v,w]);lo=points.min(0)-pad;hi=points.max(0)+pad
                if np.linalg.matrix_rank(points[:,:2]-points[0,:2],tol=1e-10)==2:
                    projection=ConvexHull(points[:,:2]);planes=np.c_[projection.equations[:,:2],np.zeros(len(projection.equations)),projection.equations[:,2]-pad]
                else:planes=None
                units.append(dict(segment=phase*N+step,name=name,vertices=points,low=lo,high=hi,pad=pad,projection_planes=planes,bands={},counts=collections.Counter()))
    lows=np.array([u['low']for u in units]);highs=np.array([u['high']for u in units]);query=np.array([r['source']['alpha_centroid_display']for r in rows.values()]);margin=max(float(np.maximum(abs(query-lo[:2]),abs(query-hi[:2])).max(axis=1).min())for lo,hi in zip(lows,highs))+1e-6
    def inspect(placed,node,ni,pi,tri,uvs,mat,alpha,texture_record):
        camera=np.stack([tri[:,:,0],reader.SIN*tri[:,:,2]-reader.COS*tri[:,:,1],reader.COS*tri[:,:,2]+reader.SIN*tri[:,:,1]],axis=2);uv=np.zeros((*tri.shape[:2],2))if uvs is None else uvs;attrs=np.concatenate([camera,uv],axis=2);lo=camera.min(1);hi=camera.max(1);eligible=np.flatnonzero(np.all(highs[:,:2]>=lo[:,:2].min(0),axis=1)&np.all(lows[:,:2]<=hi[:,:2].max(0),axis=1));opaque=mat.get('alphaMode','OPAQUE')=='OPAQUE';image,sampler,factor=(None,{},1.)if opaque else texture_record(mat);cutoff=mat.get('alphaCutoff',.5)if mat.get('alphaMode')=='MASK'else .01
        for index in eligible:
            unit=units[index];near=np.flatnonzero(np.all(hi[:,:2]>=unit['low'][:2],axis=1)&np.all(lo[:,:2]<=unit['high'][:2],axis=1));bands=unit['bands'].setdefault(placed['id'],[])
            for ti in near:
                poly=box_clip(attrs[ti],[(axis,unit['low'][axis],unit['high'][axis])for axis in (0,1)])
                if unit['projection_planes']is not None:poly=clip_planes(poly,unit['projection_planes'])
                if not len(poly):continue
                if not opaque:
                    passing=any(alpha(mat,point[3:5],True)>=cutoff for point in np.vstack([poly,poly.mean(0)]))
                    if not passing:
                        maximum,_,_=alpha_maximum(poly,image,sampler,factor)
                        if maximum<cutoff:unit['counts']['alpha_rejected']+=1;continue
                band=translation_interval(poly[:,:3],unit['vertices'],unit['pad'])
                if band is not None:bands.append(band);unit['counts']['included_triangle_pieces']+=1
            unit['bands'][placed['id']]=merge_bands(bands)
    def finish(rays,assets,map_path):
        records=[]
        for segment in range(99*N):
            members=[u for u in units if u['segment']==segment];by_owner=collections.defaultdict(list);counts=collections.Counter()
            for unit in members:
                counts.update(unit['counts'])
                for owner,bands in unit['bands'].items():by_owner[owner].extend(bands)
            by_owner={k:merge_bands(v)for k,v in by_owner.items()if v};merged=merge_bands([band for bands in by_owner.values()for band in bands]);records.append(dict(start=segment/N,end=(segment+1)/N,forbidden_height_bands=merged,receiver_bands=by_owner,counts=counts))
        result=dict(status='CONSERVATIVE_CONTINUOUS_PIECE_CLEARANCE_ENVELOPE',fit_sha256=pinned.FIT_SHA,geometry_binding=binding,map_sha256=reader.sha(map_path),assets=assets,recipe_sha256=reader.sha(Path(__file__)),subdivisions=N,height_margin=MARGIN_Z,records=records,method='Body convex hull and individual wing triangles swept by endpoint hulls plus rigorous angular padding. Receiver polygons clipped to each swept projection and alpha-filtered. Minkowski receiver-minus-swept-body intersection with the camera-depth axis gives a forbidden height interval. Per-triangle intervals are unioned without bridging real gaps.',limits=['Exact alpha can reject wholly transparent polygons; partly transparent intervals remain conservative.','Swept hulls may exclude genuine motion-dependent gaps.','Source shape,99anchors and timing retained. Source quality held; static receivers only.'])
        OUT.mkdir();payload=json.dumps(result,indent=2)+'\n';assert len(payload)<8*1024**2;(OUT/'report.json').write_text(payload);print('PIECE_ENVELOPE',len(records),flush=True);return result
    reader.main(ray_records=[dict(screen=p.tolist(),hits=[])for p in query],postprocess=finish,output=OUT,triangle_callback=inspect,query_margin=margin,output_limit_bytes=8*1024**2)

if __name__=='__main__':main()
