"""Build and review a full triangular stack while preserving the fallen endpoint."""
import hashlib,json,math,sys
from pathlib import Path
import bpy,bmesh
from mathutils import Vector
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement')]
from catalog import OUT
from tree_geometry import RAY,SIN,COS
from scenery_geometry import Mesh
from render_slots import acquire,release


def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def signature(objects):
    return hashlib.sha256(json.dumps([dict(name=o.name,vertices=[list(v.co)for v in o.data.vertices],faces=[list(f.vertices)for f in o.data.polygons],material_indices=[f.material_index for f in o.data.polygons],materials=[m.name for m in o.data.materials],uvs=[[list(d.uv)for d in layer.data]for layer in o.data.uv_layers])for o in sorted(objects,key=lambda o:o.name)],sort_keys=True).encode()).hexdigest()
def main():
    dest=OUT/'restart2-state/log-triangular-pile-v1';fit=json.loads((dest/'fit.json').read_text());base=OUT/'log-trap-state-candidate-v14/worker.blend';assert sha(base)==fit['source_model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(base));scene=bpy.context.scene;bpy.context.view_layer.update();fallen=[o for o in scene.objects if o.get('state_endpoint')=='applied'];before=signature(fallen);covered=[o for o in scene.objects if o.get('state_endpoint')=='covered'];materials=list(covered[0].data.materials)
    for o in covered:bpy.data.objects.remove(o,do_unlink=True)
    source=json.loads((OUT/'state-target-evidence/log-trap/manifest.json').read_text());left,top,right,bottom=source['bbox'];objects=[];guards=[]
    for index,row in enumerate(fit['records']):
        mesh=Mesh();mesh.tube(Vector(row['start']),Vector(row['end']),row['radius'],n=24);data=bpy.data.meshes.new(f'covered pile layer{row["layer"]} column{row["column"]}');data.from_pydata(mesh.vertices,[],mesh.faces);data.update();bm=bmesh.new();bm.from_mesh(data);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges);volume=bm.calc_volume(signed=True);assert volume>0;bm.to_mesh(data);bm.free();obj=bpy.data.objects.new(data.name,data);scene.collection.objects.link(obj)
        for material in materials:data.materials.append(material)
        uv=data.uv_layers.new(name='Native target projection')
        for face in data.polygons:
            face.material_index=0 if row['column']==0 and face.normal.dot(RAY)>.05 else 1
            for loop in face.loop_indices:
                p=data.vertices[data.loops[loop].vertex_index].co;uv.data[loop].uv=((p.x-left)/(right-left),1-(-p.y*SIN-p.z*COS-top)/(bottom-top))
        obj['state_endpoint']='covered';obj['native_source_tick']=-1;obj['geometry_status']='Unapproved triangular pile prototype';obj['source_role']=row['role'];objects.append(obj);guards.append(dict(name=obj.name,closed_volume=volume,layer=row['layer'],column=row['column'],role=row['role']))
    assert signature(fallen)==before
    for o in fallen:o.hide_render=True
    points=[v.co for o in objects for v in o.data.vertices];lo=Vector(tuple(min(p[i]for p in points)for i in range(3)));hi=Vector(tuple(max(p[i]for p in points)for i in range(3)));center=(lo+hi)/2
    scene.camera.data.ortho_scale=(hi-lo).length*1.12;scene.render.resolution_x=scene.render.resolution_y=512;scene.render.resolution_percentage=100;scene.cycles.samples=8;scene.cycles.use_denoising=False;scene.camera.location=center+RAY*3000;scene.camera.rotation_euler=(center-scene.camera.location).to_track_quat('-Z','Y').to_euler();bpy.ops.wm.save_as_mainfile(filepath=str(dest/'worker.blend'));records=[];acquire()
    try:
        for view in range(9):
            angle=-math.pi/2+view*math.pi/4;direction=Vector((math.cos(angle)*COS,math.sin(angle)*COS,SIN))if view<8 else Vector(fit['axis']);scene.camera.location=center+direction*3000;scene.camera.rotation_euler=(center-scene.camera.location).to_track_quat('-Z','Y').to_euler()
            for mode in ['actual','solid']:
                scene.view_layers[0].material_override=materials[1]if mode=='solid'else None;path=dest/f'{view:02}-{mode}.png';scene.render.filepath=str(path);bpy.ops.render.render(write_still=True);records.append(dict(view=view,mode=mode,direction=list(direction),sha256=sha(path),image=path.name))
        for mode in ['actual','solid']:
            sheet=Image.new('RGB',(2048,1088),'#282828');draw=ImageDraw.Draw(sheet)
            for view in range(8):
                im=Image.open(dest/f'{view:02}-{mode}.png').convert('RGBA');x,y=(view%4)*512,(view//4)*544;sheet.paste(im,(x,y+32),im);draw.text((x+8,y+8),'Original game camera / art view'if view==0 else f'Orbit {view}',fill='white')
            sheet.save(dest/f'{mode}-sheet.png')
    finally:release()
    assert signature(fallen)==before and sha(base)==fit['source_model_sha256'];report=dict(status='PRIVATE prototype; source cap error and inferred supports require refinement/review',model_sha256=sha(dest/'worker.blend'),source_model_sha256=fit['source_model_sha256'],fit_sha256=sha(dest/'fit.json'),fallen_endpoint_geometry_uv_material_signature=before,fallen_endpoint_unchanged=True,objects=guards,renders=records,native_first=True,maximum_source_cap_error=fit['maximum_endpoint_error_pixels'],limitations=['Not approved. Uniform radius prototype intentionally measures its native alignment error.','Supporting courses are inferred; source-visible new support surfaces remain gray.','Exact current bank contact and longitudinal support audit still required.']);(dest/'manifest.json').write_text(json.dumps(report,indent=2)+'\n')
if __name__=='__main__':main()
