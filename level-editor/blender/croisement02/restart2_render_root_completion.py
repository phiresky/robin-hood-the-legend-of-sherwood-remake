"""Full-bounds eight-view and exact-source inspection of private root completions."""
import argparse,json,sys
from pathlib import Path
import bpy
from mathutils import Vector,Matrix
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from tree_geometry import SIN,COS,RAY
from render_slots import acquire,release
from evidence_io import sha,write_json
from render_multiview_asset import render


def main():
    parser=argparse.ArgumentParser();parser.add_argument('kind',choices=['logging','southwest']);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    logging=args.kind=='logging';trial=OUT/'restart2-vegetation'/('logging-contour-v6' if logging else 'southwest-contour-v2');prior=OUT/'restart2-vegetation'/('logging-branches-v5' if logging else 'southwest-branches-v1')
    digest=sha(trial/'model.blend');out=trial/'full-review';out.mkdir(exist_ok=False)
    bpy.ops.wm.open_mainfile(filepath=str(trial/'model.blend'));scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene
    packet=json.loads((prior/'cameras.json').read_text());asset=packet['asset_id'];objs=[o for o in scene.objects if o.type=='MESH' and o.get('asset_group')==asset];bpy.context.view_layer.update()
    points=[o.matrix_world@v.co for o in objs for v in o.data.vertices]
    for view in packet['views']:
        matrix=Matrix(view['camera_matrix_world']);inverse=matrix.inverted();local=[inverse@p for p in points];lo=[min(p[i] for p in local) for i in (0,1)];hi=[max(p[i] for p in local) for i in (0,1)]
        matrix.translation=matrix@Vector(((lo[0]+hi[0])/2,(lo[1]+hi[1])/2,0))
        view.update(camera_matrix_world=[list(r) for r in matrix],camera_location=list(matrix.translation),ortho_scale=max(hi[i]-lo[i] for i in (0,1))*1.25,crop=dict(width=512,height=512))
    packet['tile_size']=[512,512];write_json(out/'cameras.json',packet)
    scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=64
    render(out/'cameras.json',out/'actual',width=512)
    images=[Image.open(out/f'actual/view-{i}-textured.png').convert('RGB') for i in range(8)];w,h=images[0].size;sheet=Image.new('RGB',(4*w,2*h))
    for i,im in enumerate(images):sheet.paste(im,((i%4)*w,(i//4)*h))
    sheet.save(out/'actual/sheet.png')
    crop=json.loads((prior/'proposal.json').read_text())['source_crop'];left,top,right,bottom=crop
    data=bpy.data.cameras.new('Exact source');data.type='ORTHO';data.ortho_scale=right-left;data.clip_end=10000;camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);scene.camera=camera
    center=Vector(((left+right)/2,-(top+bottom)/2/SIN,0));camera.location=center+RAY*6000;camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler()
    scene.render.resolution_x=(right-left)*3;scene.render.resolution_y=(bottom-top)*3;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.color_mode='RGBA'
    for o in scene.objects:
        if o.type=='MESH':o.hide_render=o not in objs
    scene.render.filepath=str(out/'source.png');bpy.ops.render.render(write_still=True)
    native=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA').crop(crop).resize(((right-left)*3,(bottom-top)*3),Image.Resampling.NEAREST);native.save(out/'native-source.png');Image.alpha_composite(native,Image.open(out/'source.png').convert('RGBA')).save(out/'source-overlay.png')
    if sha(trial/'model.blend')!=digest:raise ValueError('Inspected model changed')
    write_json(out/'evidence.json',dict(model_sha256=digest,source_crop=crop,framing='All actual vertices,25percent margin; original source direction first; prior packets preserved',images={str(p.relative_to(out)):sha(p) for p in out.rglob('*.png')},camera_sha256=sha(out/'cameras.json'),status='Actual/native evidence awaiting independent review'))

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
