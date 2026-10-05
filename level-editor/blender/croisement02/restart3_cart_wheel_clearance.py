"""Measure and remove new broken-panel intrusion into unchanged cart wheels."""
import json
import math
import sys
from pathlib import Path
import bpy
import bmesh
from mathutils import Vector

HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from tree_geometry import RAY,SIN,COS
from log_trap_state_candidate import point
from approved_texture_stage import geometry,appearance
from evidence_io import sha,write_json,record_recipe
from render_slots import acquire,release


def main():
    source=OUT/'restart3-south-cart/broken-panel-v2'
    dest=OUT/'restart3-south-cart/wheel-clearance-v1'
    assert not dest.exists()
    prior=json.loads((source/'manifest.json').read_text())
    assert sha(source/'worker.blend')==prior['model_sha256']
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(source/'worker.blend'))
        scene=bpy.context.scene;camera=scene.camera
        new_names=set(prior['new_components'])
        fixed={o.name:(geometry(o),appearance(o)) for o in scene.objects if o.type=='MESH' and o.name not in new_names}
        targets=[scene.objects[name] for name in new_names]
        wheels=[o for o in scene.objects if o.name.startswith(('Wheel ','Hub '))]
        gray=scene.objects['Tipped barrel canopy shell'].data.materials[1]
        fit=json.loads((OUT/'restart2-state/south-cart-body-fit-v3/fit.json').read_text())
        axis=Vector(fit['placement']['across'])
        dest.mkdir(parents=True)

        def render_native(name,solid=False):
            target=point(1055,910,0)
            camera.location=target+RAY*3000
            camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler()
            camera.data.ortho_scale=220
            scene.render.resolution_x=880;scene.render.resolution_y=720
            scene.view_layers[0].material_override=gray if solid else None
            scene.render.filepath=str(dest/name);bpy.ops.render.render(write_still=True)

        def boolean_copy(obj,cutter,operation):
            duplicate=obj.copy();duplicate.data=obj.data.copy();scene.collection.objects.link(duplicate)
            duplicate.name='Temporary measured solid'
            bpy.context.view_layer.objects.active=duplicate
            modifier=duplicate.modifiers.new('Measured exact solid operation','BOOLEAN')
            modifier.operation=operation;modifier.solver='EXACT';modifier.object=cutter
            bpy.ops.object.modifier_apply(modifier=modifier.name)
            return duplicate

        def volume(obj):
            bm=bmesh.new();bm.from_mesh(obj.data)
            result=abs(bm.calc_volume(signed=True));bm.free()
            return result

        def bounds(obj):
            pts=[obj.matrix_world@Vector(v) for v in obj.bound_box]
            return [min(p[i] for p in pts) for i in range(3)],[max(p[i] for p in pts) for i in range(3)]

        def overlapping(a,b):
            alo,ahi=bounds(a);blo,bhi=bounds(b)
            return all(alo[i]<=bhi[i] and blo[i]<=ahi[i] for i in range(3))

        render_native('native-before.png')
        render_native('native-solid-before.png',True)
        before=[];changes=[]
        for obj in targets:
            for wheel in wheels:
                if not overlapping(obj,wheel):continue
                intersection=boolean_copy(obj,wheel,'INTERSECT')
                amount=volume(intersection)
                row=dict(panel=obj.name,wheel=wheel.name,intersection_volume=amount)
                if intersection.data.vertices:
                    lo,hi=bounds(intersection);row['intersection_bounds']=[lo,hi]
                    pixels=[(v.co.x,-v.co.y*SIN-v.co.z*COS) for v in intersection.data.vertices]
                    row['native_intersection_bounds']=[[min(p[i] for p in pixels),max(p[i] for p in pixels)] for i in range(2)]
                before.append(row)
                bpy.data.objects.remove(intersection,do_unlink=True)
                if amount<=1e-4:continue
                cutter=wheel.copy();cutter.data=wheel.data.copy();scene.collection.objects.link(cutter)
                cutter.name='Temporary clearance solid'
                center=sum((v.co for v in cutter.data.vertices),Vector())/len(cutter.data.vertices)
                radius=21 if wheel.name.startswith('Wheel ') else 4
                half_depth=1.5 if wheel.name.startswith('Wheel ') else 3
                for vertex in cutter.data.vertices:
                    delta=vertex.co-center;axial=axis*delta.dot(axis);radial=delta-axial
                    vertex.co=center+axial*((half_depth+.3)/half_depth)+radial*((radius+.3)/radius)
                cutter.data.update()
                replacement=boolean_copy(obj,cutter,'DIFFERENCE')
                assert len(replacement.data.polygons)>0,obj.name
                obj.data=replacement.data
                bpy.data.objects.remove(replacement,do_unlink=True)
                bpy.data.objects.remove(cutter,do_unlink=True)
                changes.append(dict(panel=obj.name,wheel=wheel.name,removed_overlap_volume=amount,clearance=.3))
        after=[]
        for obj in targets:
            for wheel in wheels:
                if not overlapping(obj,wheel):continue
                intersection=boolean_copy(obj,wheel,'INTERSECT');amount=volume(intersection)
                after.append(dict(panel=obj.name,wheel=wheel.name,intersection_volume=amount))
                bpy.data.objects.remove(intersection,do_unlink=True)
                assert amount<1e-3,(obj.name,wheel.name,amount)
        for obj in targets:
            bm=bmesh.new();bm.from_mesh(obj.data)
            assert all(e.is_manifold for e in bm.edges),obj.name
            assert bm.calc_volume(signed=True)>0,obj.name
            bm.free()
            uv=obj.data.uv_layers['Native target projection']
            for face in obj.data.polygons:
                for loop in face.loop_indices:
                    p=obj.data.vertices[obj.data.loops[loop].vertex_index].co
                    uv.data[loop].uv=((p.x-945)/220,1-(-p.y*SIN-p.z*COS-820)/180)
        assert fixed=={o.name:(geometry(o),appearance(o)) for o in scene.objects if o.name in fixed}
        scene.view_layers[0].material_override=None
        bpy.context.preferences.filepaths.save_version=0
        bpy.ops.wm.save_as_mainfile(filepath=str(dest/'worker.blend'),compress=True)
        render_native('native-after.png');render_native('native-solid-after.png',True)
        allpoints=[o.matrix_world@v.co for o in scene.objects if o.type=='MESH' for v in o.data.vertices]
        center=Vector(tuple((min(p[i] for p in allpoints)+max(p[i] for p in allpoints))/2 for i in range(3)))
        scene.render.resolution_x=scene.render.resolution_y=384;camera.data.ortho_scale=270
        for index in range(8):
            az=math.radians(index*45);direction=Vector((math.sin(az)*COS,-math.cos(az)*COS,SIN))
            camera.location=center+direction*3000;camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler()
            for mode in ['actual','solid']:
                scene.view_layers[0].material_override=gray if mode=='solid' else None
                scene.render.filepath=str(dest/f'view-{index}-{mode}.png');bpy.ops.render.render(write_still=True)
        from PIL import Image
        native=Image.open(OUT/'restart2-state/south-cart-wreck-solid-v3/bounded-source.png')
        native.resize((880,720),Image.Resampling.NEAREST).save(dest/'native-source.png')
        for mode in ['actual','solid']:
            sheet=Image.new('RGBA',(1536,768))
            for index in range(8):sheet.paste(Image.open(dest/f'view-{index}-{mode}.png'),((index%4)*384,(index//4)*384))
            sheet.save(dest/f'{mode}.png')
        # Source and renders share identical native pixel bounds, not arbitrary framing.
        comparison=Image.new('RGBA',(880*3,720))
        for index,name in enumerate(['native-source.png','native-before.png','native-after.png']):comparison.paste(Image.open(dest/name),(index*880,0))
        comparison.save(dest/'native-comparison.png')
        prior.update(status='Private wheel-cleared broken panel; review pending',model_sha256=sha(dest/'worker.blend'),
                     prior_model_sha256=sha(source/'worker.blend'),wheel_clearance_changes=changes,
                     original_mesh_geometry_appearance_exact=True,recipe=record_recipe(dest,Path(__file__)))
        write_json(dest/'manifest.json',prior)
        write_json(dest/'wheel-intersection-audit.json',dict(model_sha256=prior['model_sha256'],before=before,after=after,
            source_bounds=[945,820,1165,1000],original_wheels_and_base_exact=True,
            method='Exact mesh boolean intersection volume; subtract expanded wheel only from new timber',
            caveat='Local inferred broken timber notch, not a source-observed wheel break; original wheel remains intact.'))
    finally:release()


if __name__=='__main__':main()
