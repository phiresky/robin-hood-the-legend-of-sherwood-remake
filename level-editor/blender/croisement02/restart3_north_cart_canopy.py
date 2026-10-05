"""Construct source-surveyed canopy on preserved north-cart running gear."""
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
from restart3_cart_cargo_debris import *
from scenery_geometry import Mesh
import hashlib


def signature(obj):
    return hashlib.sha256(np.asarray([v.co[:] for v in obj.data.vertices],dtype='<f8').tobytes()+json.dumps([list(p.vertices) for p in obj.data.polygons]).encode()+np.asarray([obj.matrix_world[i][:] for i in range(4)],dtype='<f8').tobytes()).hexdigest()


def main():
    root=OUT/'restart3-north-cart';dest=root/'canopy-fit-v1';dest.mkdir(exist_ok=False)
    base=OUT/'north-cart-initial-candidate-v5';old=json.loads((base/'manifest.json').read_text());assert sha(base/'worker.blend')==old['model_sha256']
    fitpath=root/'roof-fit-v1/fit.json';fit=json.loads(fitpath.read_text())
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'));scene=bpy.context.scene
        scene.render.engine='CYCLES';scene.cycles.samples=8;scene.render.film_transparent=True
        objects=[o for o in scene.objects if o.type=='MESH'];preserved={o.name:signature(o) for o in objects if 'Canopy' not in o.name and 'canopy' not in o.name}
        for obj in objects:
            if 'Canopy' in obj.name or 'canopy' in obj.name:bpy.data.objects.remove(obj,do_unlink=True)
        Image.open(base/'cart-owned-source.png').save(dest/'source.png');Image.open(base/'cart-source-domain.png').save(dest/'source-domain.png')
        paint=material(dest/'source.png');gray=next(o for o in scene.objects if o.type=='MESH').data.materials[1]
        box=[1138,114,1350,322];angle,length,width,rise,cx,cy=fit['parameters']
        u=Vector((math.cos(angle),math.sin(angle),0));v=Vector((math.sin(angle),-math.cos(angle),0));Z=Vector((0,0,1))
        oldfront=point(1138+126.6,114+109.1,22);oldv=Vector((.8660254,-.5,0));center=point(1138+cx,114+cy,96)
        eave=96+(4.6-(center-oldfront).dot(oldv))/(.5*COS/SIN);center=point(1138+cx,114+cy,eave)
        group='croisement02-north-cart-initial-physical'
        def build(name,vertices,faces):return mesh(scene,name,vertices,faces,paint,gray,box,group)
        def cube(name,c,du,dv,dz):
            m=Mesh();m.box(c,u,v,du,dv,dz);return build(name,m.vertices,m.faces)
        n=24;stride=n+1
        verts=[]
        for along in [0,length]:
            for thickness in [0,-1.2]:
                for i in range(n+1):
                    a=math.pi*i/n;verts.append(center+u*along+v*(width*math.cos(a))+Z*(rise*math.sin(a)+thickness))
        faces=[]
        for i in range(n):
            for a,b in [(0,2*stride),(stride,3*stride),(0,stride),(2*stride,3*stride)]:faces.append((a+i,a+i+1,b+i+1,b+i))
        for i in [0,n]:faces.append((i,stride+i,3*stride+i,2*stride+i))
        build('Surveyed closed barrel canopy',verts,faces)
        for end in [0,length]:
            verts=[]
            for depth in [-.4,.4]:
                for row in [0,1]:
                    for i in range(n+1):
                        a=math.pi*i/n;z=rise*math.sin(a)-.6 if row==0 else -4
                        verts.append(center+u*(end+depth)+v*(width*math.cos(a))+Z*z)
            faces=[]
            for i in range(n):
                for a,b in [(0,stride),(2*stride,3*stride),(0,2*stride),(stride,3*stride)]:faces.append((a+i,a+i+1,b+i+1,b+i))
            for i in [0,n]:faces.append((i,stride+i,3*stride+i,2*stride+i))
            build(f'Finite arch end valance {end:.2f}',verts,faces)
        for side in [-1,1]:
            across=side*(width-.8)
            cube(f'Canopy eave rail {side}',center+u*length/2+v*across-Z*3,length,2,6)
            for end in [3,length-3]:
                c=center+u*end+v*(side*(width-2));bottom=35.0
                cube(f'Canopy support {side} {end:.2f}',Vector((c.x,c.y,(eave+bottom)/2)),3,3,eave-bottom)
            for first,last in [(0,20),(length-22,length)]:
                k=10;verts=[]
                for depth in [-.35,.35]:
                    for row in [0,1]:
                        for i in range(k+1):
                            f=i/k;along=first+(last-first)*f;z=eave-4 if row==0 else 52+5*math.sin(math.pi*f)
                            verts.append(center+u*along+v*(across+depth+.45*math.sin(f*math.tau*3))+Z*(z-eave))
                st=k+1;faces=[]
                for i in range(k):
                    for a,b in [(0,st),(2*st,3*st),(0,2*st),(st,3*st)]:faces.append((a+i,a+i+1,b+i+1,b+i))
                for i in [0,k]:faces.append((i,st+i,3*st+i,2*st+i))
                build(f'Finite hanging curtain {side} {first:.2f}',verts,faces)
        for name,digest in preserved.items():assert signature(bpy.data.objects[name])==digest
        finish(scene,gray,dest,box,dict(status='Private source-fitted north cart canopy; all geometry unapproved',asset_id=group,
            source_frame=old['source_frame'],source_position=old['source_position'],baseline_model_sha256=old['model_sha256'],
            preserved_running_gear_geometry=preserved,roof_fit=str(fitpath),roof_fit_sha256=sha(fitpath),eave_world_z=eave,
            limitations=['Initial cart pose only; horses, harness, approach, breakup and terminal state excluded.',
                        'Unseen canopy depth and eave height inferred to align with unchanged cart bed; exact support review pending.',
                        'Gray hidden source-unknown surfaces are not texture approved.']))
        write_json(dest/'derived-recipe.json',dict(recipe=record_recipe(dest/'derived-recipe',Path(__file__)),preserved_running_gear_exact=True))
    finally:release()

if __name__=='__main__':main()
