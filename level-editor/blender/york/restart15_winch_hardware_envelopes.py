"""Test inferred guide/hanger/collar envelopes without writing a Blender model."""
import ast,hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';BASE=WORK/'restart2/winch-guided-entry-candidate-v1';OUT=WORK/'restart2/winch-hardware-envelopes-v6.json'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy,numpy as np
from mathutils import Vector,Matrix
from mathutils.bvhtree import BVHTree
from PIL import Image
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));up=Vector((0,0,1));screen_side=Vector((1,0,0));back=Vector((0,-c,s));spacing=3.5/c
for name in ('restart13_winch_return_study.py','restart14_winch_guided_entry.py'):
    p=Path(__file__).with_name(name);exec(compile(ast.Module(body=[n for n in ast.parse(p.read_text()).body if isinstance(n,ast.FunctionDef)],type_ignores=[]),str(p),'exec'))
center=world(2410,1064,104);axis=(center-world(2402,1050,104)).normalized();radial=Vector((-axis.y,axis.x,0));pose,params,path=guided_route(count=76)
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;scene.frame_set(88);bpy.context.view_layer.update();scope=set(json.loads((WORK/'restart2/winch-components-source-v2/component-freeze.json').read_text())['scope']);body=[o for o in scene.objects if o.name in scope];context=[o for o in scene.objects if o.type=='MESH' and o.get('native_patch')!='patch-004' and not o.hide_render];room,roomtri,roomnames=tree(context);links=sorted((o for o in scene.objects if o.name.startswith('Guided chain link')),key=lambda o:o.name)
while len(links)<params['count']:
    o=links[0].copy();o.name=f'Guided chain envelope link {len(links):03d}';scene.collection.objects.link(o);links.append(o)

def tube(points,radius=.35,sides=8):
    points=np.asarray(points,dtype=float);vv=[];ff=[]
    for j,p in enumerate(points):
        tangent=points[min(j+1,len(points)-1)]-points[max(j-1,0)];tangent/=np.linalg.norm(tangent);a=np.cross(tangent,[0,0,1])
        if np.linalg.norm(a)<1e-5:a=np.cross(tangent,[1,0,0])
        a/=np.linalg.norm(a);b=np.cross(tangent,a)
        for k in range(sides):angle=k*math.tau/sides;vv.append(p+radius*(a*math.cos(angle)+b*math.sin(angle)))
    for j in range(len(points)-1):
        for k in range(sides):a=j*sides+k;b=j*sides+(k+1)%sides;cc=(j+1)*sides+(k+1)%sides;d=(j+1)*sides+k;ff.extend(((a,b,cc),(a,cc,d)))
    for k in range(1,sides-1):ff.append((0,k+1,k));off=(len(points)-1)*sides;ff.append((off,off+k,off+k+1))
    return np.asarray(vv),np.asarray(ff)

def curvature_offset(indices,offset=2.4):
    result=[]
    for i in indices:
        before=path[i]-path[i-1];after=path[i+1]-path[i];before/=np.linalg.norm(before);after/=np.linalg.norm(after);normal=after-before;normal/=np.linalg.norm(normal);result.append(path[i]+normal*offset)
    return result

