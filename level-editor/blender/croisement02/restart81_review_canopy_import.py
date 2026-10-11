"""Independently import a private canopy derivative and render its saved poses."""
import sys
import argparse
import json
import hashlib
import shutil
from pathlib import Path
import bpy
import numpy as np
from mathutils import Matrix, Vector
from PIL import Image, ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--export',type=Path,required=True);parser.add_argument('--asset',required=True);parser.add_argument('--views',type=Path,required=True);parser.add_argument('--output',type=Path,required=True);parser.add_argument('--reuse-from',type=Path)
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);args.output.mkdir(exist_ok=False)
    memory={k:int(v.split()[0])*1024 for k,v in (line.split(':',1) for line in Path('/proc/meminfo').read_text().splitlines())}
    assert memory['MemAvailable']>=6*1024**3 and shutil.disk_usage(args.output).free>=10*1024**3
    report=json.loads(args.export.read_text());asset=next(a for a in report['assets'] if a['asset_id']==args.asset);model=Path(asset['model']['path']);assert sha(model)==asset['model']['sha256']
    bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.render.fps=25
    bpy.ops.import_scene.gltf(filepath=str(model))
    p=asset['placement']['transform'];offset=Vector((p['dx'],-p['dy']/np.sin(np.radians(35)),p['dz']))
    for o in list(scene.objects):
        if o.parent is None:o.location+=offset
    scene.frame_set(0);bpy.context.view_layer.update()
    meshes=[o for o in scene.objects if o.type=='MESH'];crowns=[o for o in meshes if o.data.shape_keys];wood=[o for o in meshes if not o.data.shape_keys]
    assert crowns and wood
    wood_basis={o.name:(np.array(o.matrix_world),np.array([v.co[:] for v in o.data.vertices])) for o in wood}
    phase_rows=[]
    for phase in range(15):
        scene.frame_set(phase*4);bpy.context.view_layer.update();errors=[]
        for o in crowns:
            keys=o.data.shape_keys.key_blocks;assert len(keys)==14
            expected=np.zeros(13)
            if phase%14:expected[phase%14-1]=1
            assert np.max(np.abs(np.array([k.value for k in keys[1:]])-expected))<1e-5
            obj=o.evaluated_get(bpy.context.evaluated_depsgraph_get());evaluated=obj.to_mesh()
            actual=np.array([v.co[:] for v in evaluated.vertices]);wanted=np.array([v.co[:] for v in keys[phase%14].data]);obj.to_mesh_clear();error=float(np.max(np.abs(actual-wanted)));assert error<1e-5;errors.append(error)
        for o in wood:
            matrix,vertices=wood_basis[o.name];assert np.array_equal(matrix,np.array(o.matrix_world));assert np.array_equal(vertices,np.array([v.co[:] for v in o.data.vertices]));assert o.animation_data is None
        phase_rows.append(dict(phase=phase,tick=phase*4,maximum_key_evaluation_error=max(errors)))
    scene.render.engine='CYCLES';scene.cycles.samples=12;scene.cycles.transparent_max_bounces=512;scene.render.resolution_x=384;scene.render.resolution_y=384;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard'
    camera_data=bpy.data.cameras.new('Saved source camera');camera_data.type='ORTHO';camera_data.clip_end=20000;camera=bpy.data.objects.new('Saved source camera',camera_data);scene.collection.objects.link(camera);scene.camera=camera
    views=json.loads(args.views.read_text())['views'];images=[];recovered=[]
    for phase,indices in [(0,[0]),(7,range(8)),(14,[0])]:
        scene.frame_set(phase*4)
        for index in indices:
            v=views[index];camera.matrix_world=Matrix(v['camera_matrix_world']);camera_data.ortho_scale=v['ortho_scale'];image=args.output/f'phase-{phase:02}-view-{index}.png'
            prior=args.reuse_from/image.name if args.reuse_from else None
            if prior and prior.exists():
                assert Image.open(prior).size==(384,384)
                shutil.copyfile(prior,image);recovered.append(dict(path=str(prior),sha256=sha(prior),scope='Recovered from the same owner interrupted render command; independent import/phase assertions rerun here.'))
            else:
                scene.render.filepath=str(image);bpy.ops.render.render(write_still=True)
            images.append(dict(phase=phase,view=index,path=str(image),sha256=sha(image)))
            (args.output/'capture-progress.json').write_text(json.dumps(dict(model_sha256=sha(model),views_sha256=sha(args.views),images=images,recovered=recovered),indent=2)+'\n')
    assert np.array_equal(np.asarray(Image.open(args.output/'phase-00-view-0.png')),np.asarray(Image.open(args.output/'phase-14-view-0.png')))
    sheet=Image.new('RGB',(1536,808),'#dddddd');draw=ImageDraw.Draw(sheet)
    for index in range(8):
        image=Image.open(args.output/f'phase-07-view-{index}.png');x=index%4*384;y=index//4*404;sheet.paste(image,(x,y+20),image);draw.text((x+4,y+3),f'Phase 7 / view {index}',fill='black')
    sheet.save(args.output/'eight-view.png')
    result=dict(status='SAVED_IMPORT_AND_15_POSES_PASS_VISUAL_REVIEW_PENDING',asset_id=args.asset,export_sha256=sha(args.export),model_sha256=sha(model),views_sha256=sha(args.views),phase_rows=phase_rows,recovered_captures=recovered,crown_meshes=len(crowns),static_wood_meshes=len(wood),native_loop_image_exact=True,images=images,limits=['Only this isolated derivative was imported; neighbor contacts/source mask ownership remain separate.','New inferred crown motion is unapproved.','Image view 0 reuses the asset own saved original-art camera.'])
    (args.output/'report.json').write_text(json.dumps(result,indent=2)+'\n')


if __name__=='__main__':
    acquire(2)
    try:main()
    finally:release()
