"""Private source-view trap composition on unchanged physical crown geometry."""
import hashlib
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from catalog import OUT,tree_workspace
from tree_geometry import RAY,SIN,COS
from log_trap_state_candidate import point,sha
from native_log_foreground_reference import crop_frame
from render_slots import acquire,release


def geometry_hash(objects):
    digest=hashlib.sha256()
    for obj in sorted(objects,key=lambda o:o.name):
        digest.update(obj.name.encode());digest.update(np.array(obj.matrix_world,dtype='<f8').tobytes())
        digest.update(np.array([tuple(v.co)for v in obj.data.vertices],dtype='<f4').tobytes())
        for polygon in obj.data.polygons:digest.update(np.array(polygon.vertices,dtype='<u4').tobytes())
        for uv in obj.data.uv_layers:digest.update(np.array([tuple(v.uv)for v in uv.data],dtype='<f4').tobytes())
    return digest.hexdigest()


def scoped_material(original,wood,phase,box):
    material=original.copy();material.name=original.name+' / isolated trap appearance'
    nodes=material.node_tree.nodes;links=material.node_tree.links
    output=next(n for n in nodes if n.type=='OUTPUT_MATERIAL'and n.is_active_output)
    surface=output.inputs['Surface'];assert len(surface.links)==1
    original_surface=surface.links[0].from_socket
    left,top,right,bottom=box;w=right-left;h=bottom-top
    position=nodes.new('ShaderNodeNewGeometry')
    horizontal=nodes.new('ShaderNodeVectorMath');horizontal.operation='DOT_PRODUCT';horizontal.inputs[1].default_value=(1/w,0,0);links.new(position.outputs['Position'],horizontal.inputs[0])
    x=nodes.new('ShaderNodeMath');x.operation='ADD';x.inputs[1].default_value=-left/w;links.new(horizontal.outputs['Value'],x.inputs[0])
    vertical=nodes.new('ShaderNodeVectorMath');vertical.operation='DOT_PRODUCT';vertical.inputs[1].default_value=(0,SIN/h,COS/h);links.new(position.outputs['Position'],vertical.inputs[0])
    y=nodes.new('ShaderNodeMath');y.operation='ADD';y.inputs[1].default_value=1+top/h;links.new(vertical.outputs['Value'],y.inputs[0])
    uv=nodes.new('ShaderNodeCombineXYZ');links.new(x.outputs[0],uv.inputs['X']);links.new(y.outputs[0],uv.inputs['Y'])
    textures=[]
    for image in(wood,phase):
        texture=nodes.new('ShaderNodeTexImage');texture.image=image;texture.interpolation='Closest';texture.extension='CLIP';links.new(uv.outputs[0],texture.inputs['Vector']);textures.append(texture)
    lightpath=nodes.new('ShaderNodeLightPath')
    camera_mask=nodes.new('ShaderNodeMath');camera_mask.operation='MULTIPLY';links.new(textures[0].outputs['Alpha'],camera_mask.inputs[0]);links.new(lightpath.outputs['Is Camera Ray'],camera_mask.inputs[1])
    state=nodes.new('ShaderNodeValue');state.name='Trap applied appearance state';state.outputs[0].default_value=0
    active=nodes.new('ShaderNodeMath');active.operation='MULTIPLY';links.new(camera_mask.outputs[0],active.inputs[0]);links.new(state.outputs[0],active.inputs[1])
    transparent=nodes.new('ShaderNodeBsdfTransparent');native=nodes.new('ShaderNodeEmission');links.new(textures[1].outputs['Color'],native.inputs['Color'])
    overlay=nodes.new('ShaderNodeMixShader');links.new(textures[1].outputs['Alpha'],overlay.inputs[0]);links.new(transparent.outputs[0],overlay.inputs[1]);links.new(native.outputs[0],overlay.inputs[2])
    result=nodes.new('ShaderNodeMixShader');links.new(active.outputs[0],result.inputs[0]);links.new(original_surface,result.inputs[1]);links.new(overlay.outputs[0],result.inputs[2]);links.new(result.outputs[0],surface)
    return material,state,textures[1]


