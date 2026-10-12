"""Construct private short wood attachments from qualified native pixel rays."""
import hashlib,json,sys,time
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
import build_models as build
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from render_slots import acquire,release

def fingerprint(obj):
    mesh=obj.data
    return hashlib.sha256(repr((
        [tuple(v.co) for v in mesh.vertices],
        [(tuple(p.vertices),p.material_index) for p in mesh.polygons],
        [[tuple(x.uv) for x in layer.data] for layer in mesh.uv_layers],
    )).encode()).hexdigest()

def main():
    dest=build.q.OUT/'geometry-v8-short-support04'
    assert not dest.exists()
    build.DEST=dest;build.guard();dest.mkdir()
    candidate_path=build.q.OUT/'support-route-probe-v1/all-pose-attachment-candidates-v2.json'
    candidates=json.loads(candidate_path.read_text())
    selection=json.loads((build.q.OUT/'conformed-v5-selected.json').read_text())
    model=next(x for x in selection['models'] if 'profile-04/' in x['path'])
    assert build.q.pin(build.q.ROOT/model['path'])==model
    build.bpy.ops.wm.open_mainfile(filepath=str(build.q.ROOT/model['path']))
    originals={o['source_phase']:o for o in build.bpy.context.scene.objects if o.type=='MESH'}
    before={p:fingerprint(o) for p,o in originals.items()}
    rows=[]
    for state in candidates['states']:
        phase=state['phase'];obj=originals[phase];mesh=obj.data;mesh.calc_loop_triangles()
        tree=BVHTree.FromPolygons([obj.matrix_world@v.co for v in mesh.vertices],
             [tuple(t.vertices) for t in mesh.loop_triangles],all_triangles=True)
        for component in state['components']:
            qualified=[c for c in component['candidates'] if 0<c['gap']<=6]
            if not qualified:
                rows.append(dict(phase=phase,component=component['component'],pixels=component['pixels'],status='UNRESOLVED_NO_SHORT_ATTACHMENT'))
                continue
            c=min(qualified,key=lambda c:c['gap']);sx,sy=c['screen']
            origin=Vector((sx,-sy/build.S,0))-build.RAY*6000
            back,normal,index,distance=tree.ray_cast(origin,build.RAY)
            assert back is not None and mesh.loop_triangles[index].material_index==1
            depth=back.dot(build.RAY)
            assert abs(depth-c['leaf_back_depth'])<.02,(phase,depth,c)
            target=Vector((c['target']['world_yup'][0],-c['target']['world_yup'][2],c['target']['world_yup'][1]))
            assert abs((back-target).length-c['gap'])<.02
            radius=.2;start=back+build.RAY*.2;end=target-build.RAY*.2
            build.bpy.ops.mesh.primitive_cylinder_add(vertices=12,radius=radius,depth=(end-start).length,
                end_fill_type='NGON',location=(start+end)*.5)
            stem=build.bpy.context.object;stem.name=f'Inferred short support04-{phase}-{component["component"]}'
            stem.rotation_euler=(end-start).to_track_quat('Z','Y').to_euler()
            stem.data.materials.append(mesh.materials[1])
            stem['source_phase']=phase;stem['inferred_support']=True
            stem['attachment_receiver']=c['target']['node'];stem.hide_render=phase!='initial';stem.hide_viewport=phase!='initial'
            build.bpy.context.view_layer.update()
            for v in stem.data.vertices:
                p=stem.matrix_world@v.co
                assert abs(p.x-sx)<.21 and abs(-p.y*build.S-p.z*build.C-sy)<.21
            rows.append(dict(phase=phase,component=component['component'],pixels=component['pixels'],
                status='BUILT_PRIVATE_CONTACT_AND_JOINED_REVIEW_PENDING',candidate=c,
                saved_model_back=list(back),support_start=list(start),support_end=list(end),radius=radius))
    assert before=={p:fingerprint(o) for p,o in originals.items()},'Observed source meshes changed'
    build.bpy.context.preferences.filepaths.save_version=0
    path=dest/'model.blend';build.bpy.ops.wm.save_as_mainfile(filepath=str(path),compress=True)
    report=dict(status='PRIVATE_PARTIAL_SUPPORT_REQUIRES_SWEEP_AND_JOINED_REVIEW',
        model=build.q.pin(path),input_model=model,candidates=build.q.pin(candidate_path),rows=rows,
        source_mesh_fingerprints=before,limits=[
            'Only measured short attachments constructed; unresolved components retained unchanged.',
            'Centerline is clear in qualified rays; finite tube sweep and wood endpoint contact remain unchecked.',
            'Each stem projects strictly inside its observed source pixel; joined all-view/source coverage still required.',
            'No canonical models, source images, or qualified state couplings changed.'])
    (dest/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(dict(built=sum(r['status'].startswith('BUILT') for r in rows),unresolved=sum(r['status'].startswith('UNRESOLVED')for r in rows))),flush=True)

if __name__=='__main__':
    def available():return int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024
    while True:
        while available()<14*1024**3:print('Waiting14GiB before support queue',flush=True);time.sleep(30)
        acquire(slots=2)
        try:
            if available()<10*1024**3:continue
            main();break
        finally:release()
