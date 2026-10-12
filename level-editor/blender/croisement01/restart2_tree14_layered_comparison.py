"""Private source-layer diagnostic: preserve native art while testing receiver separation.

Pixel patches are diagnostic receivers, not approved crown geometry. Their depth
is measured on the frozen wood where available and explicitly inferred otherwise.
The source-space partition does not infer animated material identity from motion.
"""
import hashlib
import io
import json
import math
from pathlib import Path
import shutil
import sys

import bpy
import numpy as np
from PIL import Image, ImageDraw
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT/'level-editor/refinement'), str(ROOT/'level-editor/refinement/blender'), str(ROOT/'level-editor/blender/leicester')]
from render_slots import acquire
from foliage_trees import geometry_hash

BASE = ROOT/'level-editor/work/croisement01-refinement'
R = BASE/'restart2'
OUT = R/'tree14-layered-comparison-v1'
MODEL = R/'tree14-wood-v16-projected/assets/croisement01-tree-14/model.blend'
MODEL_SHA = 'b3f49a3c8b6b1c7c0996c0da348ecfd5ae9b2b4aae8b497f52cd965baedf8e64'
TRACE = R/'tree14-upper-material-trace-v5'
SIN, COS = math.sin(math.radians(35)), math.cos(math.radians(35))
RAY = Vector((0, -COS, SIN))
BOX = (645, 39, 985, 442)
X0,Y0,X1,Y1 = BOX
W,H = X1-X0,Y1-Y0
NAMES = ('accepted-bark', 'retained-base-unreviewed', 'foreground-candidate',
         'ambiguous-restored', 'unresolved-edge', 'unclassified-crown')
COLORS = ((70,205,240),(160,130,90),(235,90,225),(245,180,40),(245,65,50),(95,110,165))


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def dump(path, value):
    path.write_text(json.dumps(value, indent=2)+'\n')


def mask(path, origin):
    small = np.asarray(Image.open(path).convert('L')) > 0
    result = np.zeros((H,W), bool)
    x,y = origin; h,w = small.shape
    result[y-Y0:y-Y0+h, x-X0:x-X0+w] = small
    return result


def plane(x,y,depth):
    return Vector((float(x), -float(y)*SIN, -float(y)*COS))+RAY*float(depth)


def material(name, image):
    mat=bpy.data.materials.new(name);mat.use_nodes=True
    nodes=mat.node_tree.nodes;nodes.clear();links=mat.node_tree.links
    tex=nodes.new('ShaderNodeTexImage');tex.image=image;tex.interpolation='Closest';tex.extension='CLIP'
    uv=nodes.new('ShaderNodeUVMap');uv.uv_map='NativeSource'
    links.new(uv.outputs['UV'],tex.inputs['Vector'])
    emission=nodes.new('ShaderNodeEmission');links.new(tex.outputs['Color'],emission.inputs['Color'])
    transparent=nodes.new('ShaderNodeBsdfTransparent')
    mix=nodes.new('ShaderNodeMixShader');links.new(tex.outputs['Alpha'],mix.inputs[0])
    links.new(transparent.outputs[0],mix.inputs[1]);links.new(emission.outputs[0],mix.inputs[2])
    output=nodes.new('ShaderNodeOutputMaterial');links.new(mix.outputs[0],output.inputs[0])
    return mat


