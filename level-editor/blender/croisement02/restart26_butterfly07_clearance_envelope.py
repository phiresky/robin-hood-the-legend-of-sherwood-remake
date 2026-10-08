"""Conservative continuous free-depth bands for the exact fixed butterfly rig."""
import collections,json,math
from pathlib import Path
import numpy as np
from scipy.spatial.transform import Rotation,Slerp
import restart14_butterfly_canopy22_audit as reader
import restart26_butterfly07_depth_refinement as pinned
from restart14_butterfly07_anatomy_contacts import geometry_binding
from restart14_butterfly07_prism_contacts import box_clip,alpha_maximum
OUT=reader.B/'butterfly07-v3-continuous-depth-envelope-v1'
SUBDIVISIONS=8
MARGIN_Z=.25

def forbidden_height(receiver_depth_range,body_depth_range):
    return (reader.SIN*(receiver_depth_range[0]-body_depth_range[1])-MARGIN_Z,
            reader.SIN*(receiver_depth_range[1]-body_depth_range[0])+MARGIN_Z)

def main():
    assert not OUT.exists();assert reader.sha(pinned.FIT)==pinned.FIT_SHA
    fit=json.loads(pinned.FIT.read_text());rows={r['phase']:r for r in fit['rows']};assert set(rows)==set(range(99))
    geometry,radii,binding=geometry_binding(fit,pinned.HELPER,pinned.HELPER_SHA)
    units=[]
    for phase in range(99):
        a,b=rows[phase],rows[(phase+1)%99];p=np.array(a['parameters']);q=np.array(b['parameters'])
        rotations=Rotation.from_euler('xyz',[p[:3],q[:3]],degrees=True);slerp=Slerp([0,1],rotations)
        angle=(rotations[0].inv()*rotations[1]).magnitude();hinges=np.abs(np.deg2rad(q[3:5]-p[3:5]))
        pad=max(radii['body']*angle**2,radii['wing']*(angle+max(hinges))**2)/(8*SUBDIVISIONS**2)+1e-6
        def at(t):
            parameters=(1-t)*p+t*q;parameters[:3]=slerp([t])[0].as_euler('xyz',degrees=True)
            source=(1-t)*np.array(a['source']['alpha_centroid_display'])+t*np.array(b['source']['alpha_centroid_display'])
            body,wings=geometry(parameters);vertices=np.vstack([body,*wings])
            # h=0 anchor. Arbitrary physical body height translates camera depth by h/SIN.
            return vertices+np.array([source[0]+parameters[5],source[1]+parameters[6],reader.COS*source[1]/reader.SIN])
        for step in range(SUBDIVISIONS):
            points=np.vstack([at(step/SUBDIVISIONS),at((step+1)/SUBDIVISIONS)])
            lo=points.min(0)-pad;hi=points.max(0)+pad
            units.append(dict(start=phase+step/SUBDIVISIONS,end=phase+(step+1)/SUBDIVISIONS,low=lo,high=hi,angular_pad=pad,bands={},counts=collections.Counter()))
    lows=np.array([u['low']for u in units]);highs=np.array([u['high']for u in units]);query=np.array([r['source']['alpha_centroid_display']for r in rows.values()])
    margin=max(float(np.maximum(abs(query-lo[:2]),abs(query-hi[:2])).max(axis=1).min())for lo,hi in zip(lows,highs))+1e-6
    def inspect(placed,node,ni,pi,tri,uvs,mat,alpha,texture_record):
        camera=np.stack([tri[:,:,0],reader.SIN*tri[:,:,2]-reader.COS*tri[:,:,1],reader.COS*tri[:,:,2]+reader.SIN*tri[:,:,1]],axis=2)
        uv=np.zeros((*tri.shape[:2],2))if uvs is None else uvs;attrs=np.concatenate([camera,uv],axis=2)
        lo=camera.min(1);hi=camera.max(1);eligible=np.flatnonzero(np.all(highs[:,:2]>=lo[:,:2].min(0),axis=1)&np.all(lows[:,:2]<=hi[:,:2].max(0),axis=1))
        opaque=mat.get('alphaMode','OPAQUE')=='OPAQUE';image,sampler,factor=(None,{},1.)if opaque else texture_record(mat)
        cutoff=mat.get('alphaCutoff',.5)if mat.get('alphaMode')=='MASK'else .01
        for index in eligible:
            unit=units[index];near=np.flatnonzero(np.all(hi[:,:2]>=unit['low'][:2],axis=1)&np.all(lo[:,:2]<=unit['high'][:2],axis=1))
            for ti in near:
                poly=box_clip(attrs[ti],[(axis,unit['low'][axis],unit['high'][axis])for axis in (0,1)])
                if not len(poly):continue
                if not opaque:
                    probes=np.vstack([poly,poly.mean(0)])
                    passing=any(alpha(mat,point[3:5],True)>=cutoff for point in probes)
                    if not passing:
                        maximum,_,_=alpha_maximum(poly,image,sampler,factor)
                        if maximum<cutoff:unit['counts']['alpha_rejected']+=1;continue
                low,high=forbidden_height((poly[:,2].min(),poly[:,2].max()),(unit['low'][2],unit['high'][2]))
                prior=unit['bands'].get(placed['id'])
                unit['bands'][placed['id']]=[float(low),float(high)]if prior is None else[min(prior[0],float(low)),max(prior[1],float(high))]
                unit['counts']['included_triangle_pieces']+=1
    def finish(rays,assets,map_path):
        records=[]
        for unit in units:
            bands=sorted(unit['bands'].values());merged=[]
            for low,high in bands:
                if merged and low<=merged[-1][1]:merged[-1][1]=max(high,merged[-1][1])
                else:merged.append([low,high])
            records.append(dict(start=unit['start'],end=unit['end'],screen_bounds=[unit['low'][:2].tolist(),unit['high'][:2].tolist()],zero_height_camera_depth_bounds=[float(unit['low'][2]),float(unit['high'][2])],angular_pad=unit['angular_pad'],forbidden_height_bands=merged,receiver_bands=unit['bands'],counts=unit['counts']))
        result=dict(status='CONSERVATIVE_CONTINUOUS_STATIC_CLEARANCE_ENVELOPE',fit_sha256=pinned.FIT_SHA,geometry_binding=binding,map_sha256=reader.sha(map_path),assets=assets,recipe_sha256=reader.sha(Path(__file__)),subdivisions=SUBDIVISIONS,height_margin=MARGIN_Z,records=records,method='Rigid body/wing endpoint coordinate boxes expanded by rotational second-derivative error bound, independent of body height. Receiver triangles clipped to their continuous source rectangles; exact level0 alpha rejects wholly transparent pieces. Full depth span of each surviving clipped polygon remains conservative, including transparent subregions. Per-receiver bands enclose any gaps. Bands hold for every time in each segment; a height curve wholly outside them has conservative physical clearance.',limits=['Conservative boxes and per-receiver depth spans can exclude genuinely free gaps; no claim of minimum physical displacement.','Native source anchors/timing and fixed global geometry remain unchanged.','Static receivers only; source fit344missing/528extra/70bright remains held.','No native FX ordering or visibility is used to demand an above-canopy path.'])
        OUT.mkdir();payload=json.dumps(result,indent=2)+'\n';assert len(payload)<8*1024**2;(OUT/'report.json').write_text(payload)
        print('ENVELOPE',len(records),len(assets),flush=True);return result
    reader.main(ray_records=[dict(screen=p.tolist(),hits=[])for p in query],postprocess=finish,output=OUT,triangle_callback=inspect,query_margin=margin,output_limit_bytes=8*1024**2)

if __name__=='__main__':main()