# Analytic normals avoid amplifying float32 coordinate noise in second differences.
def curve_guide(us,upper=False,offset=2.4):
    radius=params['radius'];a=np.asarray(station(2397+radius*radial.x),dtype=float);b=np.asarray(station(2410-radius*radial.x),dtype=float);left=np.asarray(params['lower_left_anchor'],dtype=float);delta=np.array([2399.5-left[0],-1062.94/s-left[1],0]);result=[]
    for u in us:
        smooth=u*u*(3-2*u)
        if upper:
            point=a+(b-a)*smooth+radius*(-math.cos(math.pi*u)*np.asarray(radial)+math.sin(math.pi*u)*np.asarray(up))+np.asarray(up)*params['straight_height']+delta*(1-smooth)
            tangent=(b-a-delta)*6*u*(1-u)+radius*math.pi*(math.sin(math.pi*u)*np.asarray(radial)+math.cos(math.pi*u)*np.asarray(up))
            second=(b-a-delta)*6*(1-2*u)+radius*math.pi**2*(math.cos(math.pi*u)*np.asarray(radial)-math.sin(math.pi*u)*np.asarray(up))
        else:
            height=params['guide_height_world'];point=left+np.array([0,0,height*u])+delta*smooth;tangent=np.array([0,0,height])+delta*6*u*(1-u);second=delta*6*(1-2*u)
        tangent/=np.linalg.norm(tangent);normal=second-tangent*np.dot(tangent,second);normal/=np.linalg.norm(normal);result.append(point+normal*offset)
    return result
