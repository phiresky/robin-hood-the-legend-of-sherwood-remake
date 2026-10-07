"""Bounded, unchanged hidden-archer endpoints beside exact audited rock and bank."""
import sys,json,math,shutil
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector,Matrix
from PIL import Image,ImageDraw
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import RAY,SIN,COS
from render_slots import acquire,release
BASE=OUT/'restart14-hidden-archer';ROUND=BASE/'climbing-v17';DEST=ROUND/'rock-bank-contact-v1';CAP=32*1024**2
POLICY=OUT/'restart17-small-job-disk-policy.json'
def budget():
    used=sum(p.stat().st_size for p in ROUND.rglob('*') if p.is_file());assert used<=CAP
    assert shutil.disk_usage(ROUND).free>=8*1024**3+CAP-used,'Disk reserve below bounded lane floor'

def main():
    budget();assert not DEST.exists();assert sha(POLICY)=='4bb7da826f59dc05ca8bd7654babed5bde622a8b52d63a81e8098f302ced45e2';DEST.mkdir()
    extraction_path=BASE/'surface-v8/extraction.json';extraction=json.loads(extraction_path.read_text());source=Path(extraction['source']);assert sha(source)==extraction['source_sha256'];records=[]
    authority=OUT/'restart7-source-patch-delivery/contracts-v1/source-review-v1/manifest.json';references=json.loads(authority.read_text())['images']
    for state in ['initial','applied']:
        budget();worker=ROUND/f'profile-05-{state}';model=worker/'model.blend';construction=json.loads((worker/'construction.json').read_text());assert sha(model)==construction['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;plants=[o for o in scene.objects if o.type=='MESH'];assert len(plants)==1
        names=[r['name'] for r in extraction['records']]
        with bpy.data.libraries.load(str(source),link=False) as (src,dst):dst.objects=names
        context=[]
        for obj,row in zip(dst.objects,extraction['records']):
            assert obj is not None;scene.collection.objects.link(obj);obj.parent=None;obj.matrix_world=Matrix(row['matrix_world']);obj.hide_render=False;context.append(obj)
        bpy.context.view_layer.update()
        for obj,row in zip(context,extraction['records']):
            points=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);assert np.max(np.abs(points.min(0)-np.array(row['world_min'])))<.01;assert np.max(np.abs(points.max(0)-np.array(row['world_max'])))<.01
        scene.render.engine='CYCLES';scene.render.threads_mode='FIXED';scene.render.threads=2;scene.cycles.samples=8;scene.cycles.use_denoising=False;scene.cycles.transparent_max_bounces=256;scene.render.resolution_percentage=100;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.render.film_transparent=True;scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
        world=bpy.data.worlds.new('Neutral unchanged-context inspection');world.use_nodes=True;world.node_tree.nodes['Background'].inputs['Strength'].default_value=.7;scene.world=world
        cam=bpy.data.objects.new('Contact review camera',bpy.data.cameras.new('Contact review camera'));scene.collection.objects.link(cam);scene.camera=cam;cam.data.type='ORTHO';cam.data.clip_end=10000
        output=DEST/state;output.mkdir();views=[]
        def render(name,center,direction,scale,width=320,height=320):
            budget();cam.location=center+direction*4000;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();cam.data.ortho_scale=scale;scene.render.resolution_x=width;scene.render.resolution_y=height;path=output/(name+'.png');scene.render.filepath=str(path);bpy.ops.render.render(write_still=True);views.append(dict(image=path.name,sha256=sha(path),center=list(center),direction=list(direction),ortho_scale=scale,width=width,height=height))
        # Exact source crop includes18 pixels of inferred northern continuation.
        center=Vector((152,-42.5*SIN,-42.5*COS));render('native',center,RAY,121,240,242)
        points=np.array([o.matrix_world@v.co for o in plants for v in o.data.vertices]);low,high=points.min(0),points.max(0);center=Vector((low+high)/2);scale=float(np.linalg.norm(high-low))*1.08
        for i,a in enumerate([math.pi/4,3*math.pi/4,5*math.pi/4,7*math.pi/4]):render(f'oblique-{i}',center,Vector((math.sin(a)*.96,-math.cos(a)*.96,.28)).normalized(),scale)
        render('bank-roots',Vector((145,-192,50)),Vector((.6,-1,.3)).normalized(),95)
        ref=next(r for r in references if r['profile'].endswith('05') and r['label'].lower()==state);native=authority.parent/ref['image'];assert sha(native)==ref['sha256'];im=Image.open(native).convert('RGBA');old=Image.new('RGBA',(120,121));old.paste(im,(0,18));old=old.resize((240,242),Image.Resampling.NEAREST);new=Image.open(output/'native.png').convert('RGBA');board=Image.new('RGB',(480,270),'#444');draw=ImageDraw.Draw(board);board.paste(old,(0,28),old);board.paste(new,(240,28),new);draw.text((5,5),'Native composite; top beyond map',fill='white');draw.text((245,5),'Exact endpoint + rock/bank only',fill='white');budget();board.save(output/'source-comparison.png')
        sheet=Image.new('RGB',(960,680),'#444');draw=ImageDraw.Draw(sheet)
        for i,row in enumerate(views):
            im=Image.open(output/row['image']).convert('RGBA');im.thumbnail((320,320));x,y=(i%3)*320,(i//3)*340;sheet.paste(im,(x,y+20),im);draw.text((x+5,y+3),row['image'],fill='white')
        budget();sheet.save(output/'sheet.png');assert sha(model)==construction['model_sha256'];records.append(dict(state=state,model_sha256=construction['model_sha256'],views=views,sheet_sha256=sha(output/'sheet.png'),source_comparison_sha256=sha(output/'source-comparison.png')))
    assert sha(source)==extraction['source_sha256'];budget();write_json(DEST/'evidence.json',dict(status='PRIVATE_UNCHANGED_CONTEXT_REVIEW_PENDING',records=records,source=str(source),source_sha256=sha(source),extraction=str(extraction_path),extraction_sha256=sha(extraction_path),policy_sha256=sha(POLICY),model_saved=False,limitations=['Only audited rock part035 and bank part000 are displayed; surrounding crowns and all other map assets deliberately absent.','No new contact geometry, no source ownership changes, no texture generation.','Native comparison includes unrelated source objects absent from two-neighbour diagnostic; not complete scene parity.']))
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
