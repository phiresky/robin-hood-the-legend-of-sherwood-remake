"""Eight actual and solid views of private covered/applied closed boulder endpoints."""
import json,math,sys
from pathlib import Path
import bpy
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from catalog import OUT
from log_trap_state_candidate import sha
from render_slots import acquire,release

def main():
    base=Path(sys.argv[sys.argv.index('--candidate')+1]).resolve() if '--candidate' in sys.argv else OUT/'rock-trap-state-candidate-v10';isolated='--isolated-supplement' in sys.argv;dest=base/('endpoint-isolated-supplement'if isolated else 'endpoint-orbit');dest.mkdir(exist_ok=False);report=json.loads((base/'contact-audit.json').read_text());assert report['status'].startswith('surface clearance pass');assert sha(base/'worker.blend')==report['model_sha256'];acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'));scene=bpy.context.scene;scene.cycles.samples=12;scene.cycles.use_denoising=False;scene.render.resolution_x=scene.render.resolution_y=512
        if isolated:
            for obj in scene.objects:
                if obj.type=='MESH'and not obj.get('state_endpoint'):obj.hide_render=True
        solid=bpy.data.materials.new('Private endpoint solid review');solid.use_nodes=True;solid.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.25,.25,.25,1);records=[]
        for state in (['applied']if isolated else ['covered','applied']):
            logs=[o for o in scene.objects if o.get('state_endpoint')==state]
            for o in scene.objects:
                if o.get('state_endpoint'):o.hide_render=o not in logs
            points=[o.matrix_world@v.co for o in logs for v in o.data.vertices];low=Vector(tuple(min(p[i]for p in points)for i in range(3)));high=Vector(tuple(max(p[i]for p in points)for i in range(3)));center=(low+high)/2
            for view in ([6,7]if isolated else range(8)):
                angle=math.pi/2+view*math.pi/4;direction=Vector((math.cos(angle)*math.cos(math.radians(35)),math.sin(angle)*math.cos(math.radians(35)),math.sin(math.radians(35))));scene.camera.location=center+direction*3000;scene.camera.rotation_euler=(center-scene.camera.location).to_track_quat('-Z','Y').to_euler();scene.camera.data.ortho_scale=(high-low).length*1.15
                for mode in ['actual','solid']:
                    scene.view_layers[0].material_override=solid if mode=='solid'else None;path=dest/f'{state}-{view:02d}-{mode}.png';scene.render.filepath=str(path);bpy.ops.render.render(write_still=True);records.append(dict(state=state,view=view,mode=mode,image=path.name,sha256=sha(path),camera=list(scene.camera.location),ortho_scale=scene.camera.data.ortho_scale))
        assert sha(base/'worker.blend')==report['model_sha256'];(dest/'manifest.json').write_text(json.dumps(dict(status='private endpoint geometry review; no transition identity claim',model_sha256=report['model_sha256'],records=records),indent=2)+'\n')
    finally:release()
if __name__=='__main__':main()