def prepare():
    assert not OUT.exists(), OUT
    assert sha(MODEL)==MODEL_SHA
    acquire(slots=2)
    assert shutil.disk_usage(ROOT).free>10*1024**3
    assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024>6*1024**3
    OUT.mkdir()
    trace_doc=json.loads((TRACE/'handoff.json').read_text())
    for filename,digest in trace_doc['resources'].items():
        assert sha(TRACE/filename)==digest
    dump(OUT/'private-trace-acceptance.json', dict(scope='Root private source refinement and comparison only; no user model approval',
        accepted_bark_pixels=621,foreground_comparison_pixels=237,trace_handoff_sha256=sha(TRACE/'handoff.json'),
        conditions=['All native RGBA retained.','No foliage ownership transfer or synthesis before joint reproduction.','All unresolved source stays explicit.']))
    source_path=BASE/'baseline/covered.png'
    assert sha(source_path)=='e6c41651511d5e593c2df079cccbfd4c5c272d79ae662943b1bea74cd6f97ec6'
    source=np.asarray(Image.open(source_path).convert('RGBA').crop(BOX)).copy()
    bark=mask(TRACE/'wood-candidate-proposal.png',(689,122))
    leaf=mask(TRACE/'foliage-candidate-proposal.png',(689,122))
    domain=mask(R/'tree14-source-restoration-proposal-v1/wood-domain.png',(689,122))
    restored=mask(R/'tree14-source-restoration-proposal-v1/restored-branch-domain.png',(689,122))
    native14=mask(BASE/'baseline/masks/000014.png',(689,122))
    native94=mask(BASE/'baseline/masks/000094.png',(649,43))
    packet=json.loads((R/'tree14-native-crown-animation-source-v3/source-partition.json').read_text())
    frames=[];union=np.zeros((H,W),bool)
    for frame in packet['frame_records']:
        path=ROOT/frame['path'];assert sha(path)==frame['sha256']
        rgba=np.asarray(Image.open(path).convert('RGBA'));x,y,w,h=frame['bbox']
        assert rgba.shape==(h,w,4) and set(np.unique(rgba[:,:,3])) <= {0,255}
        full=np.zeros_like(source);full[y-Y0:y-Y0+h,x-X0:x-X0+w]=rgba
        frames.append(full);union|=full[:,:,3]>0
    full_domain=native14|native94|union
    categories=[bark,(domain&~restored)&~bark&~leaf,leaf,restored&~bark&~leaf,native14&~domain]
    categories.append(full_domain&~np.any(categories,axis=0))
    assert np.all(np.sum(categories,axis=0)[full_domain]==1)
    assert [int(categories[i].sum()) for i in (0,2,4)]==[621,237,40]
    source[:,:,3]=full_domain*255
    source[~full_domain]=0
    Image.fromarray(source).save(OUT/'static-native-source.png')
    for index,rgba in enumerate(frames):Image.fromarray(rgba).save(OUT/f'native-frame-{index:02}.png')
    labeled=np.zeros_like(source)
    for name,color,selected in zip(NAMES,COLORS,categories):
        Image.fromarray(selected.astype(np.uint8)*255).save(OUT/f'{name}.png')
        labeled[selected]=(*color,255)
    Image.fromarray(labeled).save(OUT/'source-partition.png')
    bpy.ops.wm.open_mainfile(filepath=str(MODEL))
    wood=next(o for o in bpy.context.scene.objects if o.get('source_node')=='scenery-tree14-wood')
    protected=geometry_hash(wood);matrix=[list(row) for row in wood.matrix_world]
    tree=BVHTree.FromPolygons([wood.matrix_world@v.co for v in wood.data.vertices],[list(f.vertices) for f in wood.data.polygons])
    depths=np.full((H,W),np.nan);hits=np.zeros((H,W),bool)
    for y,x in zip(*np.nonzero(full_domain)):
        hit,_,_,_=tree.ray_cast(plane(X0+x+.5,Y0+y+.5,5000),-RAY,10000)
        if hit is not None:depths[y,x]=hit.dot(RAY);hits[y,x]=True
    assert np.all(hits[bark]), 'Accepted bark must remain source-supported by wood geometry.'
    # An explicit neutral comparison depth is used only for missing geometry.
    fallback=float(np.median(depths[hits]));depths[~hits]=fallback
    np.savez_compressed(OUT/'receiver-depths.npz',depth=depths,wood_hit=hits)
    for obj in bpy.context.scene.objects:
        if obj.type=='MESH':obj.hide_render=True
    wood.hide_render=False
    neutral=bpy.data.materials.new('Diagnostic retained wood neutral');neutral.diffuse_color=(.32,.32,.32,1)
    neutral.use_nodes=True;neutral.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.32,.32,.32,1)
    wood.data.materials.clear();wood.data.materials.append(neutral)
    for poly in wood.data.polygons:poly.material_index=0
    static_image=bpy.data.images.load(str(OUT/'static-native-source.png'));static_image.pack()
    frame_images=[]
    for index in range(14):
        image=bpy.data.images.load(str(OUT/f'native-frame-{index:02}.png'));image.pack();image.use_fake_user=True;frame_images.append(image)
    rows=[]
    for animated in (False,True):
        for index,(name,selected) in enumerate(zip(NAMES,categories)):
            selected=selected&(union if animated else full_domain)
            vertices=[];faces=[];uvs=[]
            # Foreground separation is a private depth hypothesis, not observed depth.
            offset=(.5 if index==2 else .25 if index>=3 else .05)+(.05 if animated else 0)
            for y,x in zip(*np.nonzero(selected)):
                base=len(vertices);depth=depths[y,x]+offset
                for xx,yy in ((x,y),(x+1,y),(x+1,y+1),(x,y+1)):
                    vertices.append(plane(X0+xx,Y0+yy,depth));uvs.append((xx/W,1-yy/H))
                faces.extend(((base,base+1,base+2),(base,base+2,base+3)))
            mesh=bpy.data.meshes.new(('FX ' if animated else 'Static ')+name);mesh.from_pydata(vertices,[],faces);mesh.update()
            uv=mesh.uv_layers.new(name='NativeSource')
            for loop in mesh.loops:uv.data[loop.index].uv=uvs[loop.vertex_index]
            obj=bpy.data.objects.new(mesh.name,mesh);bpy.context.scene.collection.objects.link(obj)
            obj['diagnostic_layer']=name;obj['animated_overlay']=animated;obj['inferred_depth_offset']=offset
            obj['source_ownership_status']='Private comparison only; source-space classes do not establish animated material identity'
            mesh.materials.append(material(mesh.name,frame_images[0] if animated else static_image))
            rows.append(dict(name=obj.name,layer=name,animated=animated,pixels=int(selected.sum()),wood_hit_pixels=int((selected&hits).sum()),inferred_depth_pixels=int((selected&~hits).sum())))
    assert geometry_hash(wood)==protected and [list(row) for row in wood.matrix_world]==matrix
    saved=OUT/'model.blend';bpy.ops.wm.save_as_mainfile(filepath=str(saved))
    dump(OUT/'preparation.json',dict(status='PRIVATE_LAYERED_DIAGNOSTIC_NOT_ASSET_APPROVAL',model_sha256=sha(saved),wood_model_sha256=MODEL_SHA,
        wood_geometry_sha256=protected,wood_transform=matrix,box=BOX,layers=rows,static_pixels=int(full_domain.sum()),
        counts={n:int(m.sum()) for n,m in zip(NAMES,categories)},frames=packet['frame_records'],fallback_depth=fallback,
        limitations=['Pixel patches are diagnostic receivers, not final continuous wood/leaf geometry.',
                    'Retained base ownership remains provisional; only621 new bark samples are privately accepted.',
                    'Animated overlay uses current source-position classes without claiming temporal material tracking.',
                    'Unclassified crown includes neighboring wood and foliage; no pure-foliage assumption.',
                    'Foreground offset and fallback depth are hypotheses, not source measurements.',
                    'Composite equality establishes conservation and alignment, not correct semantic ownership.']))
    print('PREPARED',OUT,flush=True)


