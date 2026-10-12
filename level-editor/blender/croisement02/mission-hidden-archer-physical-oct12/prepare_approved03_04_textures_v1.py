"""Derive protected texture inputs from the approved eight-view pose pixels."""
import hashlib,json,math,sys,time,shutil
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
import build_models as b
from build_support04_v1 import fingerprint
from render_slots import acquire,release
from mathutils import Vector
from mathutils.bvhtree import BVHTree
import numpy as np
from PIL import Image
from scipy.ndimage import binary_dilation
ROOT=b.q.ROOT; BASE=b.q.OUT
DEST=BASE/'approved03-04-textures-v1'
APPROVAL=ROOT/'level-editor/work/active/croisement02/restart3-review-batches/next23-review-hub-oct12-v1/user-approval.json'
PHASES=['initial','transition-0','transition-1','applied']

def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def available():return int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024

def preflight():
    user=json.loads(APPROVAL.read_text());assert user['status']=='approved' and user['card_count']==23
    cards={c['number']:c for c in user['cards']}
    for n in [3,4]:
        c=cards[n+19];assert c['decision']=='approved' and c['kind']=='geometry'
        assert c['card_id']==f'croisement02-hidden-archer{n:02}-foliage-geometry'
        for p in c['exact_source_scope']['inputs']:
            assert sha(ROOT/p['path'])==p['sha256'],p
    return user,cards

