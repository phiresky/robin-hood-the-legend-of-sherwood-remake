"""Measure native-domain visibility supplied jointly by the cover and scoped crown RGB."""
import json, sys, time
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.ndimage import binary_erosion
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
import joined02_receiver_rgb_v1 as rgb
from render_slots import acquire,release
o=rgb.original

def prepare(objects):
    white=o.bpy.data.materials.new('Diagnostic source-owned white');white.use_nodes=True
    nt=white.node_tree;nt.nodes.clear();out=nt.nodes.new('ShaderNodeOutputMaterial');em=nt.nodes.new('ShaderNodeEmission');em.inputs[0].default_value=(1,1,1,1);nt.links.new(em.outputs[0],out.inputs[0])
    for obj in o.bpy.context.scene.objects:
        if obj.type=='MESH' and obj.get('source_phase'):
            for slot in obj.material_slots:slot.material=white
    for obj in objects:
        before=rgb.mesh_pin(obj);nodes=[]
        for slot in obj.material_slots:
            material=slot.material
            if material is None:continue
            qualified=obj.name.startswith('Northwest Tree 04 / Crown') and 'observed patches' in material.name
            mat=material.copy();slot.material=mat;nt=mat.node_tree
            bsdf=next(n for n in nt.nodes if n.type=='BSDF_PRINCIPLED')
            alpha=bsdf.inputs['Alpha'];alpha_link=alpha.links[0].from_socket if alpha.is_linked else None
            out=next(n for n in nt.nodes if n.type=='OUTPUT_MATERIAL')
            em=nt.nodes.new('ShaderNodeEmission');em.inputs[0].default_value=(0,0,0,1)
            if qualified:
                tex=nt.nodes.new('ShaderNodeTexImage');tex.interpolation='Closest';tex.extension='CLIP'
                uv=nt.nodes.new('ShaderNodeUVMap');uv.uv_map='Private mission02 source projection';nt.links.new(uv.outputs['UV'],tex.inputs['Vector'])
                nt.links.new(tex.outputs['Alpha'],em.inputs[0]);nodes.append(tex)
            transparent=nt.nodes.new('ShaderNodeBsdfTransparent');mix=nt.nodes.new('ShaderNodeMixShader')
            nt.links.new(transparent.outputs[0],mix.inputs[1]);nt.links.new(em.outputs[0],mix.inputs[2])
            if alpha_link:nt.links.new(alpha_link,mix.inputs[0])
            else:mix.inputs[0].default_value=alpha.default_value
            nt.links.new(mix.outputs[0],out.inputs['Surface'])
        if nodes:rgb.TARGETS.append(dict(obj=obj,nodes=nodes,before=before))
    assert rgb.TARGETS

def main():
    dest=o.q.OUT/'joined02-receiver-coverage-v1'
    source=Path(o.__file__).read_text()
    source=source.replace("for candidate in row['bounds_candidates']:","for candidate in [c for c in row['bounds_candidates'] if c['id']=='croisement02-tree-04']:")
    source=source.replace('transparent_max_bounces=128','transparent_max_bounces=256')
    needle=" bpy.context.view_layer.update();crop=";assert source.count(needle)==1
    source=source.replace(needle," prepare(receivers)\n"+needle)
    needle=" for j,phase in enumerate(['initial','applied']):\n";assert source.count(needle)==1
    source=source.replace(needle,needle+"  phase_rgb(phase,row,DEST)\n")
    scope={'__name__':'private_joint_coverage','__file__':o.__file__,'prepare':prepare,'phase_rgb':rgb.phase_rgb}
    exec(compile(source,o.__file__,'exec'),scope);scope['DEST']=dest;scope['VIEW_REPORT']=o.q.OUT/'joined02-camera-input-v2/report.json';scope['main']()
    report=json.loads((dest/'report.json').read_text());crop=report['crop'];row=json.loads((o.q.OUT/'qualification.json').read_text())['rows'][1];checks=[]
    for phase in ['initial','applied']:
        f=row['art']['states']['initial']['frames'][0] if phase=='initial' else row['art']['states']['transition']['frames'][-1]
        mask=np.zeros((crop[3]-crop[1],crop[2]-crop[0]),bool);rgba=np.array(o.q.rgba(f));x=int(f['top_left'][0])-crop[0];y=int(f['top_left'][1])-crop[1];mask[y:y+rgba.shape[0],x:x+rgba.shape[1]]=rgba[:,:,3]>0
        actual=np.array(Image.open(dest/f'{phase}-joined.png').convert('RGBA'));coverage=actual[:,:,:3].min(2)/255.*actual[:,:,3]/255.;interior=binary_erosion(mask,iterations=1);lost=mask&(coverage<.5)
        diagram=np.zeros((*mask.shape,4),np.uint8);diagram[mask]=[40,140,40,255];diagram[lost]=[255,0,0,255];diagram[lost&interior]=[255,0,255,255];Image.fromarray(diagram).save(dest/f'{phase}-coverage.png')
        checks.append(dict(phase=phase,source_pixels=int(mask.sum()),interior_pixels=int(interior.sum()),lost=int(lost.sum()),interior_lost=int((lost&interior).sum()),coverage=o.q.pin(dest/f'{phase}-coverage.png')))
    (dest/'coverage.json').write_text(json.dumps(dict(status='TARGETED_COMBINED_SOURCE_COVERAGE_PENDING_REVIEW',checks=checks,recipe=o.q.pin(Path(__file__)),limits=['Only Tree04 receiver geometry in this targeted ownership diagnostic.','Unlit source-owned white vs other receiver black; existing alpha retained.','Half coverage threshold separates interior and filtered boundaries; this is not a final color review.','No saved model or canonical change.']),indent=2)+'\n');print(checks,flush=True)

if __name__=='__main__':
    def available():return int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024
    while True:
        while available()<11*1024**3:print('Waiting11GiB for targeted joint coverage',flush=True);time.sleep(30)
        acquire(slots=2)
        try:
            if available()<10*1024**3:continue
            main();break
        finally:release()
