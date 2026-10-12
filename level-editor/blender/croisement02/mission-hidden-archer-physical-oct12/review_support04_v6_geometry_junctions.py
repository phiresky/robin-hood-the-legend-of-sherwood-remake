"""Low-memory exact wood contact views with diagnostic support highlighting."""
import inspect,json,sys,time
from pathlib import Path
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
import build_models as b
from build_support04_v1 import fingerprint
from render_slots import acquire,release

def main():
    dest=b.q.OUT/'geometry-v8-short-support04-review-v6-geometry-junctions';assert not dest.exists();b.DEST=dest;b.guard();dest.mkdir()
    construction=json.loads((b.q.OUT/'geometry-v8-short-support04-v4/report.json').read_text());model=b.q.ROOT/construction['model']['path'];assert b.q.pin(model)==construction['model']
    b.bpy.ops.wm.open_mainfile(filepath=str(model));meshes=[o for o in b.bpy.context.scene.objects if o.type=='MESH']
    assert {o['source_phase']:fingerprint(o)for o in meshes if not o.get('inferred_support')}==construction['source_mesh_fingerprints']
    highlight=b.bpy.data.materials.new('Diagnostic support orange');highlight.use_nodes=True;highlight.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(1,.18,.015,1)
    for obj in meshes:
        if obj.get('inferred_support'):obj.data.materials.clear();obj.data.materials.append(highlight)
    context=json.loads((b.q.OUT/'support04-wood-context-v2-colors/report.json').read_text());receivers=[]
    for record in context['records']:
        path=b.q.ROOT/record['view']['path'];assert b.q.pin(path)==record['view'];assert b.q.pin(b.q.ROOT/record['source']['path'])==record['source']
        before=set(b.bpy.data.objects);b.bpy.ops.import_scene.gltf(filepath=str(path));added=set(b.bpy.data.objects)-before;t=record['placement']['transform'];assert t['rot_deg']==0
        for obj in added:
            if obj.parent not in added:obj.location+=b.Vector((t['dx'],-t['dy']/b.S,t['dz']))
            if obj.type=='MESH':receivers.append(obj)
    scope=dict(vars(b));render_source=inspect.getsource(b.render).replace("['textured','solid']","['textured']");exec(compile(render_source,__file__,'exec'),scope);render=scope['render'];reviews=[]
    for row in construction['rows']:
        if not row['status'].startswith('BUILT'):continue
        phase=row['phase'];target=row['candidate']['target']['node']
        for obj in meshes:obj.hide_render=obj['source_phase']!=phase;obj.hide_viewport=obj.hide_render
        for obj in receivers:obj.hide_render=obj.name!=target
        assert sum(not o.hide_render for o in receivers)==1,target
        b.bpy.context.view_layer.update();start=b.Vector(row['support_start']);end=b.Vector(row['support_end']);center=(start+end)*.5;radius=max(3.,(start-end).length*.7)
        points=[tuple(center+b.Vector((x*radius,y*radius,z*radius)))for x in [-1,1]for y in [-1,1]for z in [-1,1]];mesh=b.bpy.data.meshes.new('Junction framing');mesh.from_pydata(points,[],[]);proxy=b.bpy.data.objects.new('Invisible framing',mesh);b.bpy.context.scene.collection.objects.link(proxy);proxy.hide_render=True
        folder=dest/('junction-'+phase+'-'+str(row['component']));folder.mkdir();render(proxy,folder);b.bpy.data.objects.remove(proxy,do_unlink=True);b.bpy.data.meshes.remove(mesh)
        reviews.append(dict(phase=phase,component=row['component'],target=target,sheet=b.q.pin(folder/'textured-eight.png')))
    assert b.q.pin(model)==construction['model']
    (dest/'report.json').write_text(json.dumps(dict(status='PRIVATE_GEOMETRY_JUNCTION_REVIEW_PENDING',model=construction['model'],context=context,reviews=reviews,recipe=b.q.pin(Path(__file__)),limits=['Exact saved geometry; diagnostic orange supports and brown opaque wood. This is not a wood texture appearance review.','Only intended target wood visible; full-scene finite sweep evidence is separate.','Complete64saved model views already retained in prior packets.']),indent=2)+'\n')

if __name__=='__main__':
    def available():return int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024
    while True:
        while available()<11*1024**3:print('Waiting11GiB for geometry junctions',flush=True);time.sleep(30)
        acquire(slots=2)
        try:
            if available()<10*1024**3:continue
            main();break
        finally:release()
