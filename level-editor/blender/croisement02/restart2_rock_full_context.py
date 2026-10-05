"""Inspect the remaining rock rim against exact tree03/tree04/shrub62 physical context."""
import json,sys
from pathlib import Path
import bpy,numpy as np
from mathutils.bvhtree import BVHTree
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace
from log_trap_state_candidate import sha,point
from tree_geometry import RAY
from physical_opacity import OpacityRegistry
from render_slots import acquire,release


def main():
    base=OUT/'rock-trap-state-candidate-v14';dest=OUT/'restart2-state/rock-full-context-v1';dest.mkdir(exist_ok=False)
    frozen=sha(base/'worker.blend');assert frozen==json.loads((base/'manifest.json').read_text())['model_sha256']
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'));scene=bpy.context.scene;rocks=[]
        for obj in scene.objects:
            if obj.get('state_endpoint'):
                obj.hide_render=obj.get('state_endpoint')!='covered'
                if not obj.hide_render:rocks.append(obj)
        context=[];bindings=[]
        workers=[('tree03',tree_workspace(3)),('tree04',tree_workspace(4)),('shrub62',OUT/'understory-round-11/assets/croisement02-shrub-62')]
        for name,worker in workers:
            if name.startswith('tree'):names=json.loads((worker/'modified/views.json').read_text())['object_names']
            else:names=[r['object']for r in json.loads((worker/'inspection/saved-model-audit.json').read_text())['objects']]
            with bpy.data.libraries.load(str(worker/'model.blend'),link=False)as(src,dst):dst.objects=list(names)
            for obj in dst.objects:
                cursor=obj
                while cursor:
                    if cursor.name not in scene.objects:scene.collection.objects.link(cursor)
                    cursor=cursor.parent
                context.append(obj)
            bindings.append(dict(asset=name,worker=str(worker),model_sha256=sha(worker/'model.blend'),objects=names))
        bpy.context.view_layer.update();vertices=[];faces=[];registry=OpacityRegistry();materials=[]
        for obj in context:
            mesh=obj.data;mesh.calc_loop_triangles();offset=len(vertices);vertices.extend(obj.matrix_world@v.co for v in mesh.vertices)
            for triangle in mesh.loop_triangles:
                faces.append(tuple(offset+i for i in triangle.vertices));registry.add(obj,mesh,triangle)
            for material in mesh.materials:
                if material:materials.append(dict(object=obj.name,material=material.name,physical_coverage=bool(material.get('foliage_physical_opacity')),opacity_semantics=material.get('opacity_semantics')))
        tree=registry.wrap(BVHTree.FromPolygons(vertices,faces,all_triangles=True));rock_trees=[BVHTree.FromPolygons([obj.matrix_world@v.co for v in obj.data.vertices],[list(p.vertices)for p in obj.data.polygons])for obj in rocks]
        source=json.loads((OUT/'state-target-evidence/rock-trap/manifest.json').read_text());left,top,right,bottom=source['bbox'];w,h=right-left,bottom-top;scale=max(w,h)*1.2
        diagnostic=OUT/'state-target-evidence/rock-trap/unknown-rim-context-v5';rgba=np.asarray(Image.open(diagnostic/'rim-actual-locations.png'));selected=np.all(rgba[:,:,:3]==[255,30,160],axis=2);records=[]
        for yy,xx in np.argwhere(selected):
            sx=left+w/2+(float(xx)+.5-256)*scale/512;sy=top+h/2+(float(yy)+.5-256)*scale/512;origin=point(sx,sy,0)+RAY*5000;hits=[t.ray_cast(origin,-RAY)for t in rock_trees];hits=[h for h in hits if h[0]is not None];assert hits
            distance=min(h[3]for h in hits);hit=tree.ray_cast(origin,-RAY,distance-.001)
            records.append(dict(render_pixel=[int(xx),int(yy)],native_pixel=[int(sx),int(sy)],occluded_by_context=hit[0]is not None))
        target=point((left+right)/2,(top+bottom)/2,0);scene.camera.location=target+RAY*3000;scene.camera.rotation_euler=(target-scene.camera.location).to_track_quat('-Z','Y').to_euler();scene.camera.data.ortho_scale=scale;scene.cycles.transparent_max_bounces=256;scene.cycles.samples=24
        for name,show in [('full-context',True),('rocks-only',False)]:
            for obj in context:obj.hide_render=not show
            path=dest/(name+'.png');scene.render.filepath=str(path);bpy.ops.render.render(write_still=True)
        result=dict(status='Exact physical context diagnostic; no model changes',rock_model_sha256=frozen,context_bindings=bindings,materials=materials,selected_render_pixels=len(records),occluded_render_pixels=sum(r['occluded_by_context']for r in records),remaining_native_pixels=len({tuple(r['native_pixel'])for r in records if not r['occluded_by_context']}),records=records,limitations=['Physical opacity is respected where explicitly declared; material bindings are included for audit.','Geometry overlap is not new source ownership authority.','Only the covered endpoint and current static context are tested.'])
        (dest/'manifest.json').write_text(json.dumps(result,indent=2)+'\n');print({k:v for k,v in result.items()if k not in ['records','materials','context_bindings']})
    finally:release()


if __name__=='__main__':main()