def audit():
    acquire(slots=2)
    assert shutil.disk_usage(ROOT).free>10*1024**3
    assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024>6*1024**3
    packet=json.loads((OUT/'preparation.json').read_text());model=OUT/'model.blend'
    assert sha(model)==packet['model_sha256']
    bpy.ops.wm.open_mainfile(filepath=str(model))
    wood=next(o for o in bpy.context.scene.objects if o.get('source_node')=='scenery-tree14-wood')
    assert geometry_hash(wood)==packet['wood_geometry_sha256']
    wood_tree=BVHTree.FromPolygons([wood.matrix_world@v.co for v in wood.data.vertices],[list(f.vertices) for f in wood.data.polygons])
    layers=[]
    for obj in bpy.context.scene.objects:
        if 'diagnostic_layer' not in obj:continue
        image=next(n.image for n in obj.data.materials[0].node_tree.nodes if n.type=='TEX_IMAGE')
        assert image.packed_file and hashlib.sha256(bytes(image.packed_file.data)).hexdigest()==sha(OUT/('native-frame-00.png' if obj['animated_overlay'] else 'static-native-source.png'))
        tree=BVHTree.FromPolygons([obj.matrix_world@v.co for v in obj.data.vertices],[list(f.vertices) for f in obj.data.polygons],all_triangles=True)
        layers.append((obj,tree))
    for index in range(14):
        img=bpy.data.images.get(f'native-frame-{index:02}.png')
        assert img and img.packed_file and hashlib.sha256(bytes(img.packed_file.data)).hexdigest()==sha(OUT/f'native-frame-{index:02}.png')
    static=np.asarray(Image.open(OUT/'static-native-source.png').convert('RGBA'));domain=static[:,:,3]>0
    samples=[]
    for y,x in zip(*np.nonzero(domain)):
        origin=plane(X0+x+.5,Y0+y+.5,5000);intersections=[]
        for obj,tree in layers:
            point,_,face,distance=tree.ray_cast(origin,-RAY,10000)
            if point is None:continue
            poly=obj.data.polygons[face];uv=obj.data.uv_layers['NativeSource'].data
            coords=barycentric_transform(point,*[obj.matrix_world@obj.data.vertices[i].co for i in poly.vertices],*[Vector((*uv[i].uv,0)) for i in poly.loop_indices])
            tx=int(coords.x*W);ty=int((1-coords.y)*H)
            assert 0<=tx<W and 0<=ty<H
            intersections.append((distance,bool(obj['animated_overlay']),ty,tx,obj['diagnostic_layer']))
        point,_,_,wood_distance=wood_tree.ray_cast(origin,-RAY,10000)
        if point is not None:
            assert intersections and min(row[0] for row in intersections if not row[1])<wood_distance, (x,y,'wood occludes static receiver')
        samples.append((y,x,sorted(intersections)))
    counts=[];composites=[]
    for frame in [-1]+list(range(14)):
        expected_dynamic=np.zeros_like(static) if frame<0 else np.asarray(Image.open(OUT/f'native-frame-{frame:02}.png').convert('RGBA'))
        dynamic=np.zeros_like(static) if frame<0 else np.asarray(Image.open(io.BytesIO(bytes(bpy.data.images[f'native-frame-{frame:02}.png'].packed_file.data))).convert('RGBA'))
        for obj,_ in layers:
            if obj['animated_overlay'] and frame>=0:
                next(n for n in obj.data.materials[0].node_tree.nodes if n.type=='TEX_IMAGE').image=bpy.data.images[f'native-frame-{frame:02}.png']
        expected=static.copy();opaque=expected_dynamic[:,:,3]>0;expected[opaque]=expected_dynamic[opaque]
        actual=np.zeros_like(static);ownership=np.zeros_like(static);duplicates=0;missing=0
        for y,x,intersections in samples:
            statics=[row for row in intersections if not row[1]]
            duplicates+=max(0,len(statics)-1)
            for _,animated,ty,tx,name in intersections:
                rgba=(dynamic if animated else static)[ty,tx]
                if rgba[3]:actual[y,x]=rgba;ownership[y,x]=(*COLORS[NAMES.index(name)],255);break
            else:missing+=1
        mismatch=int(np.any(actual!=expected,axis=2).sum())
        record=dict(frame=frame,pixels=int(domain.sum()),missing=missing,duplicate_static_receivers=duplicates,rgba_mismatch=mismatch)
        assert mismatch==missing==duplicates==0,record
        counts.append(record);composites.append(actual)
        Image.fromarray(actual).save(OUT/('projected-static.png' if frame<0 else f'projected-frame-{frame:02}.png'))
        if frame in (-1,0,7,13):Image.fromarray(ownership).save(OUT/f'first-hit-layer-{frame}.png')
        dump(OUT/'audit-checkpoint.json',dict(model_sha256=packet['model_sha256'],completed=counts))
    board=Image.new('RGB',(W*4,H+25),'#292930');draw=ImageDraw.Draw(board)
    for index,(frame,label) in enumerate(((-1,'Static assembly'),(0,'Native frame 0'),(7,'Native frame 7'),(13,'Native frame 13'))):
        rgba=Image.fromarray(composites[frame+1]);board.paste(rgba,(index*W,25),rgba)
        draw.text((index*W+5,5),label,fill='white')
    board.save(OUT/'static-and-animation-projection.png')
    dump(OUT/'audit.json',dict(status='SOURCE_CONSERVATION_PASS_SEMANTIC_OWNERSHIP_STILL_HELD',model_sha256=packet['model_sha256'],frames=counts,
        method='Reopened saved mesh; BVH first-hit with actual UV barycentric sampling of source RGBA. Native clock4ticks/frame56loop.',
        limitations=packet['limitations']))
    assert sha(MODEL)==MODEL_SHA and sha(model)==packet['model_sha256']
    print('AUDIT PASS',len(counts),'states',flush=True)


