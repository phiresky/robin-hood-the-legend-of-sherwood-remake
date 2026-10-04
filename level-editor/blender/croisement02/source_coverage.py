"""Independent raster comparison of saved tree geometry against its source masks."""
import json
import math
from pathlib import Path
import bpy
import numpy as np
from mathutils import Vector
from PIL import Image
from catalog import OUT
from evidence_io import sha
from tree_geometry import SIN,COS,RAY


def audit(workspace,objects,transparent_bounces=64):
    if not isinstance(transparent_bounces,int) or not 1<=transparent_bounces<=1024:
        raise ValueError("Transparent bounce budget must be an integer in1..1024")
    report=json.loads((workspace/'inspection/refinement.json').read_text())
    if 'mask' not in report:return None
    if 'source_packet' in report:packet_path=Path(report['source_packet'])
    else:packet_path=Path(next(r['packet'] for r in json.loads((OUT/'forest-v4-sources/manifest.json').read_text()) if r['mask']==report['mask']))
    packet=json.loads(packet_path.read_text());expected=np.zeros((1152,1792),dtype=bool)
    def paste(alpha,x,y):
        h,w=alpha.shape;left,top=max(0,x),max(0,y);right,bottom=min(1792,x+w),min(1152,y+h)
        if right>left and bottom>top:expected[top:bottom,left:right]|=alpha[top-y:bottom-y,left-x:right-x]
    x,y,_,_=packet['native_bbox'];paste(np.asarray(Image.open(packet_path.parent/'complete-source.png'))[:,:,3]>127,x,y)
    native=next(r for r in json.loads((OUT/'baseline/masks/manifest.json').read_text())['masks'] if r['index']==report['mask'])
    if 'wood_domain_mask' in report:
        cfg=json.loads((workspace/'workspace.json').read_text())
        masks=json.loads(Path(cfg['source_mask_manifest']).read_text())
        inventory=Path(masks['mask_inventory'])
        native=next(r for r in json.loads(inventory.read_text())['masks'] if r['index']==report['wood_domain_mask'])
        native=dict(native,png=str((inventory.parent/native['png']).resolve()))
    paste(np.asarray(Image.open(OUT/'baseline/masks'/native['png']).convert('L'))>0,*native['box_top_left'])
    physical_authority=packet.get('physical_silhouette_authority')
    if physical_authority:
        if 'coverage_domain_mask' in report:raise ValueError('Competing physical and ownership coverage authorities')
        path=packet_path.parent/'complete-source.png'
        if sha(path)!=physical_authority['sha256'] or not physical_authority.get('reason'):
            raise ValueError('Physical silhouette proof is stale or unexplained')
        expected[:]=False
        paste(np.asarray(Image.open(path).convert('RGBA'))[:,:,3]>127,x,y)
    observed_expected=None
    observed_path=packet_path.parent/'observed-source.png'
    if observed_path.exists():
        saved=expected.copy();expected[:]=False
        paste(np.asarray(Image.open(observed_path).convert('RGBA'))[:,:,3]>127,x,y)
        observed_expected=expected.copy();expected[:]=saved
    coverage_domain=None
    if 'coverage_domain_mask' in report:
        # A mixed native mask can contain another receiver's rock or bark.
        # Its explicitly reviewed authored domain replaces the raw union;
        # inferred front pixels outside this domain remain measurable extras.
        cfg=json.loads((workspace/'workspace.json').read_text())
        mask_path=Path(cfg['source_mask_manifest']);masks=json.loads(mask_path.read_text())
        inventory_path=Path(masks['mask_inventory']);inventory=json.loads(inventory_path.read_text())
        index=report['coverage_domain_mask'];part_ids=set(cfg['part_ids'])
        assignments=masks['projections']['exterior']['assignments']
        if not any(index in a.get('mask_indices',[]) and (a.get('source_node') in part_ids or a.get('asset_group')==cfg['asset_id']) for a in assignments):
            raise ValueError('Coverage domain is not assigned to this worker')
        domain=next(r for r in inventory['masks'] if r['index']==index)
        path=(inventory_path.parent/domain['png']).resolve()
        expected[:]=False
        paste(np.asarray(Image.open(path).convert('L'))>0,*domain['box_top_left'])
        coverage_domain=dict(index=index,png=str(path),png_sha256=sha(path),inventory_sha256=sha(inventory_path),
                             source_manifest_sha256=sha(mask_path),semantics='Exact authored domain replaces raw native and complete-source union; extra inferred silhouette remains reported')
    yy,xx=np.nonzero(expected);left=max(0,int(xx.min())-10);right=min(1792,int(xx.max())+11);top=max(0,int(yy.min())-10);bottom=min(1152,int(yy.max())+11)
    width,height=right-left,bottom-top;expected=expected[top:bottom,left:right]
    scene=bpy.data.scenes.new('Isolated source coverage audit');copies=[]
    for obj in objects:
        copy=obj.copy();copy.parent=None;copy.matrix_world=obj.matrix_world.copy();copy.hide_render=False
        scene.collection.objects.link(copy);copies.append(copy)
    target=Vector(((left+right)/2,-(top+bottom)/2/SIN,0));data=bpy.data.cameras.new('Exact source coverage')
    data.type='ORTHO';data.sensor_fit='HORIZONTAL';data.ortho_scale=width;data.clip_end=20000
    camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);camera.location=target+RAY*5000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();scene.camera=camera
    scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=transparent_bounces
    scene.render.resolution_x=width;scene.render.resolution_y=height;scene.render.resolution_percentage=100
    scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA'
    scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
    destination=workspace/'inspection/source-coverage';destination.mkdir(exist_ok=True);scene.render.filepath=str(destination/'render.png')
    bpy.ops.render.render(write_still=True,scene=scene.name)
    actual=np.asarray(Image.open(destination/'render.png').convert('RGBA'))[:,:,3]>127
    missing=expected&~actual;extra=actual&~expected;intersection=expected&actual
    source=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGB').crop((left,top,right,bottom));source.save(destination/'source.png')
    overlay=np.asarray(source).copy();overlay[missing]=[255,40,40];overlay[extra]=[0,220,255];Image.fromarray(overlay).save(destination/'difference.png')
    Image.fromarray(expected.astype('uint8')*255).save(destination/'expected.png')
    result=dict(model_sha256=sha(workspace/'model.blend'),source_packet_sha256=sha(packet_path),source_crop=[left,top,right,bottom],expected_pixels=int(expected.sum()),rendered_pixels=int(actual.sum()),missing_pixels=int(missing.sum()),extra_pixels=int(extra.sum()),intersection_over_union=float(intersection.sum()/np.count_nonzero(expected|actual)),legend='Red: native coverage missed. Cyan: rendered coverage outside assigned native masks. Crossed foliage edges and mask-derived wood thickness can differ.',status='measurement; requires visual review')
    result['render_config']=dict(engine='CYCLES',samples=8,transparent_max_bounces=transparent_bounces)
    if coverage_domain is not None:result['coverage_domain']=coverage_domain
    if physical_authority:result['physical_silhouette_authority']=physical_authority
    if observed_expected is not None:
        observed=observed_expected[top:bottom,left:right]
        result['observed_source_coverage']=dict(expected_pixels=int(observed.sum()),missing_pixels=int((observed&~actual).sum()),
            recall=float((observed&actual).sum()/observed.sum()) if observed.any() else None,
            source_rgba_sha256=sha(observed_path),semantics='Separate observed-source recall; inferred silhouette outside observed pixels is not source evidence')
    (destination/'report.json').write_text(json.dumps(result,indent=2)+'\n')
    for obj in copies+[camera]:bpy.data.objects.remove(obj,do_unlink=True)
    bpy.data.cameras.remove(data);bpy.data.scenes.remove(scene)
    return result
