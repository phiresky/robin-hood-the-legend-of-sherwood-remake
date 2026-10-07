"""Test a source-constrained chain entry between the two timber support planes."""
import ast, hashlib, json, math, sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
WORK=ROOT/'level-editor/work/york-refinement/restart2'
BASE=WORK/'winch-components-source-v2'
OUT=WORK/'winch-guided-entry-study-v3'
if OUT.exists(): raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy, numpy as np
from mathutils import Vector, Matrix
from mathutils.bvhtree import BVHTree
s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
up=Vector((0,0,1));screen_side=Vector((1,0,0));spacing=3.5/c
recipe=Path(__file__).with_name('restart13_winch_return_study.py')
exec(compile(ast.Module(body=[n for n in ast.parse(recipe.read_text()).body if isinstance(n,ast.FunctionDef)],type_ignores=[]),str(recipe),'exec'))
center=world(2410,1064,104);axis=(center-world(2402,1050,104)).normalized();radial=Vector((-axis.y,axis.x,0))

def guided_route(radius=9.4, anchor_x=2397, visible_x=2399.5, entry_y=943, count=72):
    a=station(anchor_x+radius*radial.x);b=station(2410-radius*radial.x)
    left=a-radius*radial;right=b+radius*radial
    upper_game_y=1062.94
    guide_height=(upper_game_y-entry_y)/c-left.z
    assert guide_height>0
    delta=Vector((visible_x-anchor_x,-upper_game_y/s-left.y,0))
    def smooth(u): return u*u*(3-2*u)
    def points(height):
        result=[]
        for t in np.linspace(0,height,513,endpoint=False):
            result.append(left+up*t+delta*smooth(min(t/guide_height,1)))
        for u in np.linspace(0,1,513,endpoint=False):
            result.append(a+(b-a)*smooth(u)+radius*(-math.cos(math.pi*u)*radial+math.sin(math.pi*u)*up)+up*height+delta*(1-smooth(u)))
        for t in np.linspace(height,0,513,endpoint=False): result.append(right+up*t)
        for u in np.linspace(0,1,513):
            result.append(b+(a-b)*smooth(u)+radius*(math.cos(math.pi*u)*radial-math.sin(math.pi*u)*up))
        return np.asarray(result)
    length=count*spacing;lo,hi=30.,160.
    for _ in range(24):
        mid=(lo+hi)/2;p=points(mid);total=float(np.linalg.norm(np.diff(p,axis=0),axis=1).sum())
        if total<length:lo=mid
        else:hi=mid
    height=(lo+hi)/2;p=points(height);dist=np.r_[0,np.cumsum(np.linalg.norm(np.diff(p,axis=0),axis=1))]
    def pose(t,index):
        t%=dist[-1];point=Vector(tuple(float(np.interp(t,dist,p[:,j])) for j in range(3)))
        k=max(1,min(int(np.searchsorted(dist,t)),len(p)-1));tangent=Vector(p[k]-p[k-1]).normalized()
        side=(screen_side-tangent*tangent.dot(screen_side)).normalized();normal=side.cross(tangent).normalized()
        if index%2:side=normal;normal=side.cross(tangent).normalized()
        return point,Matrix((side,tangent,normal)).transposed()
    return pose,{'radius':radius,'lower_left_anchor':list(left),'right_anchor':list(right),'upper_left_native_x':visible_x,'guide_start_native_y':entry_y,'upper_left_game_y':upper_game_y,'guide_height_world':guide_height,'straight_height':height,'path_length':float(dist[-1]),'count':count},p

def tree(objects):
    vertices=[];faces=[];names=[]
    for o in objects:
        off=len(vertices);vertices.extend(o.matrix_world@v.co for v in o.data.vertices);o.data.calc_loop_triangles()
        for f in o.data.loop_triangles:faces.append(tuple(off+i for i in f.vertices));names.append(o.name)
    return BVHTree.FromPolygons(vertices,faces,all_triangles=True),np.asarray(vertices)[np.asarray(faces)],names

