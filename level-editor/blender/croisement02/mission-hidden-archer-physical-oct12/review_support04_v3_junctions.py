"""Review complete saved poses and measured attachments against exact wood receivers."""
from pathlib import Path
original=Path(__file__).with_name('review_support04_v1.py')
source=original.read_text().replace('geometry-v8-short-support04-v3','geometry-v8-short-support04-v4').replace('geometry-v8-short-support04-review-v1','geometry-v8-short-support04-review-v3-junctions')
needle="    assert b.q.pin(model)==construction['model']\n    (dest/'report.json')"
assert source.count(needle)==1
extra='''    context=json.loads((b.q.OUT/'support04-wood-context-v1/report.json').read_text())
    for record in context['records']:
        path=b.q.ROOT/record['view']['path'];assert b.q.pin(path)==record['view']
        assert b.q.pin(b.q.ROOT/record['source']['path'])==record['source']
        before=set(b.bpy.data.objects);b.bpy.ops.import_scene.gltf(filepath=str(path));added=set(b.bpy.data.objects)-before
        t=record['placement']['transform'];assert t['rot_deg']==0
        for obj in added:
            if obj.parent not in added:obj.location+=b.Vector((t['dx'],-t['dy']/b.S,t['dz']))
    junctions=[]
    for row in construction['rows']:
        if not row['status'].startswith('BUILT'):continue
        phase=row['phase']
        for obj in meshes:obj.hide_render=obj['source_phase']!=phase;obj.hide_viewport=obj.hide_render
        b.bpy.context.view_layer.update()
        center=(b.Vector(row['support_start'])+b.Vector(row['support_end']))*.5
        radius=max(3.,(b.Vector(row['support_start'])-b.Vector(row['support_end'])).length*.7)
        points=[tuple(center+b.Vector((x*radius,y*radius,z*radius)))for x in [-1,1]for y in [-1,1]for z in [-1,1]]
        mesh=b.bpy.data.meshes.new('Junction framing');mesh.from_pydata(points,[],[])
        proxy=b.bpy.data.objects.new('Invisible junction framing',mesh);b.bpy.context.scene.collection.objects.link(proxy);proxy.hide_render=True
        folder=dest/('junction-'+phase+'-'+str(row['component']));folder.mkdir();b.render(proxy,folder)
        b.bpy.data.objects.remove(proxy,do_unlink=True);b.bpy.data.meshes.remove(mesh)
        junctions.append(dict(phase=phase,component=row['component'],sheets=[b.q.pin(folder/f'{mode}-eight.png')for mode in ['textured','solid']]))
    (dest/'junctions.json').write_text(json.dumps(dict(status='PRIVATE_JUNCTION_REVIEW_PENDING',context=context,model=construction['model'],junctions=junctions,limits=['Wood-only context; full-scene finite support contact audit is separate.']),indent=2)+'\\n')
'''
source=source.replace(needle,extra+needle)
source=source.replace('Wood junction context requires separate review.','Wood junction sheets generated separately; independent visual review pending.')
exec(compile(source,__file__,'exec'),{'__name__':'__main__','__file__':__file__})
