"""Render the saved candidate materials using its frozen review cameras."""
import argparse
import json
import sys
from pathlib import Path
import bpy
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_slots import acquire,release
from evidence_io import sha
from render_multiview_asset import render

def render_workspace(workspace,width=256,release_slot=True):
    workspace=Path(workspace).resolve()
    acquire()
    model_hash=sha(workspace/'model.blend')
    bpy.ops.wm.open_mainfile(filepath=str(workspace/'model.blend'))
    scene=bpy.data.scenes['Croisement02 Refinement'];scene.render.engine='CYCLES';scene.cycles.samples=4;scene.cycles.transparent_max_bounces=64
    scene.world=bpy.data.worlds.new('Neutral actual-material inspection');scene.world.color=(.10,.10,.10)
    output=workspace/'inspection/actual-materials'
    if output.exists():
        i=1
        while output.with_name(f'actual-materials-archive-{i:03}').exists():i+=1
        output.rename(output.with_name(f'actual-materials-archive-{i:03}'))
    packet=json.loads((workspace/'modified/views.json').read_text())
    for view in packet['views']:
        view['crop']={'width':packet['tile_size'][0],'height':packet['tile_size'][1]}
    manifest=workspace/'inspection/actual-camera-manifest.json'
    manifest.write_text(json.dumps(packet,indent=2)+'\n')
    render(manifest,output,width=width)
    paths=[output/f'view-{i}-textured.png' for i in range(8)];images=[Image.open(p).convert('RGB') for p in paths]
    w,h=images[0].size;sheet=Image.new('RGB',(w*4,h*2))
    for i,image in enumerate(images):sheet.paste(image,((i%4)*w,(i//4)*h))
    sheet.save(output/'sheet.png')
    if sha(workspace/'model.blend')!=model_hash:raise RuntimeError('Model changed during actual-material inspection')
    (output/'evidence.json').write_text(json.dumps(dict(model_sha256=model_hash,sheet_sha256=sha(output/'sheet.png')),indent=2)+'\n')
    from opacity_bounds import measure
    crowns=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==workspace.name and o.get('projection_component')=='crown']
    if crowns:
        bounds=[measure(o) for o in crowns]
        (output/'opacity-bounds.json').write_text(json.dumps(dict(model_sha256=model_hash,crowns=bounds),indent=2)+'\n')
    from source_coverage import audit
    objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==workspace.name]
    audit(workspace,objects)
    print(output/'sheet.png')
    if release_slot:release()

def main():
    parser=argparse.ArgumentParser();parser.add_argument('workspace',type=Path);parser.add_argument('--width',type=int,default=256)
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    render_workspace(args.workspace,args.width)

if __name__=='__main__':main()
