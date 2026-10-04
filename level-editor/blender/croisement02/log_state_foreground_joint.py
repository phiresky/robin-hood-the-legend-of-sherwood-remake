"""Measure actual source-camera log visibility behind selected native crown owners."""
import hashlib,json,sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from catalog import OUT,tree_workspace
from tree_geometry import RAY
from log_trap_state_candidate import point
from render_slots import acquire,release

def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def main():
    base=OUT/'log-trap-state-candidate-v10';dest=OUT/'log-state-foreground-joint-v3';dest.mkdir(exist_ok=False);bindings=[];selected=[]
    for index in (26,29,30):
        worker=tree_workspace(index);audit=json.loads((worker/'inspection/saved-model-audit.json').read_text());digest=sha(worker/'model.blend');assert audit['model_sha256']==digest
        selected.append((worker,[r['object']for r in audit['objects']]));bindings.append(dict(tree=index,worker=str(worker),model_sha256=digest,source_nodes=[r['source_node']for r in audit['objects']]))
    support=json.loads((base/'dense-contact-audit.json').read_text());assert support['status'].startswith('sampled support pass');assert support['model_sha256']==sha(base/'worker.blend')
    source=OUT/'state-target-evidence/log-trap';manifest=json.loads((source/'manifest.json').read_text());left,top,right,bottom=manifest['bbox'];scale=max(right-left,bottom-top)*1.2
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'));scene=bpy.context.scene;logs=[o for o in scene.objects if o.get('state_endpoint')=='applied']
        for o in scene.objects:
            if o.get('state_endpoint'):o.hide_render=o not in logs
        trees=[]
        for worker,names in selected:
            with bpy.data.libraries.load(str(worker/'model.blend'),link=False)as(src,dst):dst.objects=names
            for o in dst.objects:
                if o:scene.collection.objects.link(o);o.hide_render=False;trees.append(o)
        bpy.context.view_layer.update();camera=scene.camera;target=point((left+right)/2,(top+bottom)/2,0);camera.location=target+RAY*3000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();camera.data.ortho_scale=scale;scene.cycles.samples=16;scene.cycles.transparent_max_bounces=256
        for enabled,name in [(False,'logs-only-actual'),(True,'joint-actual')]:
            for o in trees:o.hide_render=not enabled
            scene.render.filepath=str(dest/f'{name}.png');bpy.ops.render.render(write_still=True)
        emission=bpy.data.materials.new('Diagnostic visible log emission');emission.use_nodes=True;nodes=emission.node_tree.nodes;nodes.clear();output=nodes.new('ShaderNodeOutputMaterial');shader=nodes.new('ShaderNodeEmission');shader.inputs['Color'].default_value=(1,0,1,1);shader.inputs['Strength'].default_value=1;emission.node_tree.links.new(shader.outputs[0],output.inputs[0])
        for o in logs:
            for slot in o.material_slots:slot.material=emission
        for enabled,name in [(False,'logs-only-visibility'),(True,'joint-visibility')]:
            for o in trees:o.hide_render=not enabled
            scene.render.filepath=str(dest/f'{name}.png');bpy.ops.render.render(write_still=True)
        masks=[]
        for name in ['logs-only-visibility','joint-visibility']:
            rgb=np.array(Image.open(dest/f'{name}.png'))[:,:,:3];masks.append((rgb[:,:,0]>240)&(rgb[:,:,1]<15)&(rgb[:,:,2]>240))
        body,visible=masks;yy,xx=np.mgrid[:512,:512];ix=np.floor((right-left)/2+(xx+.5-256)*scale/512).astype(int);iy=np.floor((bottom-top)/2+(yy+.5-256)*scale/512).astype(int);alpha=np.array(Image.open(source/'tick-089.png'))[:,:,3]>127;valid=(ix>=0)&(ix<alpha.shape[1])&(iy>=0)&(iy<alpha.shape[0]);expected=np.zeros((512,512),bool);expected[valid]=alpha[iy[valid],ix[valid]]
        rgb=np.zeros((512,512,3),np.uint8);rgb[expected&visible]=(60,190,80);rgb[expected&~visible]=(255,0,180);rgb[visible&~expected]=(255,155,0);rgb[body&~visible&~expected]=(40,90,200);Image.fromarray(rgb).save(dest/'source-interaction.png')
        report=dict(status='Unapproved joint diagnostic, not a completed visibility integration',log_model_sha256=sha(base/'worker.blend'),support_audit_sha256=sha(base/'dense-contact-audit.json'),trees=bindings,camera=dict(bbox=manifest['bbox'],ortho_scale=scale,resolution=[512,512],transparent_max_bounces=256),measurements=dict(native_pixels=int(expected.sum()),visible_log_pixels=int(visible.sum()),native_visible=int((expected&visible).sum()),native_covered_by_selected_trees=int((expected&body&~visible).sum()),inferred_continuation_hidden=int((body&~visible&~expected).sum()),unsupported_still_visible=int((visible&~expected).sum())),limitations=['All selected tree material alpha and physical geometry retained; only log diagnostic materials replaced in memory.','No source-mask clipping, alpha-island geometry, or model mutation.','Current tree crown repetition quality is a separate review issue; this checks exact source-camera interaction.','Log terrain support is bound to its continuous clearance audit; other foreground owners and temporal motion still require review.'])
        for worker,_ in selected:assert sha(worker/'model.blend')==next(r['model_sha256']for r in bindings if r['worker']==str(worker))
        (dest/'manifest.json').write_text(json.dumps(report,indent=2)+'\n');print(report['measurements'])
    finally:release()
if __name__=='__main__':main()
