"""Test a complete attached net endpoint with a localized cloth/counterweight contact."""
import json, math, sys
from pathlib import Path
import bpy, bmesh
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement')]
from catalog import OUT
from tree_geometry import RAY,SIN,COS
from scenery_geometry import Mesh
from log_trap_state_candidate import sha
from render_slots import acquire,release


def main():
    base=OUT/'net-endpoint-candidate-v3';dest=OUT/'restart2-state/net-attached-v1'
    dest.mkdir(parents=True,exist_ok=False)
    old=json.loads((base/'manifest.json').read_text());assert sha(base/'worker.blend')==old['model_sha256']
    attachments=json.loads((base/'attachment-physical-occlusion-v2.json').read_text())
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'))
        scene=bpy.context.scene
        for name,shift in [('Occupied bag',54),('Bag hanging cord',54),('Wooden piece',87),('Wood hanging cord',87)]:
            for v in bpy.data.objects[name].data.vertices:v.co+=RAY*shift
        bag=bpy.data.objects['Occupied bag'];wood=bpy.data.objects['Wooden piece']
        def volume(obj):
            bm=bmesh.new();bm.from_mesh(obj.data);assert all(e.is_manifold for e in bm.edges)
            value=bm.calc_volume(signed=True);bm.free();assert value>0;return value
        before=volume(bag)
        bpy.context.view_layer.objects.active=bag
        mod=bag.modifiers.new('Local cloth contact at counterweight','BOOLEAN');mod.operation='DIFFERENCE';mod.solver='EXACT';mod.object=wood
        bpy.ops.object.modifier_apply(modifier=mod.name)
        after=volume(bag)
        assert after>before*.9,'Reject excessive cloth carving'
        fit=json.loads((OUT/'net-endpoint-volume-fit-v2/manifest.json').read_text())
        source=next(r for r in fit['records']if r['family']=='piege01'and r['variant']=='i')
        x,y,w,h=source['bbox']
        for face in bag.data.polygons:
            face.material_index=0 if face.normal.dot(RAY)>.05 else 1
            for loop in face.loop_indices:
                p=bag.data.vertices[bag.data.loops[loop].vertex_index].co
                bag.data.uv_layers.active.data[loop].uv=((p.x-x)/w,1-(-p.y*SIN-p.z*COS-y)/h)
        extensions=[]
        for cord,shift in [('Bag hanging cord',54),('Wood hanging cord',87)]:
            row=next(r for r in attachments['cords']if r['cord']==cord)
            proposal=next(p['proposal']for p in row['hypotheses']if p['proposal']['camera_ray_shift']==shift)
            obj=bpy.data.objects[cord];points=[v.co for v in obj.data.vertices];maximum=max(p.z for p in points)
            top=sum((p for p in points if p.z>maximum-.8),Vector())/len([p for p in points if p.z>maximum-.8])
            end=Vector(proposal['attachment_world']);mesh=Mesh();mesh.tube(top-Vector((0,0,.1)),end+Vector((0,0,.1)),.55,n=8)
            data=bpy.data.meshes.new(cord+' inferred extension');data.from_pydata(mesh.vertices,[],mesh.faces);data.update()
            bm=bmesh.new();bm.from_mesh(data);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(data);bm.free()
            extension=bpy.data.objects.new(data.name,data);scene.collection.objects.link(extension)
            data.materials.append(bpy.data.materials['Unobserved net surfaces'])
            extensions.append(dict(object=extension.name,attachment=proposal,closed_volume=volume(extension)))
        objects=[o for o in scene.objects if o.type=='MESH']
        guard=[dict(name=o.name,closed_volume=volume(o))for o in objects]
        points=[v.co for o in objects for v in o.data.vertices];lo=Vector(tuple(min(p[i]for p in points)for i in range(3)));hi=Vector(tuple(max(p[i]for p in points)for i in range(3)));center=(lo+hi)/2
        scene.camera.location=center+RAY*3000;scene.camera.rotation_euler=(center-scene.camera.location).to_track_quat('-Z','Y').to_euler();scene.camera.data.ortho_scale=(hi-lo).length*1.2
        bpy.ops.wm.save_as_mainfile(filepath=str(dest/'worker.blend'))
        records=[]
        for view in range(8):
            angle=math.pi/2+view*math.pi/4;direction=Vector((math.cos(angle)*COS,math.sin(angle)*COS,SIN))
            scene.camera.location=center+direction*3000;scene.camera.rotation_euler=(center-scene.camera.location).to_track_quat('-Z','Y').to_euler()
            for mode in ['actual','solid']:
                scene.view_layers[0].material_override=bpy.data.materials['Unobserved net surfaces']if mode=='solid'else None
                path=dest/f'{view:02}-{mode}.png';scene.render.filepath=str(path);bpy.ops.render.render(write_still=True);records.append(dict(view=view,mode=mode,image=path.name,sha256=sha(path)))
        report=dict(status='Private attached endpoint hypothesis; reopened guards and visual review pending',model_sha256=sha(dest/'worker.blend'),base_model_sha256=old['model_sha256'],source_sha256=source['source_sha256'],objects=guard,cloth_contact=dict(before_volume=before,after_volume=after,removed_fraction=(before-after)/before),extensions=extensions,renders=records,limitations=['The bag deforms locally around the counterweight; this is an inference, not a measured hidden surface.','Attachments reference unchanged tree45 wood; full joint scene review remains required.','Only occupied piege01 final phase 0 is represented.','Gray unknown surfaces are unfinished appearance.'])
        (dest/'manifest.json').write_text(json.dumps(report,indent=2)+'\n')
    finally:release()


if __name__=='__main__':main()
