"""Complete private Tree14 wood/crown source-role geometry proposal.

Wood topology is retained. A procedural source-mask reduction is used only for
geometry review; a regular known-source atlas bake is required before any fill
or export. Neighboring artwork is retained as separate comparison context.
"""
import hashlib
import json
import math
from pathlib import Path
import shutil
import sys

import numpy as np
from PIL import Image, ImageDraw
from scipy.ndimage import binary_dilation

ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/croisement01-refinement';R=BASE/'restart2'
OUT=R/'tree14-whole-tree-v1';ROLE=R/'tree14-whole-source-role-proposal-v1'
WOOD=R/'tree14-wood-v16-projected/assets/croisement01-tree-14/model.blend'
CROWN=R/'tree14-crown94-geometry-v6/model.blend'
WOOD_SHA='b3f49a3c8b6b1c7c0996c0da348ecfd5ae9b2b4aae8b497f52cd965baedf8e64'
CROWN_SHA='93b33e7ef5c18f117eaab93f1a512b0f34928ae7b5889c33a96648151af1bacd'


def sha(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def dump(path,value):path.write_text(json.dumps(value,indent=2)+'\n')


def sources():
    OUT.mkdir(exist_ok=True)
    assert not (OUT/'source-packet.json').exists() and not (OUT/'model.blend').exists()
    source_path=BASE/'baseline/covered.png';source=np.asarray(Image.open(source_path).convert('RGBA'))
    h,w=source.shape[:2]
    def full(path,origin):
        arr=np.asarray(Image.open(path).convert('L'))>0;x,y=origin;hh,ww=arr.shape
        result=np.zeros((h,w),bool);result[y:y+hh,x:x+ww]=arr;return result
    masks=json.loads((BASE/'baseline/masks/manifest.json').read_text())['masks']
    def native(index):
        row=next(row for row in masks if row['index']==index)
        return full(BASE/'baseline/masks'/row['png'],row['box_top_left'])
    wood=full(ROLE/'wood-domain.png',(689,122));foreground=full(ROLE/'foreground-domain.png',(689,122))
    n14=native(14);n94=native(94)
    foreign_wood=np.any([native(i) for i in (9,11,12,15,16)],axis=0)&~n14
    # Existing native wood roles supply neighboring context, never a leaf-only
    # claim for all pixels in the broad view mask. Their own refinement remains
    # separate. Border attribution is an explicit contextual hypothesis.
    known_leaf=n94&~wood&~foreign_wood
    context_wood=n94&foreign_wood
    context_foreground=foreground&~n94
    assert not np.any(known_leaf&wood)
    for name,mask in [('wood-role',wood),('crown-static-role',known_leaf),('neighbor-wood-context',context_wood),('neighbor-foreground-context',context_foreground)]:
        Image.fromarray(mask.astype(np.uint8)*255).save(OUT/(name+'.png'))
        rgba=source.copy();rgba[:,:,3]=mask*255;rgba[~mask]=0;Image.fromarray(rgba).save(OUT/(name+'-source.png'))
    old=json.loads((R/'tree14-native-crown-animation-source-v3/source-partition.json').read_text())
    covered=np.zeros((h,w),bool);lobes=[]
    for lobe in old['lobes']:
        index=lobe['index'];x0,y0,x1,y1=lobe['bbox_source']
        original=np.asarray(Image.open(lobe['unknown']).convert('RGBA'))[:,:,3]>0
        support=binary_dilation(original,iterations=2)
        alpha=known_leaf[y0:y1,x0:x1]&support
        rgba=source[y0:y1,x0:x1].copy();rgba[:,:,3]=alpha*255;rgba[~alpha]=0
        front=OUT/f'lobe-{index:02}-static.png';Image.fromarray(rgba).save(front)
        # Back/interior coverage is inferred from the undithered silhouette;
        # it is not a checkerboard of native FX samples.
        physical=n94[y0:y1,x0:x1]&support
        hidden=np.full((*physical.shape,4),105,np.uint8);hidden[:,:,3]=physical*255
        back=OUT/f'lobe-{index:02}-unknown.png';Image.fromarray(hidden).save(back)
        covered[y0:y1,x0:x1]|=alpha
        lobes.append(dict(index=index,bbox_source=lobe['bbox_source'],source=str(front),unknown=str(back),source_sha256=sha(front),unknown_sha256=sha(back)))
    missing=known_leaf&~covered
    extensions=[]
    for y,x in zip(*np.nonzero(missing)):
        candidates=[row for row in lobes if row['bbox_source'][0]<=x<row['bbox_source'][2] and row['bbox_source'][1]<=y<row['bbox_source'][3]]
        assert candidates, ('Static source beyond every frozen lobe',int(x),int(y))
        row=min(candidates,key=lambda row:(x-(row['bbox_source'][0]+row['bbox_source'][2])/2)**2+(y-(row['bbox_source'][1]+row['bbox_source'][3])/2)**2)
        xx,yy=x-row['bbox_source'][0],y-row['bbox_source'][1]
        arr=np.array(Image.open(row['source']).convert('RGBA'));arr[yy,xx]=source[y,x];Image.fromarray(arr).save(row['source'])
        arr=np.array(Image.open(row['unknown']).convert('RGBA'));arr[yy,xx,3]=255;Image.fromarray(arr).save(row['unknown'])
        row['source_sha256']=sha(row['source']);row['unknown_sha256']=sha(row['unknown']);covered[y,x]=True
        extensions.append(dict(pixel=[int(x),int(y)],lobe=row['index'],reason='Static undithered edge beyond the old FX-only support; source sample preserved at native coordinates'))
    assert np.array_equal(known_leaf,covered)
    proposal=json.loads((ROLE/'proposal.json').read_text())
    report=dict(status='COMPLETE_TREE_SOURCE_ROLE_PROPOSAL_NO_MODEL_APPROVAL',source_sha256=sha(source_path),wood_model_sha256=WOOD_SHA,crown_model_sha256=CROWN_SHA,
        source_rgb_sha256=sha(source_path),native_alpha_sha256=sha(OUT/'crown-static-role.png'),lobes=lobes,
        static_support_extensions=extensions,counts=dict(wood=int(wood.sum()),static_crown=int(known_leaf.sum()),neighbor_wood_context=int(context_wood.sum()),neighbor_foreground_context=int(context_foreground.sum()),missing_static_leaf=0),
        role_proposal_sha256=sha(ROLE/'proposal.json'),inferred_role_border_pixels=proposal['counts']['inferred_role_borders'],
        native_frames=old['frame_records'],native_loop_ticks=56,
        limitations=['1713 source-role border pixels are inferred; every native source color is retained.',
                    'Neighbor wood mask roles are comparison context, not projected leaf texture.',
                    '676 foreground fragments outside crown94 remain separate neighboring context.',
                    'Lower wood/root geometry is unchanged. Crown volume and unseen surfaces remain inferred.',
                    'Procedural wood-mask reduction requires a normal known-source atlas bake before synthesis/export.',
                    'Native fourteen-frame packed conservation proof is separate; editor animation integration is not claimed.'])
    dump(OUT/'source-packet.json',report);print(report['counts'],flush=True)


def build():
    import bpy
    from mathutils import Matrix
    sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender'),str(ROOT/'level-editor/blender/leicester')]
    from render_slots import acquire
    from foliage_trees import geometry_hash
    from render_multiview_asset import render
    from setup_map import fit_camera
    acquire(slots=2)
    assert shutil.disk_usage(ROOT).free>10*1024**3
    assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024>6*1024**3
    packet=json.loads((OUT/'source-packet.json').read_text())
    assert sha(WOOD)==WOOD_SHA and sha(CROWN)==CROWN_SHA
    assert not (OUT/'model.blend').exists()
    bpy.ops.wm.open_mainfile(filepath=str(WOOD))
    config=json.loads((WOOD.parent/'workspace.json').read_text())
    scene=bpy.data.scenes[config['scene_name']];bpy.context.window.scene=scene
    wood=next(o for o in scene.objects if o.get('source_node')=='scenery-tree14-wood')
    wood_hash=geometry_hash(wood);wood_matrix=[list(row) for row in wood.matrix_world]
    for obj in scene.objects:
        if obj.type=='MESH':obj.hide_render=True
    wood.hide_render=False
    mask_image=bpy.data.images.load(str(OUT/'wood-role.png'));mask_image.colorspace_settings.name='Non-Color';mask_image.pack()
    # Reduce the old observed domain only. Its existing baked first-hit/color
    # ownership is retained inside the new bark domain; backs remain unknown.
    for slot in wood.material_slots:
        original=slot.material;mat=original.copy();slot.material=mat
        assert mat.use_nodes
        nodes=mat.node_tree.nodes;links=mat.node_tree.links
        output=next(n for n in nodes if n.type=='OUTPUT_MATERIAL' and n.is_active_output)
        assert output.inputs['Surface'].is_linked
        old_shader=output.inputs['Surface'].links[0].from_socket
        geometry=nodes.new('ShaderNodeNewGeometry');separate=nodes.new('ShaderNodeSeparateXYZ');links.new(geometry.outputs['Position'],separate.inputs[0])
        u=nodes.new('ShaderNodeMath');u.operation='DIVIDE';links.new(separate.outputs['X'],u.inputs[0]);u.inputs[1].default_value=1408
        sy=nodes.new('ShaderNodeMath');sy.operation='MULTIPLY';links.new(separate.outputs['Y'],sy.inputs[0]);sy.inputs[1].default_value=math.sin(math.radians(35))/960
        sz=nodes.new('ShaderNodeMath');sz.operation='MULTIPLY';links.new(separate.outputs['Z'],sz.inputs[0]);sz.inputs[1].default_value=math.cos(math.radians(35))/960
        add=nodes.new('ShaderNodeMath');add.operation='ADD';links.new(sy.outputs[0],add.inputs[0]);links.new(sz.outputs[0],add.inputs[1])
        v=nodes.new('ShaderNodeMath');v.operation='ADD';links.new(add.outputs[0],v.inputs[0]);v.inputs[1].default_value=1
        uv=nodes.new('ShaderNodeCombineXYZ');links.new(u.outputs[0],uv.inputs['X']);links.new(v.outputs[0],uv.inputs['Y'])
        tex=nodes.new('ShaderNodeTexImage');tex.image=mask_image;tex.interpolation='Closest';tex.extension='CLIP';links.new(uv.outputs[0],tex.inputs['Vector'])
        neutral=nodes.new('ShaderNodeBsdfPrincipled');neutral.inputs['Base Color'].default_value=(.34,.34,.34,1);neutral.inputs['Roughness'].default_value=1
        mix=nodes.new('ShaderNodeMixShader');links.new(tex.outputs['Color'],mix.inputs[0]);links.new(neutral.outputs[0],mix.inputs[1]);links.new(old_shader,mix.inputs[2]);links.new(mix.outputs[0],output.inputs['Surface'])
        mat['source_role_reduction']=sha(OUT/'wood-role.png');mat['requires_known_source_atlas_rebake']=True
    names=['Tree14 Crown94 provisional','Tree14 Crown94 inferred interior']
    with bpy.data.libraries.load(str(CROWN),link=False) as (available,target):
        assert all(n in available.objects for n in names);target.objects=names
    crowns=[o for o in target.objects if o]
    for obj in crowns:scene.collection.objects.link(obj)
    bpy.context.view_layer.update()
    import re
    for obj in crowns:
        obj.hide_render=False;obj.hide_viewport=False;obj.hide_set(False);obj['asset_group']=config['asset_id']
        obj['ownership_status']='Complete static source roles; explicit neighboring wood excluded; source-role borders inferred'
        for slot in obj.material_slots:
            old=slot.material;mat=old.copy();slot.material=mat
            match=re.search(r'lobe(\d+)',old.name)
            index=int(match.group(1)) if match else int(old.name.rsplit(' ',1)[-1])
            lobe=packet['lobes'][index]
            observed=bool(mat.get('foliage_observed'))
            tex=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE')
            path=lobe['source'] if observed else lobe['unknown']
            tex.image=bpy.data.images.load(path,check_existing=True);tex.image.pack()
            mat['foliage_source_rgb_sha256']=packet['source_rgb_sha256'];mat['foliage_native_alpha_sha256']=packet['native_alpha_sha256']
    assert geometry_hash(wood)==wood_hash and [list(row) for row in wood.matrix_world]==wood_matrix
    objects=[wood]+crowns
    scene.world=bpy.data.worlds.new('Tree14 whole tree neutral review');scene.world.color=(.15,.15,.15)
    scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;scene.render.film_transparent=True
    saved=OUT/'model.blend';bpy.ops.wm.save_as_mainfile(filepath=str(saved))
    dump(OUT/'construction.json',dict(status='PRIVATE_GEOMETRY_SOURCE_ROLE_PROPOSAL_REQUIRES_SELF_REVIEW',model_sha256=sha(saved),wood_geometry_sha256=wood_hash,wood_geometry_unchanged=True,wood_matrix=wood_matrix,
        source_packet_sha256=sha(OUT/'source-packet.json'),objects=[o.name for o in objects],crown_geometry_hashes={o.name:geometry_hash(o) for o in crowns},no_synthesis=True,no_canonical_writes=True,requires_known_source_atlas_rebake=True))
    manifest=json.loads((WOOD.parent/'input/views.json').read_text())
    manifest['object_names']=[o.name for o in objects];manifest['render_object_names']=None
    camera_data=bpy.data.cameras.new('Tree14 full crown review camera');camera_data.type='ORTHO'
    camera=bpy.data.objects.new(camera_data.name,camera_data);scene.collection.objects.link(camera)
    for view in manifest['views']:
        view['crop']={'width':384,'height':384};camera.matrix_world=Matrix(view['camera_matrix_world'])
        fit_camera(camera,objects,1.,padding=1.15);view['camera_matrix_world']=[list(row) for row in camera.matrix_world];view['ortho_scale']=camera_data.ortho_scale
    path=OUT/'views.json';dump(path,manifest)
    render(path,OUT/'views',modes=('textured',),width=384)
    images=[]
    for i in range(8):
        rgba=Image.open(OUT/f'views/view-{i}-textured.png').convert('RGBA');arr=np.asarray(rgba)
        assert arr[:,:,:3].max()>20 and (arr[:,:,3]>0).any(), ('Empty review',i)
        images.append(rgba.convert('RGB'))
    width,height=images[0].size;sheet=Image.new('RGB',(width*4,height*2))
    for i,img in enumerate(images):sheet.paste(img,((i%4)*width,(i//4)*height))
    sheet.save(OUT/'material-eight.png')
    assert sha(WOOD)==WOOD_SHA and sha(CROWN)==CROWN_SHA and sha(saved)==json.loads((OUT/'construction.json').read_text())['model_sha256']
    dump(OUT/'render-complete.json',dict(model_sha256=sha(saved),original_camera_top_left=True,status='RENDERED_PENDING_VISUAL_AND_JOINT_SOURCE_AUDIT'))
    # Finish the previously rejected diagnostic views with correct grouping and
    # lighting, while this same bounded render lease is already held.
    sys.path.insert(0,str(Path(__file__).parent))
    import restart2_tree14_layered_comparison as diagnostic
    diagnostic.OUT=R/'tree14-layered-comparison-v2';diagnostic.views()


if __name__=='__main__':
    phase=sys.argv[sys.argv.index('--')+1] if '--' in sys.argv else sys.argv[1]
    {'sources':sources,'build':build}[phase]()