upper=curve_guide(np.linspace(.016,.984,64),True,3.2);lower_a=curve_guide(np.linspace(.08,.43,20));lower_b=curve_guide(np.linspace(.57,.72,20));parts={'upper guide shoe':tube(upper),'lower guide shoe A':tube(lower_a),'lower guide shoe B':tube(lower_b)};anchors=[]
frame_tree,frame_tri,frame_names=tree([o for o in body if o.name.startswith(('Angled','Frame'))]);frame_anchors=[]
for label,points in [('A',lower_a),('B',lower_b)]:
    start=np.asarray(points[len(points)//2]);mount_tree,mount_tri,mount_names=tree([o for o in body if o.name.startswith(('Angled','Frame')) and (label=='A' or not o.name.endswith('.001'))]);point,normal,index,distance=mount_tree.find_nearest(Vector(start));end=np.asarray(point,dtype=float);normal=np.asarray(normal,dtype=float);direction=end-start;direction/=np.linalg.norm(direction);vv,ff=tube([start,end],.22)
    assert abs(np.dot(normal,direction))>.1
    for vertex in vv[-8:]:vertex+=direction*((1e-4-np.dot(vertex-end,normal))/np.dot(direction,normal))
    parts[f'lower guide bracket {label}']=(vv,ff);frame_anchors.append({'guide':label,'owner':mount_names[index],'point_world':list(point),'normal':normal.tolist(),'length_world':float(distance),'contact_tolerance_world':1e-4})
upper_normal=np.cross([0,0,1],path[1026]-path[513]);upper_normal/=np.linalg.norm(upper_normal)
for index,t in enumerate((.25,.75)):
    p=np.asarray(upper[int((len(upper)-1)*t)]);elbow=p-upper_normal*4;hit=room.ray_cast(Vector(elbow),Vector((0,0,1)))
    if hit[0] is None:raise ValueError('No ceiling for inferred hanger')
    endpoint=np.asarray(hit[0]);vv,ff=tube([p,elbow,endpoint],.35)
    for vertex in vv[-8:]:
        sample=room.ray_cast(Vector((vertex[0],vertex[1],elbow[2])),Vector((0,0,1)))
        if sample[0] is None:raise ValueError('Missing roof cap contact')
        vertex[2]=sample[0].z-1e-4
    parts[f'upper hanger {index}']=(vv,ff);anchors.append({'hanger':index,'owner':roomnames[hit[2]],'point_world':endpoint.tolist(),'normal':list(hit[1]),'contact':'Endpoint lies exactly on measured room surface; no penetration added.'})

def collar(center_z):
    center_z+=2
    center=np.array([2399.5,-1062.94/s,center_z]);vv=[];ff=[];major=2.4;minor=.25
    for j in range(32):
        theta=j*math.tau/32;r=np.array([math.cos(theta),math.sin(theta),0.]);
        for k in range(8):phi=k*math.tau/8;vv.append(center+r*(major+minor*math.cos(phi))+np.array([0,0,minor*math.sin(phi)]))
    for j in range(32):
        for k in range(8):a=j*8+k;b=((j+1)%32)*8+k;cc=((j+1)%32)*8+(k+1)%8;d=j*8+(k+1)%8;ff.extend(((a,b,cc),(a,cc,d)))
    disc=scene.objects['Travelling round part solid drum'];front_y=min((disc.matrix_world@v.co).y for v in disc.data.vertices)-1e-4;hub=np.array([2398.,front_y,center_z]);direction=hub-center;direction[2]=0;direction/=np.linalg.norm(direction);tip=center+direction*(major+minor);armv,armf=tube([hub,tip],.3);armv[:8,1]=hub[1];return {'traveller sliding collar':(np.asarray(vv),np.asarray(ff)),'traveller collar arm':(armv,armf)}

def mesh_tree(parts):
    vertices=[];faces=[];names=[]
    for name,(vv,ff) in parts.items():
        offset=len(vertices);vertices.extend(vv);faces.extend(ff+offset);names.extend([name]*len(ff))
    vertices=np.asarray(vertices);faces=np.asarray(faces);return BVHTree.FromPolygons(vertices,faces,all_triangles=True),vertices[faces],names

def intersections(a,at,an,b,bt,bn):
    pairs=a.overlap(b);counts={}
    if pairs:
        ai,bi=np.asarray(pairs).T;mask=crosses(at[ai],bt[bi])|crosses(bt[bi],at[ai])
        for aa,bb in zip(ai[mask],bi[mask]):key=f'{an[aa]} / {bn[bb]}';counts[key]=counts.get(key,0)+1
    return counts

chains=[];link_clearances=[];rx=1.5;perimeter=2*math.pi*rx+4*(3-rx);profile=np.asarray([capsule(j*perimeter/256,rx)[0] for j in range(256)]);sampling_bound=max(np.linalg.norm(np.roll(profile,-1,axis=0)-profile,axis=1))
for phase in np.arange(0,7,.125):
    for i,o in enumerate(links):p,r=pose(i*spacing+float(phase)/c,i);o.location=p;o.rotation_euler=r.to_euler()
    bpy.context.view_layer.update();chains.append((float(phase),*tree(links)));centerlines=[]
    for i in range(params['count']):
        p,r=pose(i*spacing+float(phase)/c,i);centerlines.append(profile@np.asarray(r).T+np.asarray(p))
    clearance=min(float(np.linalg.norm(a[:,None,:]-b[None,:,:],axis=2).min())-float(sampling_bound)-.7 for a,b in zip(centerlines,centerlines[1:]+centerlines[:1]));link_clearances.append({'phase':float(phase),'minimum_adjacent_wire_clearance_bound':clearance})
static,st,staticnames=mesh_tree(parts);static_chain=[]
for phase,chain,ct,cn in chains:
    counts=intersections(static,st,staticnames,chain,ct,cn)
    if counts:static_chain.append({'phase':phase,'pairs':counts})
body_contacts=[];collar_chain=[];chain_body=[];chain_room=[]
for phase,chain,ct,cn in chains:
    counts=intersections(chain,ct,cn,room,roomtri,roomnames)
    if counts:chain_room.append({'phase':phase,'pairs':counts})
for frame in range(45):
    scene.frame_set(frame*2);bpy.context.view_layer.update();bodytree,bt,bn=tree(body);z=scene.objects['Travelling round part solid drum'].matrix_world.translation.z;col=collar(z);coltree,cot,con=mesh_tree(col);counts=intersections(static,st,staticnames,bodytree,bt,bn);cc=intersections(coltree,cot,con,bodytree,bt,bn)
    mutual=intersections(static,st,staticnames,coltree,cot,con)
    if counts or cc or mutual:body_contacts.append({'frame':frame,'static_hardware_pairs':counts,'collar_pairs':cc,'moving_hardware_pairs':mutual})
    for phase,chain,ct,cn in chains:
        counts=intersections(chain,ct,cn,bodytree,bt,bn)
        if counts:chain_body.append({'frame':frame,'phase':phase,'pairs':counts})
        counts=intersections(coltree,cot,con,chain,ct,cn)
        if counts:collar_chain.append({'frame':frame,'phase':phase,'pairs':counts})
source=WORK/'geometry-pass-01/native-state-source-v1';record=next(r for r in json.loads((source/'manifest.json').read_text())['records'] if r['id']=='patch-004');frames=next(r['frames'] for r in record['rows'] if r['action']=='PatchTransition');exposure=[]
for frame in (0,22,30,36,44):
    scene.frame_set(frame*2);bpy.context.view_layer.update();z=scene.objects['Travelling round part solid drum'].matrix_world.translation.z;hardware,ht,hn=mesh_tree({**parts,**collar(z)});fixed,ft,fn=tree([o for o in scene.objects if o.type=='MESH' and not o.hide_render]);f=frames[frame];alpha=Image.open(source/f['image']).getchannel('A');hits={}
    for y in range(875,979):
        for x in range(2385,2435):
            origin=Vector((x+.5,-(y+.5)/s,0))+back*10000;new=hardware.ray_cast(origin,-back)
            if new[0] is None:continue
            old=fixed.ray_cast(origin,-back)
            if old[0] is not None and old[3]<new[3]-1e-5:continue
            name=hn[new[2]];record=hits.setdefault(name,{'visible_pixels':0,'outside_source_alpha_pixels':0,'covered_frozen_body_pixels':0,'examples':[]});record['visible_pixels']+=1;xx,yy=x-f['bbox'][0],y-f['bbox'][1];opaque=0<=xx<alpha.width and 0<=yy<alpha.height and alpha.getpixel((xx,yy))>=128;record['outside_source_alpha_pixels']+=not opaque;record['covered_frozen_body_pixels']+=old[2] is not None and fn[old[2]] in scope
            if len(record['examples'])<10:record['examples'].append({'pixel':[x,y],'old_owner':fn[old[2]] if old[2] is not None else None,'source_opaque':opaque})
    exposure.append({'frame':frame,'parts':hits})
result={'status':'Private inferred envelope trial; no model saved or approval','base_model_sha256':hashlib.sha256((BASE/'model.blend').read_bytes()).hexdigest(),'chain_path':params,'chain_note':'Four extra hidden links raise the upper return; existing 24body components unchanged. This route is only an in-memory CPU proposal.','ceiling_anchors':anchors,'frame_anchors':frame_anchors,'static_chain_crossings':static_chain,'static_room_crossings':intersections(static,st,staticnames,room,roomtri,roomnames),'body_contacts':body_contacts,'collar_chain_crossings':collar_chain,'chain_body_crossings':chain_body,'chain_room_crossings':chain_room,'adjacent_link_clearances':link_clearances,'chain_phase_samples':len(chains),'body_pose_samples':45,'native_exposure':exposure,'inference':'Stationary guide shoes with offset ceiling hangers; sliding collar follows the measured round-part pose, not a tracked chain link. Collar braking/drive is not observed.','limitations':['Finite samples and exact triangle-edge intersections do not prove continuous clearance or detect full containment.','Collar-arm endpoint contact at the existing round-part face is intentional; any other overlap needs review.','Source comparison uses one fixed chain phase, and per-pixel first hits; cannot certify subpixel source preservation.']};OUT.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'static_chain_failing_phases':len(static_chain),'chain_body_failing_pairs':len(chain_body),'chain_room_failing_phases':len(chain_room),'minimum_adjacent_wire_clearance_bound':min(r['minimum_adjacent_wire_clearance_bound'] for r in link_clearances),'body_failing_poses':len(body_contacts),'collar_chain_failing_pairs':len(collar_chain),'native_exposure':exposure,'ceiling_anchors':anchors,'frame_anchors':frame_anchors},indent=2))
