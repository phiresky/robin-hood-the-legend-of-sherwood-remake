"""Build bounded terminal cask and detached wood fragments in private workers."""
import json
import math
import sys
from pathlib import Path
import bpy
import bmesh
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image
from scipy.spatial import ConvexHull

HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from tree_geometry import SIN,COS,RAY
from log_trap_state_candidate import point,material
from evidence_io import sha,write_json,record_recipe
from render_slots import acquire,release


def start():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=8
    scene.cycles.use_denoising=False;scene.render.film_transparent=True
    scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard'
    scene.world=bpy.data.worlds.new('World');scene.world.use_nodes=True
    scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.15,.15,.15,1)
    light=bpy.data.lights.new('Sun','SUN');light.energy=2
    obj=bpy.data.objects.new('Sun',light);scene.collection.objects.link(obj);obj.rotation_euler=(.6,-.5,-.4)
    camera=bpy.data.cameras.new('Original-game-first review');camera.type='ORTHO';camera.clip_end=10000
    obj=bpy.data.objects.new('Review camera',camera);scene.collection.objects.link(obj);scene.camera=obj
    gray=bpy.data.materials.new('Unknown cargo surfaces');gray.use_nodes=True
    gray.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.17,.17,.17,1)
    return scene,gray


def mesh(scene,name,vertices,faces,paint,gray,box,group):
    data=bpy.data.meshes.new(name);data.from_pydata(vertices,[],faces);data.update()
    bm=bmesh.new();bm.from_mesh(data);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    bmesh.ops.triangulate(bm,faces=list(bm.faces))
    cuts=min(16,max(0,math.ceil(max(e.calc_length() for e in bm.edges)/2)-1))
    if cuts:bmesh.ops.subdivide_edges(bm,edges=list(bm.edges),cuts=cuts,use_grid_fill=True)
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    assert all(e.is_manifold for e in bm.edges),name
    assert bm.calc_volume(signed=True)>0,name
    bm.to_mesh(data);bm.free();obj=bpy.data.objects.new(name,data);scene.collection.objects.link(obj)
    obj['asset_group']=group;obj['source_state']='south cart terminal; private geometry'
    data.materials.append(paint);data.materials.append(gray)
    uv=data.uv_layers.new(name='Native target projection')
    for face in data.polygons:
        for loop in face.loop_indices:
            p=data.vertices[data.loops[loop].vertex_index].co
            uv.data[loop].uv=((p.x-box[0])/(box[2]-box[0]),1-(-p.y*SIN-p.z*COS-box[1])/(box[3]-box[1]))
    return obj


