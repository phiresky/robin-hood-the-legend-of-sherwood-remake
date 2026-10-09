"""Separate direct atlas ownership from lighting contamination in the palette test."""
import json,sys,hashlib,shutil
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_slots import acquire,release
import render_texture_coverage
R=ROOT/'level-editor/work/croisement01-refinement/restart2';O=R/'approved-tree02-fill-v1/croisement01-tree-02/baked-v4-bounded-bark-gaps'
assert shutil.disk_usage(R).free>=10*1024**3+4*1024**2;assert next(int(s.split()[1]) for s in Path('/proc/meminfo').read_text().splitlines() if s.startswith('MemAvailable:'))>=6*1024**2
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();before=sha(O/'worker.blend');acquire()
@bpy.app.handlers.persistent
def threads(scene,*_):scene.render.threads_mode='FIXED';scene.render.threads=2
bpy.app.handlers.render_pre.append(threads)
original_render=render_texture_coverage.render

def direct_palette(*args,**kwargs):
    count=0
    for material in bpy.data.materials:
        if not material.use_nodes:continue
        textures=[n for n in material.node_tree.nodes if n.type=='TEX_IMAGE' and n.image and n.image.name.startswith('Coverage diagnostic')]
        if not textures:continue
        assert len(textures)==1
        terminals=[n for n in material.node_tree.nodes if n.type=='OUTPUT_MATERIAL' and n.is_active_output];assert len(terminals)==1
        emission=material.node_tree.nodes.new('ShaderNodeEmission');emission.inputs['Strength'].default_value=1
        material.node_tree.links.new(textures[0].outputs['Color'],emission.inputs['Color']);material.node_tree.links.new(emission.outputs[0],terminals[0].inputs['Surface']);count+=1
    assert count>0
    # Opaque palette is a direct front-geometry diagnostic matching the CPU BVH,
    # not a replacement for the unchanged saved-material or alpha review.
    return original_render(*args,**kwargs)
render_texture_coverage.render=direct_palette
try:
 result=render_texture_coverage.inspect(O/'coverage-views-384.json',O,O/'coverage-unlit-v1-384');assert sha(O/'worker.blend')==before;result['scope']='Unlit opaque ownership palette isolates direct front-geometry atlas coverage; excludes reflected palette light. Physical alpha/source appearance remains checked separately in the saved-material review.';(O/'coverage-unlit-v1-384/scope.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result['views']),flush=True)
finally:release()
