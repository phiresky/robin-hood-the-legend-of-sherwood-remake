"""Separate east gate roof candidate fitted to the reviewed native silhouette."""
import math
import bpy
import bmesh
from mathutils import Vector

TAG = 'south-gate-east-mask63-candidate-v2'
KNOTS = (0,.2,.4,.6,.8,.95,1)
RADII = (55.1897116,36.6361644,24.7766668,13.9145433,6.4319810,1.250515,1.250516)


def radius(t):
    """Shape-preserving cubic interpolation of circular section radii."""
    h=[b-a for a,b in zip(KNOTS,KNOTS[1:])]
    delta=[(b-a)/step for a,b,step in zip(RADII,RADII[1:],h)]
    slopes=[0.0]*len(KNOTS)
    for i in range(1,len(KNOTS)-1):
        if delta[i-1]*delta[i]>0:
            w1=2*h[i]+h[i-1];w2=h[i]+2*h[i-1]
            slopes[i]=(w1+w2)/(w1/delta[i-1]+w2/delta[i])
    for index,d0,d1,h0,h1 in ((0,delta[0],delta[1],h[0],h[1]),
                               (-1,delta[-1],delta[-2],h[-1],h[-2])):
        value=((2*h0+h1)*d0-h0*d1)/(h0+h1)
        if value*d0<=0:value=0
        elif d0*d1<0 and abs(value)>3*abs(d0):value=3*d0
        slopes[index]=value
    i=next((i for i in range(len(h)) if KNOTS[i]<=t<=KNOTS[i+1]),len(h)-1)
    u=(t-KNOTS[i])/h[i]
    return ((2*u**3-3*u*u+1)*RADII[i]+(u**3-2*u*u+u)*h[i]*slopes[i]
            +(-2*u**3+3*u*u)*RADII[i+1]+(u**3-u*u)*h[i]*slopes[i+1])


def refine():
    collection=bpy.data.collections['Derby Working']
    sources=[o for o in collection.objects if o.type=='MESH' and not o.hide_render
             and o.get('source_node') in [f'building-{i:03}' for i in range(13,17)]]
    if len(sources)!=4:raise ValueError('Expected four east tower roof sectors')
    # The reviewed silhouette and first-hit texel test now protect ownership.
    # The older .25 grazing cutoff unnecessarily discarded visible roof flanks.
    for o in sources:o['projection_min_cosine']=.05
    if all(o.get('round3_south_gate')==TAG for o in sources):return {'tag':TAG,'reused':True}
    if any(o.get('round2_south_gate')!='south-gate-round2-radial-v1' for o in sources):
        raise ValueError('Candidate must start from the approved round roof')
    # Bitmap indices denote pixel areas; fit the physical centers, not their corners.
    cx,cy=943.4,-4176.6447744
    base,top=260.4055083,379.5194654
    s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
    # A flared circular eave and a rounded metal finial are independently visible.
    # Each section stays circular; no bitmap contour is extruded into the mesh.
    profile=[(base-3,RADII[0]-2),(base-1,RADII[0]),(base,RADII[0])]
    profile += [(base+(top-base)*t,radius(t)) for t in [i/48 for i in range(1,46)]]
    ball_z=(-cy*s-2084)/c
    profile += [(ball_z-4,.8),(ball_z-3,2.8),(ball_z,4.15),
                (ball_z+3,2.8),(ball_z+4,.55),((-cy*s-2069)/c,.4),
                ((-cy*s-2067.5)/c,0)]
    if any(b[0]<=a[0] for a,b in zip(profile,profile[1:])):raise ValueError('Profile must ascend')
    # Stable angular ownership prevents sector boundaries drifting on replay.
    owned_angles={'building-013':list(range(65,88)),
                  'building-014':list(range(42,65)),
                  'building-015':list(range(17,42)),
                  'building-016':list(range(17))+list(range(88,96))}
    allocations={o:owned_angles[o['source_node']]for o in sources};count=96
    records=[]
    for o in sources:
        verts=[];faces=[]
        for j in allocations[o]:
            start=len(verts)
            for z,r in profile[:-1]:
                for a in (2*math.pi*j/count,2*math.pi*(j+1)/count):
                    verts.append(Vector((cx+r*math.cos(a),cy+r*math.sin(a),z)))
            n=len(profile)-1;apex=len(verts);verts.append(Vector((cx,cy,profile[-1][0])))
            center=len(verts);verts.append(Vector((cx,cy,profile[0][0])))
            faces += [(start+2*k,start+2*k+1,start+2*k+3,start+2*k+2)for k in range(n-1)]
            faces += [(start+2*n-2,start+2*n-1,apex),
                      tuple([center]+[start+2*k for k in range(n)]+[apex]),
                      tuple([center,apex]+[start+2*k+1 for k in reversed(range(n))]),
                      (center,start+1,start)]
        mesh=bpy.data.meshes.new(o.name+' / reviewed mask63 candidate')
        inverse=o.matrix_world.inverted();mesh.from_pydata([inverse@p for p in verts],[],faces)
        bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
        bad=sum(not e.is_manifold for e in bm.edges);flat=sum(f.calc_area()<1e-8 for f in bm.faces)
        bm.to_mesh(mesh);bm.free()
        if bad or flat:raise ValueError(f'Invalid candidate {o.name}: {bad}/{flat}')
        material=bpy.data.materials.get('Round two south gate neutral')
        if not material:raise ValueError('Expected approved worker neutral material')
        mesh.materials.append(material)
        uv=mesh.uv_layers.new(name='Source projection')
        for loop in mesh.loops:
            p=o.matrix_world@mesh.vertices[loop.vertex_index].co
            uv.data[loop.index].uv=(p.x/1920,1-(-p.y*s-p.z*c)/2752)
        o.data=mesh;o['round3_south_gate']=TAG
        records.append({'node':o['source_node'],'faces':len(faces),'nonmanifold':bad,'degenerate':flat})
    bpy.context.view_layer.update()
    return {'tag':TAG,'status':'candidate requires renewed approval','changed':records,
            'center':[cx,cy],'base':base,'curve_top':top,'profile':profile,
            'mask_index':63,'mask_layer':0,'mask_layer_index':48,'state':'covered exterior'}
