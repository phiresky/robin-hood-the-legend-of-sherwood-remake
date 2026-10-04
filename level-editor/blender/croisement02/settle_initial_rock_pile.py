"""Infer a supported initial rock stack without changing native image projections."""
import json, sys
from pathlib import Path
import bpy
import numpy as np
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from catalog import OUT
from tree_geometry import RAY, SIN, COS
from log_trap_state_candidate import sha,point
from render_slots import acquire,release


def points(obj):
    return np.array([obj.matrix_world@v.co for v in obj.data.vertices],dtype=float)


def axes(obj):
    transform=obj.matrix_world.to_3x3()
    normals=np.array([transform@p.normal for p in obj.data.polygons],dtype=float)
    vertices=points(obj)
    edges=np.array([vertices[e.vertices[1]]-vertices[e.vertices[0]] for e in obj.data.edges])
    edges/=np.linalg.norm(edges,axis=1)[:,None]
    return normals,np.unique(np.round(edges,7),axis=0)


def collision_interval(a,b):
    pa,pb=points(a),points(b)
    na,ea=axes(a);nb,eb=axes(b)
    cross=np.cross(ea[:,None,:],eb[None,:,:]).reshape(-1,3)
    ns=np.concatenate([na,nb,cross]);length=np.linalg.norm(ns,axis=1);ns=ns[length>1e-8]/length[length>1e-8,None]
    ns=np.unique(np.round(ns,7),axis=0)
    lo,hi=-float('inf'),float('inf');normal=None
    ray=np.array(RAY)
    for start in range(0,len(ns),512):
        n=ns[start:start+512];ap=pa@n.T;bp=pb@n.T;amin,amax=ap.min(axis=0),ap.max(axis=0);bmin,bmax=bp.min(axis=0),bp.max(axis=0);dot=n@ray
        parallel=np.abs(dot)<1e-7
        if np.any(parallel&((amax<bmin-1e-6)|(bmax<amin-1e-6))):return None
        n=n[~parallel];dot=dot[~parallel];amin=amin[~parallel];amax=amax[~parallel];bmin=bmin[~parallel];bmax=bmax[~parallel]
        t0=(amin-bmax)/dot;t1=(amax-bmin)/dot
        lower=np.minimum(t0,t1);upper=np.maximum(t0,t1)
        if len(lower):
            lo=max(lo,float(lower.max()));index=int(upper.argmin())
            if upper[index]<hi:hi=float(upper[index]);normal=(n[index]*np.sign(dot[index])).tolist()
        if lo>hi:return None
    return dict(first=lo,last=hi,separating_normal=normal,axes=len(ns))


def main():
    base=OUT/'rock-trap-state-candidate-v11';dest=OUT/'rock-trap-state-candidate-v12';dest.mkdir(exist_ok=True);assert not(dest/'worker.blend').exists()
    report=json.loads((base/'manifest.json').read_text());assert sha(base/'worker.blend')==report['model_sha256'];acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'));bpy.context.view_layer.update();scene=bpy.context.scene
        rocks=sorted([o for o in scene.objects if o.get('state_endpoint')=='covered'],key=lambda o:o.name)
        # Two bank-supported bodies carry the upper rock; this depth order is inferred.
        assert len(rocks)==3
        rocks=[rocks[0],rocks[2],rocks[1]]
        old={o.name:points(o) for o in scene.objects if o.type=='MESH'};placed=[];records=[]
        for obj in rocks:
            intervals=[dict(support=a.name,**value)for a in placed if (value:=collision_interval(a,obj)) is not None]
            shift=0.;support=None;normal=None
            for _ in range(len(intervals)+1):
                overlapping=[r for r in intervals if r['first']-1e-6<=shift<=r['last']+1e-6]
                if not overlapping:break
                chosen=max(overlapping,key=lambda r:r['last']);shift=chosen['last']+.005;support=chosen['support'];normal=chosen['separating_normal']
            if shift:
                assert normal[2]>.1,('No upward supporting contact',obj.name,normal)
                obj.location+=RAY*shift;bpy.context.view_layer.update()
            now=points(obj);prior=old[obj.name]
            projected_error=max(float(np.abs(now[:,0]-prior[:,0]).max()),float(np.abs((now[:,1]-prior[:,1])*SIN+(now[:,2]-prior[:,2])*COS).max()))
            assert projected_error<1e-4
            for a in placed:
                interval=collision_interval(a,obj)
                assert interval is None or not(interval['first']<-.001 and interval['last']>.001),(a.name,obj.name,interval)
            records.append(dict(object=obj.name,ray_translation=shift,supported_by=support or 'native bank surface',support_normal=normal,source_projection_max_error=projected_error,minimum_world_z=float(now[:,2].min()),inference='Camera-ray depth is inferred; source pixel positions are unchanged.'))
            placed.append(obj)
        for obj in scene.objects:
            if obj.type=='MESH'and obj not in rocks:assert np.array_equal(old[obj.name],points(obj)),obj.name
        bpy.ops.wm.save_as_mainfile(filepath=str(dest/'worker.blend'))
        result=dict(report);result.update(status='private inferred initial stack; visual and contact review pending',base_model_sha256=report['model_sha256'],model_sha256=sha(dest/'worker.blend'),initial_stack=records,stack_limitations=['No convex volume intersections remain between covered boulders.','A body lifted off terrain has an upward rock-contact normal; rigid-body stability and native moving-body identities are not established.','Applied endpoint geometry, bank receivers, native source UVs and source image pixels remain unchanged.'])
        (dest/'manifest.json').write_text(json.dumps(result,indent=2)+'\n')
        source=json.loads((OUT/'state-target-evidence/rock-trap/manifest.json').read_text());left,top,right,bottom=source['bbox'];target=point((left+right)/2,(top+bottom)/2,0)
        for view,direction in [('source',RAY),('oblique',Vector((-1,-1,.8)).normalized())]:
            scene.camera.location=target+direction*3000;scene.camera.rotation_euler=(target-scene.camera.location).to_track_quat('-Z','Y').to_euler();scene.render.filepath=str(dest/f'covered-{view}-actual-bank1.png');bpy.ops.render.render(write_still=True)
        print(json.dumps(records,indent=2),flush=True)
    finally:release()


if __name__=='__main__':main()