def crosses(a,b):
    e1=b[:,1]-b[:,0];e2=b[:,2]-b[:,0];hit=np.zeros(len(a),dtype=bool)
    for i in range(3):
        origin=a[:,i];direction=a[:,(i+1)%3]-origin;p=np.cross(direction,e2);det=np.einsum('ij,ij->i',e1,p);valid=abs(det)>1e-8;inv=np.zeros_like(det);inv[valid]=1/det[valid];t=origin-b[:,0];u=np.einsum('ij,ij->i',t,p)*inv;q=np.cross(t,e1);v=np.einsum('ij,ij->i',direction,q)*inv;d=np.einsum('ij,ij->i',e2,q)*inv;hit|=valid&(u>=-1e-6)&(v>=-1e-6)&(u+v<=1+1e-6)&(d>1e-6)&(d<1-1e-6)
    return hit

bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;scene.frame_set(88);bpy.context.view_layer.update();scope=set(json.loads((BASE/'component-freeze.json').read_text())['scope']);body,bt,names=tree([o for o in scene.objects if o.name in scope])
pose,params,path=guided_route();rx=1.5;wire=.35;perimeter=2*math.pi*rx+4*(3-rx);local=[];faces=[]
for j in range(32):
    p,n=capsule(j*perimeter/32,rx)
    for k in range(8):phi=k*math.tau/8;local.append(p+n*(wire*math.cos(phi))+Vector((0,0,wire*math.sin(phi))))
for j in range(32):
    for k in range(8):
        a=j*8+k;b=((j+1)%32)*8+k;cc=((j+1)%32)*8+(k+1)%8;d=j*8+(k+1)%8;faces.extend(((a,b,cc),(a,cc,d)))
local=np.asarray(local);facearray=np.asarray(faces);profile=np.asarray([capsule(j*perimeter/128,rx)[0] for j in range(128)]);sampling_bound=max(np.linalg.norm(np.roll(profile,-1,axis=0)-profile,axis=1))
rows=[]
for phase in np.arange(0,7,.5):
    vv=[];ff=[];centerlines=[]
    for i in range(params['count']):
        p,r=pose(i*spacing+float(phase)/c,i);matrix=np.asarray(r);off=len(vv);vv.extend(local@matrix.T+np.asarray(p));ff.extend(facearray+off);centerlines.append(profile@matrix.T+np.asarray(p))
    vv=np.asarray(vv);ff=np.asarray(ff);chain=BVHTree.FromPolygons(vv,ff,all_triangles=True);pairs=chain.overlap(body);counts={}
    if pairs:
        ai,bi=np.asarray(pairs).T;ct=vv[ff[ai]];mask=crosses(ct,bt[bi])|crosses(bt[bi],ct)
        for name in np.asarray(names)[bi[mask]]:counts[str(name)]=counts.get(str(name),0)+1
    clearance=min(float(np.linalg.norm(a[:,None,:]-b[None,:,:],axis=2).min())-sampling_bound-2*wire for a,b in zip(centerlines,centerlines[1:]+centerlines[:1]))
    rows.append({'phase_native_pixels':float(phase),'exact_body_crossing_pairs':counts,'minimum_adjacent_wire_clearance_bound':float(clearance)})
OUT.mkdir();(OUT/'study.json').write_text(json.dumps({'status':'Private coupled entry diagnostic; no geometry saved or approved','source_model_sha256':hashlib.sha256((BASE/'model.blend').read_bytes()).hexdigest(),'constraint':'Upper native X retained through Y943; only inferred lower guide shifts toward drum station; wood and drum remain exact.','path':params,'profile':{'rx':rx,'ry':3,'wire_radius':wire},'rows':rows},indent=2)+'\n');(OUT/'path.json').write_text(json.dumps(path.tolist())+'\n');print(json.dumps({'path':params,'rows':rows},indent=2))