def views():
    from render_multiview_asset import render
    from setup_map import fit_camera
    from mathutils import Matrix
    acquire(slots=2)
    assert shutil.disk_usage(ROOT).free>10*1024**3
    assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024>6*1024**3
    packet=json.loads((OUT/'preparation.json').read_text());model=OUT/'model.blend'
    assert sha(model)==packet['model_sha256']
    bpy.ops.wm.open_mainfile(filepath=str(model))
    manifest=json.loads((MODEL.parent/'input/views.json').read_text())
    scene=bpy.data.scenes[manifest['scene_name']];bpy.context.window.scene=scene
    objects=[o for o in bpy.data.objects if o.type=='MESH' and ('diagnostic_layer' in o or o.get('source_node')=='scenery-tree14-wood')]
    assert len(objects)==13
    for obj in objects:
        if obj.name not in scene.objects:scene.collection.objects.link(obj)
        obj.hide_render=False;obj.hide_viewport=False;obj.hide_set(False)
        obj['asset_group']=manifest['asset_id']
    scene.world=bpy.data.worlds.new('Layer diagnostic neutral review');scene.world.color=(.15,.15,.15)
    scene.render.film_transparent=True
    manifest['object_names']=[o.name for o in objects];manifest['render_object_names']=None
    data=bpy.data.cameras.new('Layer diagnostic camera');data.type='ORTHO'
    camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera)
    for view in manifest['views']:
        view['crop']={'width':384,'height':384}
        camera.matrix_world=Matrix(view['camera_matrix_world'])
        fit_camera(camera,objects,1.,padding=1.15)
        view['camera_matrix_world']=[list(row) for row in camera.matrix_world];view['ortho_scale']=data.ortho_scale
    path=OUT/'views.json';dump(path,manifest)
    scene.render.engine='CYCLES';scene.cycles.samples=4;scene.cycles.transparent_max_bounces=64
    render_dir=OUT/'saved-views-fixed-v1';assert not render_dir.exists()
    render(path,render_dir,modes=('textured',),width=256)
    images=[]
    for i in range(8):
        original=Image.open(render_dir/f'view-{i}-textured.png').convert('RGBA')
        arr=np.asarray(original);assert np.max(arr[:,:,:3])>20 and np.any(arr[:,:,3]>0), ('Empty camera review',i)
        images.append(original.convert('RGB'))
    w,h=images[0].size;sheet=Image.new('RGB',(w*4,h*2))
    for i,img in enumerate(images):sheet.paste(img,((i%4)*w,(i//4)*h))
    sheet.save(OUT/'saved-eight-fixed-v1.png')
    assert sha(model)==packet['model_sha256']
    dump(OUT/'views-complete.json',dict(model_sha256=sha(model),original_camera_top_left=True,diagnostic_only=True))


def bundle():
    global OUT
    acquire(slots=2)
    assert shutil.disk_usage(ROOT).free>10*1024**3+512*1024**2
    assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024>6*1024**3
    old=OUT;packet=json.loads((old/'preparation.json').read_text())
    assert sha(old/'model.blend')==packet['model_sha256']
    OUT=R/'tree14-layered-comparison-v2';assert not OUT.exists();OUT.mkdir()
    for path in old.iterdir():
        if path.is_file() and path.suffix in ('.png','.npz','.html'):
            shutil.copy2(path,OUT/path.name)
    shutil.copy2(old/'private-trace-acceptance.json',OUT/'private-trace-acceptance.json')
    bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'))
    for index in range(14):
        name=f'native-frame-{index:02}.png'
        image=bpy.data.images.get(name) or bpy.data.images.load(str(OUT/name))
        image.use_fake_user=True
        if not image.packed_file:image.pack()
        assert hashlib.sha256(bytes(image.packed_file.data)).hexdigest()==sha(OUT/name)
    bpy.context.scene['native_overlay_frames']=[f'native-frame-{i:02}.png' for i in range(14)]
    bpy.context.scene['native_overlay_frame_ticks']=4
    saved=OUT/'model.blend';bpy.ops.wm.save_as_mainfile(filepath=str(saved))
    packet['prior_model_sha256']=packet['model_sha256'];packet['model_sha256']=sha(saved)
    packet['revision_reason']='Retain all14 packed frames across save/reopen; v1 preserved only boundframe0.'
    dump(OUT/'preparation.json',packet)
    audit()
    views()


if __name__=='__main__':
    arguments=sys.argv[sys.argv.index('--')+1:]
    phase=arguments[0]
    if len(arguments)>1:
        assert phase in ('audit','views')
        OUT=Path(arguments[1]).resolve()
    {'prepare':prepare,'audit':audit,'views':views,'bundle':bundle}[phase]()
