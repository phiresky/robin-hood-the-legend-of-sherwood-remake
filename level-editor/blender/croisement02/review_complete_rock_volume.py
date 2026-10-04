"""Supplement frozen comparisons with fitted views of complete inferred rocks."""
import argparse,json,sys
from pathlib import Path
import bpy
from mathutils import Matrix,Vector
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from evidence_io import sha,write_json
from render_slots import acquire,release
from render_multiview_asset import render


def main():
    parser=argparse.ArgumentParser();parser.add_argument('worker',type=Path);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);worker=args.worker.resolve();model_hash=sha(worker/'model.blend')
    destination=worker/'inspection/complete-volume';destination.mkdir(exist_ok=False);acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));bpy.context.view_layer.update()
        scene=bpy.data.scenes['Croisement02 Refinement'];scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=64
        scene.world=bpy.data.worlds.new('Complete rock volume neutral environment');scene.world.color=(.10,.10,.10)
        points=[obj.matrix_world @ v.co for obj in bpy.data.collections['Croisement02 Working'].all_objects if obj.type=='MESH' and obj.get('asset_group')==worker.name for v in obj.data.vertices]
        packet=json.loads((worker/'modified/views.json').read_text());evidence=[]
        for view in packet['views']:
            matrix=Matrix(view['camera_matrix_world']);inverse=matrix.inverted();local=[inverse @ p for p in points]
            xs=[p.x for p in local];ys=[p.y for p in local];cx=(min(xs)+max(xs))/2;cy=(min(ys)+max(ys))/2
            matrix.translation+=matrix.to_3x3() @ Vector((cx,cy,0))
            view['camera_matrix_world']=[list(row) for row in matrix];view['camera_location']=list(matrix.translation)
            view['ortho_scale']=max(max(xs)-min(xs),max(ys)-min(ys))*1.18;view['crop']={'width':512,'height':512}
            evidence.append(dict(index=view['index'],camera_matrix_world=view['camera_matrix_world'],ortho_scale=view['ortho_scale'],projected_width=max(xs)-min(xs),projected_height=max(ys)-min(ys)))
        packet['tile_size']=[512,512];packet['framing']='Supplemental complete-volume fit; fixed comparison cameras retained separately'
        manifest=destination/'views.json';write_json(manifest,packet);render(manifest,destination,modes=('textured','solid'),width=512)
        for mode in ['textured','solid']:
            images=[Image.open(destination/f'view-{i}-{mode}.png').convert('RGB') for i in range(8)];w,h=images[0].size;sheet=Image.new('RGB',(4*w,2*h))
            for i,im in enumerate(images):sheet.paste(im,((i%4)*w,(i//4)*h))
            sheet.save(destination/f'{mode}-sheet.png')
        if sha(worker/'model.blend')!=model_hash:raise ValueError('Supplemental review altered saved worker')
        write_json(destination/'evidence.json',dict(model_sha256=model_hash,fixed_manifest_sha256=sha(worker/'modified/views.json'),fitted_manifest_sha256=sha(manifest),actual_sheet_sha256=sha(destination/'textured-sheet.png'),solid_sheet_sha256=sha(destination/'solid-sheet.png'),cameras=evidence,method='Same eight camera orientations as frozen comparison; recentered per view and fitted with18% margin around every mesh vertex. Saved model and original comparison evidence unchanged.'))
        print(destination)
    finally:release()

if __name__=='__main__':main()
