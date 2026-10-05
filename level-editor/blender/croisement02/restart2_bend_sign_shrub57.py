"""Private continuous source-ray bend with unchanged native projection and RGBA."""
import sys,json,math
from pathlib import Path
import bpy
import numpy as np
from mathutils import Vector
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import SIN,COS,RAY
from render_slots import acquire,release
from restart2_sign_neighbors import camera_to,render


def main(mode="smooth", version=1):
    dest=OUT/f'restart2-fence/shrub57-sign-bend-v{version}';dest.mkdir(exist_ok=False)
    prior=OUT/'restart2-fence/sign-fragment-bounds-v1/target-7.json'
    detail=json.loads(prior.read_text());source=Path(detail['inputs']['shrub-57']['worker'])/'model.blend'
    assert sha(source)==detail['inputs']['shrub-57']['model_sha256']
    constraints={}
    for row in detail['pixels']:
        for hit in row.get('blockers',[]):
            key=tuple(row['source_pixel']);constraints[key]=max(constraints.get(key,0),hit['required_retreat']+2)
    points=np.array(list(constraints))+0.5;demands=np.array(list(constraints.values()))
    bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene
    obj=bpy.data.objects['West Rock Foliage 57']
    for o in scene.objects:
        if o.type=='MESH':o.hide_render=o!=obj
    before=np.array([tuple(obj.matrix_world@v.co) for v in obj.data.vertices])
    uv_before=[tuple(x.uv) for x in obj.data.uv_layers.active.data]
    scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.use_denoising=False;scene.cycles.transparent_max_bounces=128
    scene.cycles.pixel_filter_type='BOX';scene.cycles.filter_width=.01;scene.cycles.seed=0;scene.cycles.use_adaptive_sampling=False
    scene.render.film_transparent=True;scene.render.use_compositing=False;scene.render.dither_intensity=0
    scene.render.resolution_x=384;scene.render.resolution_y=384;scene.render.resolution_percentage=100
    scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
    data=bpy.data.cameras.new('Private shrub depth review');data.type='ORTHO';data.sensor_fit='HORIZONTAL';data.ortho_scale=192;data.clip_end=20000
    camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);scene.camera=camera
    native_center=Vector((38,-284/SIN,0));camera_to(camera,native_center,RAY)
    old=render(scene,dest/'native-before.png')
    projected=np.column_stack((before[:,0],-SIN*before[:,1]-COS*before[:,2]))
    deformation=None
    if mode in ['rigid','rigid_refined']:
        subdivision=None
        if mode=='rigid_refined':
            from sign_refine_pairs import refine
            refinement_prior=json.loads((OUT/'restart2-fence/shrub57-sign-bend-v5/report.json').read_text())
            conflicts=[r['component'] for r in refinement_prior['deformation']['components'] if r['unresolved']]
            detail,subdivision=refine(obj,detail,conflicts)
            before=np.array([tuple(obj.matrix_world@v.co) for v in obj.data.vertices])
            projected=np.column_stack((before[:,0],-SIN*before[:,1]-COS*before[:,2]))
            uv_before=[tuple(x.uv) for x in obj.data.uv_layers.active.data]
        from sign_rigid_depth import bend
        deformation=bend(obj,detail)
        if subdivision:deformation['subdivision']=subdivision
        after=np.array([tuple(obj.matrix_world@v.co) for v in obj.data.vertices])
        shifts=np.array([deformation['maximum_shift']])
        reprojection=np.column_stack((after[:,0],-SIN*after[:,1]-COS*after[:,2]))
        projection_error=float(np.max(np.abs(reprojection-projected)))
        assert projection_error<.0002
        assert uv_before==[tuple(x.uv) for x in obj.data.uv_layers.active.data]
    elif mode=='piecewise':
        from sign_piecewise_depth import bend
        deformation=bend(obj)
        after=np.array([tuple(obj.matrix_world@v.co) for v in obj.data.vertices])
        shifts=np.array([deformation['maximum_shift']])
        projection_error=None
    elif mode=='smooth':
        shifts=[]
        for p,z in zip(projected,before[:,2]):
            distance=np.linalg.norm(points-p,axis=1)
            # A plateau covers complete microtriangles around each sampled sign ray.
            field=float(np.max(demands*np.exp(-(np.maximum(distance-5,0)/22)**2)))
            # All source rays use a continuous monotone depth deformation, rooted at Z0.5.
            shifts.append(field*np.clip((z-.5)/80,0,1))
        shifts=np.array(shifts);after=before-shifts[:,None]*np.array(RAY)
        reprojection=np.column_stack((after[:,0],-SIN*after[:,1]-COS*after[:,2]))
        assert np.max(np.abs(reprojection-projected))<.0001
        assert 1-SIN*float(demands.max())/80>0
        inverse=obj.matrix_world.inverted()
        for v,p in zip(obj.data.vertices,after):v.co=inverse@Vector(p)
        obj.data.update();bpy.context.view_layer.update()
        assert uv_before==[tuple(x.uv) for x in obj.data.uv_layers.active.data]
        projection_error=float(np.max(np.abs(reprojection-projected)))
    else:
        raise ValueError('Unknown deformation mode')
    bpy.context.view_layer.update()
    bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'))
    new=render(scene,dest/'native-after.png')
    a,b=np.array(old),np.array(new);opaque=(a[:,:,3]>127)|(b[:,:,3]>127)
    compare=Image.new('RGBA',(768,384),(60,60,60,255));compare.alpha_composite(old,(0,0));compare.alpha_composite(new,(384,0));compare.convert('RGB').save(dest/'source-before-after.png')
    center=Vector(((after[:,0].min()+after[:,0].max())/2,(after[:,1].min()+after[:,1].max())/2,(after[:,2].min()+after[:,2].max())/2))
    data.ortho_scale=250;sheet=Image.new('RGB',(4*384,2*408),(65,65,65))
    for i,angle in enumerate(np.arange(8)*math.pi/4):
        camera_to(camera,center,Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN)))
        pic=render(scene,dest/f'actual-{i}.png');sheet.paste(pic,(i%4*384,i//4*408),pic.getchannel('A'))
        ImageDraw.Draw(sheet).text((i%4*384+4,i//4*408+386),f'Actual saved materials {i}',fill='white')
    sheet.save(dest/'actual-eight.png')
    write_json(dest/'report.json',dict(status='Private local depth bend; fresh physical sign and ground checks pending',model_sha256=sha(dest/'model.blend'),source_model_sha256=sha(source),constraints_sha256=sha(prior),deformation=deformation,vertex_count=len(after),maximum_shift=float(shifts.max()),source_projection_max_error=projection_error,uv_unchanged=(mode in ["smooth","rigid","rigid_refined"]),source_projection_contract="Ray-preserving deformation; piecewise mode interpolates UV at new face cuts",native_raster=dict(alpha_changed_pixels=int(np.count_nonzero(a[:,:,3]!=b[:,:,3])),opaque_rgb_changed_pixels=int(np.count_nonzero(np.any(a[:,:,:3]!=b[:,:,:3],axis=2)&opaque)),maximum_channel_error=int(np.max(np.abs(a.astype(int)-b.astype(int))))),ground_fringe_fixed="Depth map is identity at and below Z0.5",bounds_before=[before.min(0).tolist(),before.max(0).tolist()],bounds_after=[after.min(0).tolist(),after.max(0).tolist()],minimum_source_ray_jacobian=(deformation.get('minimum_source_ray_jacobian') if deformation else 1-SIN*float(demands.max())/80),method=(deformation['method'] if deformation else 'Maximum of smooth radial source-screen depth envelopes; same continuous field applies to every front/back/interior vertex, tapering linearly to zero at grounded Z0.5. Positive source-ray Jacobian prevents inversion of depth order on one ray.'),limitations=['Four-pose constraints only; all32 sign poses need validation.','Grounded fringe and source projection preserved; nearby bank/rock intersections require separate check.','Raw card bounds include transparent margins.','New geometry has no inherited user approval; selector remains unchanged.']))
    assert sha(source)==detail['inputs']['shrub-57']['model_sha256']
    print(dest)
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