def main():
    base=OUT/(sys.argv[sys.argv.index('--candidate')+1] if '--candidate' in sys.argv else 'log-trap-state-candidate-v12');support=json.loads((base/'dense-contact-audit.json').read_text());assert support['status'].startswith('sampled support pass');assert sha(base/'worker.blend')==support['model_sha256']
    source=OUT/'state-target-evidence/log-trap';source_manifest=json.loads((source/'manifest.json').read_text());box=source_manifest['bbox']
    reference=source/'native-order-reference';reference_hash=sha(reference/'manifest.json')
    animation=next(r for r in json.loads((OUT/'animation-references/manifest.json').read_text())['animations']if r['index']==2)
    dest=OUT/(sys.argv[sys.argv.index('--output')+1] if '--output' in sys.argv else 'log-state-appearance-proof-v4');dest.mkdir(exist_ok=False);(dest/'renderer.py').write_bytes(Path(__file__).read_bytes())
    for phase in(0,6):crop_frame(animation['frames'][phase],box).save(dest/f'native-canopy-phase-{phase:02d}.png')
    selected=[];bindings=[]
    for index in(26,29,30):
        worker=tree_workspace(index);audit=json.loads((worker/'inspection/saved-model-audit.json').read_text());assert sha(worker/'model.blend')==audit['model_sha256'];selected.append((worker,[r['object']for r in audit['objects']]));bindings.append(dict(tree=index,worker=str(worker),model_sha256=audit['model_sha256']))
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'));scene=bpy.context.scene
        logs=[o for o in scene.objects if o.get('state_endpoint')];trees=[]
        for worker,names in selected:
            with bpy.data.libraries.load(str(worker/'model.blend'),link=False)as(src,dst):dst.objects=list(names)
            for obj in dst.objects:scene.collection.objects.link(obj);obj.hide_render=False;trees.append(obj)
        bpy.context.view_layer.update();before_hash=geometry_hash(logs+trees)
        target=point((box[0]+box[2])/2,(box[1]+box[3])/2,0);scene.camera.location=target+RAY*3000;scene.camera.rotation_euler=(target-scene.camera.location).to_track_quat('-Z','Y').to_euler();scene.camera.data.ortho_scale=max(box[2]-box[0],box[3]-box[1])
        scene.cycles.use_denoising=False;scene.render.use_compositing=False;scene.render.dither_intensity=0;scene.cycles.pixel_filter_type='BOX';scene.cycles.filter_width=.01;scene.cycles.samples=8;scene.cycles.seed=0;scene.cycles.use_adaptive_sampling=False;scene.cycles.transparent_max_bounces=256
        # Keep the importance-sampling strategy identical in both controls and states.
        # Emission shaders and their radiometric behavior are unchanged by this setting.
        for obj in logs+trees:
            for slot in obj.material_slots:
                if hasattr(slot.material.cycles,'emission_sampling'):slot.material.cycles.emission_sampling='NONE'
        def render(name,state):
            for obj in logs:obj.hide_render=obj['state_endpoint']!=state
            scene.render.filepath=str(dest/f'{name}.png');bpy.ops.render.render(write_still=True)
        render('initial-before','covered');render('applied-before','applied')
        wood=bpy.data.images.load(str(source/'tick-089.png'));phases={p:bpy.data.images.load(str(dest/f'native-canopy-phase-{p:02d}.png'))for p in(0,6)}
        altered=[];modified=[]
        for obj in trees:
            if obj.get('projection_component')!='crown':continue
            for slot in obj.material_slots:
                original=slot.material;material,state,texture=scoped_material(original,wood,phases[0],box);slot.material=material;modified.append((state,texture));altered.append(dict(object=obj.name,original_material=original.name,proof_material=material.name))
        assert altered,'No verified crown components selected'
        render('initial-after-state-off','covered')
        for state,texture in modified:state.outputs[0].default_value=1
        render('applied-phase-00','applied')
        for state,texture in modified:texture.image=phases[6]
        render('applied-phase-06','applied')
        for state,texture in modified:texture.image=phases[0]
        emission=bpy.data.materials.new('Visible wood diagnostic');emission.use_nodes=True;emission.cycles.emission_sampling='NONE';nodes=emission.node_tree.nodes;nodes.clear();output=nodes.new('ShaderNodeOutputMaterial');shader=nodes.new('ShaderNodeEmission');shader.inputs['Color'].default_value=(1,0,1,1);emission.node_tree.links.new(shader.outputs[0],output.inputs[0])
        for obj in logs:
            if obj['state_endpoint']=='applied':
                for slot in obj.material_slots:slot.material=emission
        render('applied-phase-00-visibility','applied')
        for obj in trees:obj.hide_render=True
        render('logs-only-visibility','applied')
        assert geometry_hash(logs+trees)==before_hash
        assert sha(base/'worker.blend')==support['model_sha256'];assert sha(reference/'manifest.json')==reference_hash
        for worker,_ in selected:assert sha(worker/'model.blend')==next(r['model_sha256']for r in bindings if r['worker']==str(worker))
        report=dict(status='private camera-ray appearance prototype; no scene, model, exporter or runtime changes',renderer_sha256=sha(dest/'renderer.py'),log_model_sha256=support['model_sha256'],trees=bindings,geometry_and_uv_hash_before_and_after=before_hash,reference_manifest_sha256=reference_hash,native_wood_rgba_sha256=sha(source/'tick-089.png'),phase_source_hashes={str(p):sha(Path(animation['frames'][p]['image']))for p in(0,6)},altered_materials=altered,camera=dict(bbox=box,ortho_scale=scene.camera.data.ortho_scale,resolution=[512,512],denoising=False,compositing=False,dither=0,filter='BOX',filter_width=.01,transparent_bounces=256,samples=8,mesh_emission_importance_sampling='NONE consistently before controls and across states'),limitations=['Only crown material camera-ray appearance inside exact native wood RGBA footprint is gated by applied state; all geometry, UVs and wood materials unchanged.','Initial state keeps original materials behavior; non-camera lighting and shadow rays keep original crown behavior.','Native canopy animation RGBA is retained above target wood in source draw order; this is not a global leaf alpha change.','This Blender shader proof is not a supported exported runtime implementation and is scoped to the native source camera.','Source geometry still has uncovered native wood pixels; this proof does not invent missing wood or complete motion.'])
        (dest/'manifest.json').write_text(json.dumps(report,indent=2)+'\n')
    finally:release()


if __name__=='__main__':main()
