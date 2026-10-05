"""Review the attached net against unchanged nearby tree geometry."""
import json,sys
from pathlib import Path
import bpy,bmesh
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement')]
from catalog import OUT,tree_workspace
from log_trap_state_candidate import sha,point
from tree_geometry import RAY
from render_slots import acquire,release


def main():
    base=OUT/'restart2-state/net-attached-v1';wood_only='--wood-only'in sys.argv;dest=base/('joint-wood-only-v2'if wood_only else 'joint-v2');dest.mkdir(exist_ok=True);assert not(dest/'manifest.json').exists()
    frozen=sha(base/'worker.blend');assert frozen==json.loads((base/'manifest.json').read_text())['model_sha256']
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'));scene=bpy.context.scene;bindings=[];wood=[]
        for index in [43,45,46]:
            worker=tree_workspace(index);names=json.loads((worker/'modified/views.json').read_text())['object_names']
            with bpy.data.libraries.load(str(worker/'model.blend'),link=False)as(src,dst):dst.objects=list(names)
            for obj in dst.objects:
                cursor=obj
                while cursor:
                    if cursor.name not in scene.objects:scene.collection.objects.link(cursor)
                    cursor=cursor.parent
                if 'wood 'in obj.name:wood.append(obj)
                elif wood_only:obj.hide_render=True
            bindings.append(dict(tree=index,model_sha256=sha(worker/'model.blend'),objects=names))
        bpy.context.view_layer.update();intersections=[]
        for name in ['Occupied bag','Wooden piece']:
            obj=bpy.data.objects[name]
            for receiver in wood:
                test=obj.copy();test.data=obj.data.copy();scene.collection.objects.link(test);bpy.context.view_layer.objects.active=test
                mod=test.modifiers.new('Independent wood contact probe','BOOLEAN');mod.operation='INTERSECT';mod.solver='EXACT';mod.use_self=True;mod.object=receiver
                bpy.ops.object.modifier_apply(modifier=mod.name);bm=bmesh.new();bm.from_mesh(test.data);volume=abs(bm.calc_volume(signed=True));bm.free();intersections.append(dict(body=name,tree_object=receiver.name,intersection_volume=volume));bpy.data.objects.remove(test,do_unlink=True)
        center=point(1330,1010,135);scene.camera.data.ortho_scale=245;scene.render.resolution_x=scene.render.resolution_y=640;scene.cycles.samples=16;records=[]
        for name,direction in [('source',RAY),('front-left',Vector((-1,1,.8)).normalized()),('front-right',Vector((1,1,.8)).normalized()),('reverse',Vector((0,1,.8)).normalized())]:
            scene.camera.location=center+direction*3000;scene.camera.rotation_euler=(center-scene.camera.location).to_track_quat('-Z','Y').to_euler();path=dest/(name+'.png');scene.render.filepath=str(path);
            if not path.exists():bpy.ops.render.render(write_still=True)
            records.append(dict(view=name,image=path.name,sha256=sha(path),camera_direction=list(direction)))
        assert sha(base/'worker.blend')==frozen
        (dest/'manifest.json').write_text(json.dumps(dict(status='Private joint context proof; visual and physical review pending',net_sha256=frozen,tree_bindings=bindings,body_tree_intersections=intersections,renders=records,limitations=['Trees are imported unchanged for context only.','Cord endpoints intentionally meet the selected branch; this audit tests the two larger bodies separately.']),indent=2)+'\n');print(intersections)
    finally:release()


if __name__=='__main__':main()
