"""Private diagnostic of scoped state RGB on unchanged observed receiver faces."""
import sys,json,time,hashlib
from pathlib import Path
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
import joined02_review as original
from render_slots import acquire,release
TARGETS=[]

def mesh_pin(obj):
    mesh=obj.data
    return hashlib.sha256(repr(([tuple(v.co)for v in mesh.vertices],
        [tuple(p.vertices)for p in mesh.polygons],
        [(layer.name,[tuple(v.uv)for v in layer.data])for layer in mesh.uv_layers
         if layer.name!='Private mission02 source projection'])).encode()).hexdigest()

def prepare(objects):
    for obj in objects:
        if not obj.name.startswith('Northwest Tree 04 / Crown'):continue
        before=mesh_pin(obj);nodes=[]
        for slot in obj.material_slots:
            material=slot.material
            if material is None or 'observed patches'not in material.name:continue
            # RGB only: existing alpha socket, topology and culling remain untouched.
            mat=material.copy();slot.material=mat;nt=mat.node_tree
            bsdf=next(n for n in nt.nodes if n.type=='BSDF_PRINCIPLED')
            color=bsdf.inputs['Base Color'];mix=nt.nodes.new('ShaderNodeMixRGB');mix.blend_type='MIX'
            if color.is_linked:nt.links.new(color.links[0].from_socket,mix.inputs[1])
            else:mix.inputs[1].default_value=color.default_value
            tex=nt.nodes.new('ShaderNodeTexImage');tex.interpolation='Closest';tex.extension='CLIP'
            uv=nt.nodes.new('ShaderNodeUVMap');uv.uv_map='Private mission02 source projection'
            nt.links.new(uv.outputs['UV'],tex.inputs['Vector']);nt.links.new(tex.outputs['Color'],mix.inputs[2])
            nt.links.new(tex.outputs['Alpha'],mix.inputs[0]);nt.links.new(mix.outputs[0],color)
            nodes.append(tex)
        if nodes:
            TARGETS.append(dict(obj=obj,nodes=nodes,before=before))
    assert TARGETS,'No qualified observed Tree04 receiver material found'

def phase_rgb(phase,row,dest):
    frame=row['art']['states']['initial']['frames'][0]if phase=='initial'else row['art']['states']['transition']['frames'][-1]
    path=dest/f'{phase}-unchanged-source.png';image=original.q.rgba(frame);image.save(path)
    im=original.bpy.data.images.load(str(path));im.pack();x0,y0=frame['top_left'];w,h=image.size
    original.bpy.context.view_layer.update()
    for target in TARGETS:
        obj=target['obj'];mesh=obj.data
        layer=mesh.uv_layers.get('Private mission02 source projection')or mesh.uv_layers.new(name='Private mission02 source projection')
        for loop in mesh.loops:
            p=obj.matrix_world@mesh.vertices[loop.vertex_index].co
            layer.data[loop.index].uv=((p.x-x0)/w,1-(-p.y*original.S-p.z*original.C-y0)/h)
        for tex in target['nodes']:tex.image=im
        assert mesh_pin(obj)==target['before'],'Receiver source coordinates changed'

def main():
    source=Path(original.__file__).read_text();needle=" bpy.context.view_layer.update();crop="
    assert source.count(needle)==1
    source=source.replace(needle," prepare(receivers)\n"+needle)
    needle=" for j,phase in enumerate(['initial','applied']):\n"
    assert source.count(needle)==1
    source=source.replace(needle,needle+"  phase_rgb(phase,row,DEST)\n")
    source=source.replace('transparent_max_bounces=128','transparent_max_bounces=256')
    scope={'__name__':'private_rgb_diagnostic','__file__':original.__file__,
           'prepare':prepare,'phase_rgb':phase_rgb}
    exec(compile(source,original.__file__,'exec'),scope)
    dest=original.q.OUT/'joined02-receiver-rgb-diagnostic-v1'
    scope['DEST']=dest;scope['VIEW_REPORT']=original.q.OUT/'joined02-camera-input-v2/report.json';scope['main']()
    p=dest/'report.json';report=json.loads(p.read_text())
    report['status']='PRIVATE_EPHEMERAL_RECEIVER_RGB_COMPARISON_NOT_MODEL_APPROVAL'
    report['inference']='Native mission RGB affects existing observed Tree04 crown surfaces where the mission alpha covers them. This is a state appearance hypothesis, not a shadow or physical leaf classification.'
    report['limits']+=['Receiver positions, topology, original UV and alpha/culling unchanged; extra projected UV changes RGB only.',
        'Scene is an ephemeral diagnostic, not a saved-model approval packet.',
        'Native RGB passes through the existing PBR material and lighting; exact final appearance is not yet qualified.',
        'No canonical source or model changed.']
    report['targets']=[dict(name=t['obj'].name,source_mesh_fingerprint=t['before'])for t in TARGETS]
    report['recipe']=original.q.pin(Path(__file__))
    report['base_recipe']=original.q.pin(Path(original.__file__))
    p.write_text(json.dumps(report,indent=2)+'\n')

if __name__=='__main__':
    def available():return int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024
    while True:
        while available()<14*1024**3:print('Waiting14GiB before private RGB comparison',flush=True);time.sleep(30)
        acquire(slots=2)
        try:
            if available()<10*1024**3:continue
            main();break
        finally:release()
