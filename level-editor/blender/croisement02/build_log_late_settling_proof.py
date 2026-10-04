"""Isolated two-body late motion: native timing, source rays and continuous support."""
import json,sys
from pathlib import Path
import bpy
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from catalog import OUT
from tree_geometry import SIN,RAY
from log_trap_state_candidate import point,sha
from audit_log_endpoint_contact import planar_triangle,overlap
from render_slots import acquire,release

def main():
    dest=OUT/'log-late-settling-proof-v1';tracks=json.loads((dest/'tracks.json').read_text());base=OUT/'log-trap-state-candidate-v14';basehash=sha(base/'worker.blend');assert not(dest/'motion.blend').exists()
    bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'));scene=bpy.context.scene;names=[t['object']for t in tracks['tracks']]
    for obj in list(scene.objects):
        if obj.get('state_endpoint') and obj.name not in names:bpy.data.objects.remove(obj,do_unlink=True)
    logs=[bpy.data.objects[name]for name in names]
    bank=OUT/'terrain-bank-candidate/assets/croisement02-north-woodland-bank';bankhash=sha(bank/'model.blend');audit=json.loads((bank/'inspection/saved-model-audit.json').read_text());banknames=[r['object']for r in audit['objects']if r['source_node']in[f'building-{i:03d}'for i in range(5)]]
    with bpy.data.libraries.load(str(bank/'model.blend'),link=False)as(src,dst):dst.objects=banknames
    for obj in dst.objects:scene.collection.objects.link(obj)
    bpy.context.view_layer.update();planes=[]
    for obj in dst.objects:
        obj.data.calc_loop_triangles()
        for triangle in obj.data.loop_triangles:
            plane=planar_triangle([obj.matrix_world@obj.data.vertices[i].co for i in triangle.vertices])
            if plane:planes.append(plane)
        obj.hide_render=True
    records=[];scene.render.fps=25;scene.frame_start=64;scene.frame_end=88
    for track in tracks['tracks']:
        obj=bpy.data.objects[track['object']];obj.hide_render=False;obj.data.calc_loop_triangles()
        for key in track['keys']:
            dx,dy=key['offset'];shift=Vector((dx,-dy/SIN,0));minimum=min(v.co.z for v in obj.data.vertices)
            for triangle in obj.data.loop_triangles:
                plane=planar_triangle([obj.data.vertices[i].co+shift for i in triangle.vertices])
                if plane is None:continue
                points,n,c=plane
                for bp,bn,bc in planes:
                    if any(max(p[a]for p in points)<min(p[a]for p in bp)or max(p[a]for p in bp)<min(p[a]for p in points)for a in(0,1)):continue
                    intersection=overlap(points,bp);area=abs(sum(a[0]*b[1]-b[0]*a[1]for a,b in zip(intersection,intersection[1:]+intersection[:1])))/2
                    if area<1e-5:continue
                    for x,y in intersection:minimum=min(minimum,(c-n[0]*x-n[1]*y)/n[2]-(bc-bn[0]*x-bn[1]*y)/bn[2])
            assert minimum>=-.05,(obj.name,key,minimum)
            obj.location=shift;obj.keyframe_insert('location',frame=key['tick']+1);records.append(dict(object=obj.name,tick=key['tick'],source_offset=key['offset'],minimum_clearance=minimum))
        action=obj.animation_data.action
        for layer in action.layers:
            for strip in layer.strips:
                for bag in strip.channelbags:
                    for curve in bag.fcurves:
                        for keyframe in curve.keyframe_points:keyframe.interpolation='CONSTANT'
    for obj in list(dst.objects):bpy.data.objects.remove(obj,do_unlink=True)
    scene.frame_set(64);bpy.ops.wm.save_as_mainfile(filepath=str(dest/'motion.blend'))
    bpy.ops.object.select_all(action='DESELECT')
    for obj in logs:obj.select_set(True)
    bpy.context.view_layer.objects.active=logs[0]
    bpy.ops.export_scene.gltf(filepath=str(dest/'motion.glb'),export_format='GLB',use_selection=True,export_animations=True,export_frame_range=True,export_force_sampling=True)
    acquire()
    try:
        scene.cycles.samples=12;scene.cycles.use_denoising=False
        for tick in [63,72,87]:
            scene.frame_set(tick+1)
            for view,direction in [('source',RAY),('oblique',Vector((-1,-1,.8)).normalized())]:
                center=point(574,525,0)if view=='source'else point(574,525,50);scene.camera.location=center+direction*3000;scene.camera.rotation_euler=(center-scene.camera.location).to_track_quat('-Z','Y').to_euler();scene.camera.data.ortho_scale=160;scene.render.filepath=str(dest/f'{tick:03d}-{view}.png');bpy.ops.render.render(write_still=True)
    finally:release()
    assert sha(base/'worker.blend')==basehash;assert sha(bank/'model.blend')==bankhash
    (dest/'manifest.json').write_text(json.dumps(dict(status='private partial motion prototype; full state animation incomplete',base_model_sha256=basehash,bank_model_sha256=bankhash,tracks_sha256=sha(dest/'tracks.json'),model_sha256=sha(dest/'motion.blend'),glb_sha256=sha(dest/'motion.glb'),contact_checks=records,limitations=['Only two exposed upper bodies fromtick63to87; eight other bodies and earliercollapse not represented.','Static own-source endpoint RGB used to inspect geometry motion; native rolling/material phase not claimed.','Standalone GLB animation export requires independent playback verification before any supported runtime claim.']),indent=2)+'\n')
if __name__=='__main__':main()