def main():
    user,cards=preflight();assert not (DEST/'prepared').exists();(DEST/'prepared').mkdir()
    rows=[]
    for n in [3,4]:
        card=cards[n+19];scope=card['exact_source_scope'];model=ROOT/scope['model']['path']
        b.bpy.ops.wm.open_mainfile(filepath=str(model));scene=b.bpy.context.scene
        objects=[o for o in scene.objects if o.type=='MESH'];before={o.name:fingerprint(o) for o in objects}
        for phase in PHASES:
            assert available()>=10*1024**3
            active=[o for o in objects if o['source_phase']==phase]
            assert active
            for o in objects:o.hide_render=o not in active;o.hide_viewport=o not in active
            b.bpy.context.view_layer.update()
            asset=f'croisement02-hidden-archer{n:02}-{phase}'
            worker=DEST/'prepared'/f'{n:02}-{phase}';views=worker/'modified/views';views.mkdir(parents=True)
            collection=b.bpy.data.collections.new(asset);scene.collection.children.link(collection)
            for o in active:collection.objects.link(o);o['asset_group']=asset;o['source_node']=asset
            b.bpy.context.preferences.filepaths.save_version=0
            b.bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'),compress=True)
            assert before=={o.name:fingerprint(o) for o in objects}
            vertices=[];triangles=[];observed=[];receivers={}
            for o in active:
                mesh=o.data;mesh.calc_loop_triangles();offset=len(vertices)
                vertices.extend(o.matrix_world@v.co for v in mesh.vertices)
                for t in mesh.loop_triangles:
                    triangles.append(tuple(offset+v for v in t.vertices))
                    observed.append(not bool(o.get('inferred_support')) and t.material_index==0)
                receivers[o.name]=[p.index for p in mesh.polygons if bool(o.get('inferred_support')) or p.material_index!=0]
            tree=BVHTree.FromPolygons(vertices,triangles,all_triangles=True)
            points=np.array(vertices);center=Vector((points.min(0)+points.max(0))/2)
            scale=float(np.linalg.norm(points.max(0)-points.min(0)))*1.12
            camera=b.bpy.data.objects.new('Texture ray camera',b.bpy.data.cameras.new('Texture ray camera'));scene.collection.objects.link(camera);camera.data.type='ORTHO';camera.data.ortho_scale=scale
            review=(BASE/'geometry-v9-short-support03-review-v1'/phase if n==3 else BASE/('geometry-v8-short-support04-review-v3-junctions' if phase in PHASES[:2] else 'geometry-v8-short-support04-review-v4-remaining')/phase)
            sheets={k:Image.new('RGBA',(1024,512)) for k in ['textured','solid']};records=[];proof=[]
            for i in range(8):
                angle=i*math.pi/4;direction=Vector((b.C*math.sin(angle),-b.C*math.cos(angle),b.S))
                camera.location=center+direction*4000;camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler();b.bpy.context.view_layer.update();matrix=camera.matrix_world.copy()
                occupied=np.zeros((256,256),bool);known=np.zeros((256,256),bool);unit=scale/256
                for y in range(256):
                    for x in range(256):
                        origin=matrix.translation+matrix.col[0].to_3d()*((x+.5-128)*unit)+matrix.col[1].to_3d()*((128-y-.5)*unit)
                        hit,normal,face,distance=tree.ray_cast(origin,-matrix.col[2].to_3d())
                        if hit is not None:occupied[y,x]=True;known[y,x]=observed[face]
                protected=binary_dilation(known,iterations=1);editable=occupied&~protected
                original=np.array(Image.open(review/f'textured-{i}.png').convert('RGBA'))
                assert original.shape==(256,256,4)
                # Keep the exact approved shaded material pixels, including unknown gray.
                solid=np.array(Image.open(review/f'solid-{i}.png').convert('RGBA'));solid[:,:,3]=occupied.astype(np.uint8)*255
                ownership=np.full((256,256,4),255,np.uint8);ownership[editable,:3]=0
                for kind,pixels in [('textured',original),('solid',solid),('known',ownership)]:Image.fromarray(pixels).save(views/f'view-{i}-{kind}.png')
                for kind,pixels in [('textured',original),('solid',solid)]:sheets[kind].paste(Image.fromarray(pixels),((i%4)*256,(i//4)*256))
                records.append(dict(index=i,azimuth_degrees=i*45,camera_matrix_world=[list(r) for r in matrix],camera_location=list(camera.location),camera_rotation_euler=list(camera.rotation_euler),ortho_scale=scale,crop=dict(left=0,top=0,width=256,height=256),ownership_sha256=sha(views/f'view-{i}-known.png')))
                proof.append(dict(view=i,known_first_hits=int(known.sum()),protected_pixels=int(protected.sum()),editable_pixels=int(editable.sum()),approved_textured_rgba_exact=True,source_textured_sha256=sha(review/f'textured-{i}.png'),source_solid_sha256=sha(review/f'solid-{i}.png')))
                print(n,phase,i,proof[-1],flush=True)
            for kind,image in sheets.items():image.save(worker/'modified'/f'{kind}.png')
            b.bpy.data.objects.remove(camera,do_unlink=True)
            qualification=json.loads((BASE/'qualification.json').read_text());source_frames=qualification['rows'][n-1]['art']['states'];frames=source_frames['initial']['frames']+source_frames['transition']['frames'];source=ROOT/frames[PHASES.index(phase)]['image']['path']
            evidence={str(APPROVAL):sha(APPROVAL),str(model):sha(model),str(BASE/'qualification.json'):sha(BASE/'qualification.json')}
            evidence.update({str(review/f'{kind}-{i}.png'):sha(review/f'{kind}-{i}.png') for kind in ['textured','solid'] for i in range(8)})
            manifest=dict(version=1,asset_id=asset,scene_name=scene.name,collection_name=collection.name,tile_size=[256,256],layout=dict(columns=4,rows=2),elevation_degrees=35,source_image=str(source),source_sha256=sha(source),source_blend=str(worker/'model.blend'),source_mask_evidence=evidence,object_names=[o.name for o in active],render_object_names=[o.name for o in active],texture_receiver_object_names=[o.name for o in active],texture_receiver_face_indices=receivers,review_state=phase,views=records,texture_material_suffix='archer-inferred-only-v1',projection_layers=[],known_rule='Exact saved-camera first hits on original observed front material are protected, dilated one output pixel. Only inferred face first hits editable; original material slots and UVs must remain exact during bake.',limitations=['Generation transport adds only bottom padding, never rescales the 256px approved views.','All four poses are discrete; texture approval does not assert smooth motion.'])
            write(worker/'modified/views.json',manifest)
            derivation=dict(status='APPROVED_GEOMETRY_TEXTURE_INPUT_DERIVATIVE',profile=n,phase=phase,source_model=scope['model'],prepared_model_sha256=sha(worker/'model.blend'),geometry_uv_material_indices_unchanged=True,geometry_fingerprints=before,source_user_decision=card,user_approval_sha256=sha(APPROVAL),views=proof,editable_pixels=sum(p['editable_pixels'] for p in proof),changes='Only state visibility, collection membership and private asset metadata. Reuses exact approved per-view pixels and reconstructed exact review cameras. Local mask excludes observed faces and adjacent pixel boundaries.')
            write(worker/'derivation.json',derivation);rows.append(dict(profile=n,phase=phase,worker=str(worker),model_sha256=sha(worker/'model.blend'),editable_pixels=derivation['editable_pixels']))
        assert sha(model)==scope['model']['sha256']
    write(DEST/'prepared-batch.json',dict(status='PREPARED_FOR_INPUT_REVIEW_NO_API',rows=rows,user_approval=str(APPROVAL),user_approval_sha256=sha(APPROVAL)))
    print('ARCHER_APPROVED_INPUT_PREPARATION_COMPLETE',flush=True)

if __name__=='__main__':
    preflight()
    while True:
        while available()<11*1024**3:print('Waiting for 11 GiB before archer preparation',flush=True);time.sleep(30)
        assert shutil.disk_usage(ROOT).free>=10*1024**3
        acquire(slots=2)
        try:
            if available()<10*1024**3:continue
            main();break
        finally:release()