def finish(scene,gray,dest,box,metadata):
    objects=[o for o in scene.objects if o.type=='MESH'];vertices=[];faces=[]
    for obj in objects:
        start_index=len(vertices);vertices.extend(v.co.copy() for v in obj.data.vertices)
        faces.extend(tuple(start_index+i for i in p.vertices) for p in obj.data.polygons)
    bvh=BVHTree.FromPolygons(vertices,faces)
    for obj in objects:
        for face in obj.data.polygons:
            hit=bvh.ray_cast(face.center+RAY*2000,-RAY,4000)
            face.material_index=0 if face.normal.dot(RAY)>.05 and hit[0] is not None and (hit[0]-face.center).length<.05 else 1
    bpy.context.preferences.filepaths.save_version=0
    bpy.ops.wm.save_as_mainfile(filepath=str(dest/'worker.blend'),compress=True)
    center=Vector(tuple((min(p[i] for p in vertices)+max(p[i] for p in vertices))/2 for i in range(3)))
    camera=scene.camera;camera.data.ortho_scale=max((max(p[i] for p in vertices)-min(p[i] for p in vertices)) for i in range(3))*1.65
    scene.render.resolution_x=scene.render.resolution_y=384
    for index in range(8):
        az=math.radians(index*45);direction=Vector((math.sin(az)*COS,-math.cos(az)*COS,SIN))
        camera.location=center+direction*3000;camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler()
        for mode in ['actual','solid']:
            scene.view_layers[0].material_override=gray if mode=='solid' else None
            scene.render.filepath=str(dest/f'view-{index}-{mode}.png');bpy.ops.render.render(write_still=True)
    for mode in ['actual','solid']:
        sheet=Image.new('RGBA',(1536,768))
        for index in range(8):sheet.paste(Image.open(dest/f'view-{index}-{mode}.png'),((index%4)*384,(index//4)*384))
        sheet.save(dest/f'{mode}.png')
    target=point((box[0]+box[2])/2,(box[1]+box[3])/2,0)
    camera.location=target+RAY*3000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler()
    camera.data.ortho_scale=box[2]-box[0]
    scene.render.resolution_x=(box[2]-box[0])*6;scene.render.resolution_y=(box[3]-box[1])*6
    for mode in ['actual','solid']:
        scene.view_layers[0].material_override=gray if mode=='solid' else None
        scene.render.filepath=str(dest/f'native-{mode}.png');bpy.ops.render.render(write_still=True)
    metadata.update(model_sha256=sha(dest/'worker.blend'),source_box=box,components=[o.name for o in objects],
                    native_first=True,recipe=record_recipe(dest,Path(__file__)))
    write_json(dest/'manifest.json',metadata)


def barrel(source):
    fitpath=OUT/'restart3-south-cart/barrel-fit-v1/fit.json';fit=json.loads(fitpath.read_text())
    dest=OUT/'restart3-south-cart/barrel-v1';dest.mkdir(exist_ok=False)
    part=source['parts'][2];frame=part['frames'][-1]
    assert sha(Path(frame['image']))==frame['image_sha256']
    rgba=np.asarray(Image.open(frame['image']).convert('RGBA')).copy()
    domain=np.asarray(Image.open(fitpath.parent/'body-domain.png'))>0;rgba[~domain,3]=0
    Image.fromarray(rgba).save(dest/'source.png')
    scene,gray=start();paint=material(dest/'source.png');box=[1154,707,1218,832]
    angle,length,radius,cx,cy=fit['parameters'];axis=Vector((math.cos(angle),math.sin(angle),0));cross=Vector((-axis.y,axis.x,0))
    n=32;phase=math.pi/n;height=(radius+.6)*math.cos(phase)
    center=point(1154+cx,707+cy,height)
    profile=[(-.5,.88),(-.4,.94),(-.28,1),(-.05,1),(.18,1),(.38,.94),(.5,.88)]
    def ring(t,r):return [center+axis*(length*t)+r*(cross*math.cos(phase+j*math.tau/n)+Vector((0,0,math.sin(phase+j*math.tau/n)))) for j in range(n)]
    vertices=[p for t,scale in profile for p in ring(t,radius*scale)]
    faces=[tuple(reversed(range(n))),tuple(range((len(profile)-1)*n,len(profile)*n))]
    for i in range(len(profile)-1):
        for j in range(n):faces.append((i*n+j,i*n+(j+1)%n,(i+1)*n+(j+1)%n,(i+1)*n+j))
    group='croisement02-south-cart-terminal-cask'
    mesh(scene,'Inferred lying cask body',vertices,faces,paint,gray,box,group)
    for index,t in enumerate([-.17,.13]):
        a,b=t-1.3/length,t+1.3/length
        verts=[p for axial,r in [(a,radius),(b,radius),(a,radius+.6),(b,radius+.6)] for p in ring(axial,r)]
        quads=[]
        for j in range(n):
            k=(j+1)%n
            quads.extend([(j,k,n+k,n+j),(2*n+j,3*n+j,3*n+k,2*n+k),
                          (j,2*n+j,2*n+k,k),(n+j,n+k,3*n+k,3*n+j)])
        mesh(scene,f'Cask band {index}',verts,quads,paint,gray,box,group)
    finish(scene,gray,dest,box,dict(status='Private cask hypothesis; physical/source review pending',asset_id=group,
        source_frame=frame,fit_sha256=sha(fitpath),fit=fit['parameters'],contact='Two finite lower band facets at Z0; exact receiver audit pending',
        limitations=fit['limitations']+['Hidden round ends, stave thickness and band material are inferred; gray regions await geometry-approved texture fill.']))


def scraps(source):
    dest=OUT/'restart3-south-cart/loose-wood-v1';dest.mkdir(exist_ok=False)
    part=source['parts'][0];frame=part['frames'][-1];assert sha(Path(frame['image']))==frame['image_sha256']
    rgba=np.asarray(Image.open(frame['image']).convert('RGBA'));selected=np.zeros(rgba.shape[:2],bool)
    regions=[('Long detached board',(54,128,68,157),2.),('Small detached chip',(88,132,99,141),1.5),('Thin detached splinter',(109,138,117,142),.8)]
    scene,gray=start();box=[953,844,1165,1001];records=[]
    for _,(left,top,right,bottom),_ in regions:selected[top:bottom,left:right]=rgba[top:bottom,left:right,3]>127
    bounded=rgba.copy();bounded[~selected,3]=0;Image.fromarray(bounded).save(dest/'source.png');Image.fromarray(selected.astype(np.uint8)*255).save(dest/'source-domain.png')
    paint=material(dest/'source.png');group='croisement02-south-cart-terminal-loose-wood'
    for name,(left,top,right,bottom),thickness in regions:
        ys,xs=np.where((rgba[top:bottom,left:right,3]>127));coords=np.column_stack((xs+left,ys+top)).astype(float)
        hull=ConvexHull(coords);outline=[tuple(coords[i]) for i in hull.vertices];n=len(outline)
        vertices=[]
        # Ground footprint and upper surface share world XY; top projects to the surveyed wood pixels.
        top_points=[point(953+x,844+y,thickness) for x,y in outline]
        vertices.extend(Vector((p.x,p.y,0)) for p in top_points);vertices.extend(top_points)
        faces=[tuple(reversed(range(n))),tuple(range(n,2*n))]+[(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)]
        mesh(scene,name,vertices,faces,paint,gray,box,group)
        records.append(dict(name=name,source_region=[left,top,right,bottom],native_opaque_pixels=len(xs),thickness=thickness,
                            outline=outline,geometry_inference='Convex finite broken plank on ground; fine alpha notches remain source appearance'))
    finish(scene,gray,dest,box,dict(status='Private three detached wood fragments; source/contact review pending',asset_id=group,
        source_frame=frame,regions=records,limitations=['Only three clearly detached wood fragments, not every opaque source particle.',
         'Connected dark ground regions, cast shadows, cart body, wheels, fence and uncertain isolated pixels are not assigned solid geometry.',
         'Planar plank thickness and unseen underside inferred; exact receiver contact audit pending.']))


def main():
    source=json.loads((OUT/'state-target-evidence/south-cart/manifest.json').read_text())
    acquire()
    try:barrel(source);scraps(source)
    finally:release()


if __name__=='__main__':main()
