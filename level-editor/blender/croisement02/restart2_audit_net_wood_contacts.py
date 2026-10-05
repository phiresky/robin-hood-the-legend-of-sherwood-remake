"""Check net/tree contacts using temporary world-space, outward-wound receiver probes."""
import json,sys
from pathlib import Path
import bpy,bmesh
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent)]
from catalog import OUT,tree_workspace
from log_trap_state_candidate import sha


def main():
    base=OUT/'restart2-state/net-attached-v1';bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'));scene=bpy.context.scene;receivers=[];audit=[]
    for index in [43,45,46]:
        worker=tree_workspace(index);names=[n for n in json.loads((worker/'modified/views.json').read_text())['object_names']if 'wood 'in n]
        with bpy.data.libraries.load(str(worker/'model.blend'),link=False)as(src,dst):dst.objects=list(names)
        for obj in dst.objects:
            cursor=obj
            while cursor:
                if cursor.name not in scene.objects:scene.collection.objects.link(cursor)
                cursor=cursor.parent
        bpy.context.view_layer.update()
        for obj in dst.objects:
            bm=bmesh.new();bm.from_mesh(obj.data);original_volume=bm.calc_volume(signed=True);closed=all(e.is_manifold for e in bm.edges);boundary=sum(not e.is_manifold for e in bm.edges);bm.free()
            # These are audit-only copies; original model files remain unchanged.
            mesh=bpy.data.meshes.new('World receiver probe');mesh.from_pydata([obj.matrix_world@v.co for v in obj.data.vertices],[],[list(p.vertices)for p in obj.data.polygons]);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));normalized_volume=bm.calc_volume(signed=True);bm.to_mesh(mesh);bm.free();probe=bpy.data.objects.new('Probe '+obj.name,mesh);scene.collection.objects.link(probe)
            receivers.append(probe);audit.append(dict(tree=index,object=obj.name,worker_sha256=sha(worker/'model.blend'),closed=closed,nonmanifold_edges=boundary,original_signed_volume=original_volume,world_matrix_determinant=obj.matrix_world.determinant(),normalized_world_volume=normalized_volume))
    results=[]
    for name in ['Occupied bag','Wooden piece']:
        obj=bpy.data.objects[name]
        for probe,record in zip(receivers,audit):
            if not record['closed']:
                results.append(dict(body=name,receiver=record['object'],status='Volume test invalid for nonclosed receiver'));continue
            test=obj.copy();test.data=obj.data.copy();scene.collection.objects.link(test);bpy.context.view_layer.objects.active=test;mod=test.modifiers.new('Closed world receiver intersection','BOOLEAN');mod.operation='INTERSECT';mod.solver='EXACT';mod.use_self=True;mod.object=probe;bpy.ops.object.modifier_apply(modifier=mod.name);bm=bmesh.new();bm.from_mesh(test.data);v=abs(bm.calc_volume(signed=True));bm.free();bpy.data.objects.remove(test,do_unlink=True);results.append(dict(body=name,receiver=record['object'],intersection_volume=v))
    report=dict(status='Independent world-space receiver diagnostic',model_sha256=sha(base/'worker.blend'),receivers=audit,intersections=results,limitations=['Outward winding is normalized only on temporary audit copies.','Nonclosed receivers cannot support an exact solid-volume claim.'])
    (base/'wood-contact-audit-v2.json').write_text(json.dumps(report,indent=2)+'\n');print(report)


if __name__=='__main__':main()
