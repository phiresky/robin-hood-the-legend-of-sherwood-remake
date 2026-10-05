"""Private additive front-wall hypothesis from native source; preserve approved shed."""
import json,sys,math
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY
from scenery_geometry import Mesh
from refinement_workspace import _geometry
from render_multiview_asset import render
from refinement_review import _tree
from audit_scene_first_hit import full_mask


def main():
    asset='croisement02-woodcutters-shed';old=OUT/'texture-fill-round-1'/asset/'experiment-retry-roof/bake-v1/worker.blend'
    expected='5ec1a62d56bca7efdc45e5ca2dff31f15bb3519f6f563fdef13dc63f460aca6b';assert sha(old)==expected
    worker=OUT/'scenery-round-1/assets'/asset;dest=OUT/'restart2-vegetation/shed-front-v6';dest.mkdir(exist_ok=False)
    bpy.ops.wm.open_mainfile(filepath=str(old));bpy.context.preferences.filepaths.save_version=0
    scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene
    objects=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')==asset];assert len(objects)==2
    before={o.name:_geometry(o,protect_appearance=True)for o in objects}
    mask_cfg=json.loads((worker/'source-masks.json').read_text());ip=Path(mask_cfg['mask_inventory']);inv=json.loads(ip.read_text());lookup={r['index']:r for r in inv['masks']}
    assignment=next(r for r in mask_cfg['projections']['exterior']['assignments']if r.get('asset_group')==asset)
    domain=np.zeros((1152,1792),bool)
    domain|=full_mask(lookup[127],ip)
    domain&=~full_mask(lookup[110],ip)
    for index in assignment.get('exclude_mask_indices',[]):domain&=~full_mask(lookup[index],ip)
    source=np.array(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA'));source[:,:,3]=domain*255;Image.fromarray(source).save(dest/'front-source.png')
    mat=bpy.data.materials.new('Shed additive front native source');mat.use_nodes=True
    nodes=mat.node_tree.nodes;p=nodes.get('Principled BSDF');im=nodes.new('ShaderNodeTexImage');im.image=bpy.data.images.load(str(dest/'front-source.png'),check_existing=False);im.image.pack();im.interpolation='Closest';p.inputs['Base Color'].default_value=(0,0,0,1);mix=nodes.new('ShaderNodeMixRGB');mix.blend_type='MIX';mix.inputs[1].default_value=(.045,.025,.009,1);mat.node_tree.links.new(im.outputs['Alpha'],mix.inputs[0]);mat.node_tree.links.new(im.outputs['Color'],mix.inputs[2]);mat.node_tree.links.new(mix.outputs[0],p.inputs['Emission Color']);p.inputs['Emission Strength'].default_value=1;p.inputs['Roughness'].default_value=.9
    patch_path=dest/'own-native-board-supplement.png';Image.open(OUT/'animation-references/composite-frame-0.png').crop((1668,190,1680,210)).save(patch_path)
    supplement=nodes.new('ShaderNodeTexImage');supplement.image=bpy.data.images.load(str(patch_path),check_existing=False);supplement.image.pack();supplement.interpolation='Closest';supplement.extension='REPEAT';uvnode=nodes.new('ShaderNodeUVMap');uvnode.uv_map='Inferred board grain';mat.node_tree.links.new(uvnode.outputs['UV'],supplement.inputs['Vector']);mat.node_tree.links.new(supplement.outputs['Color'],mix.inputs[1])
    inferred=bpy.data.materials.new('Shed additive hidden board edges inferred dark wood');inferred.diffuse_color=(.095,.065,.027,1);inferred.use_nodes=True;inferred.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=inferred.diffuse_color
    raw=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text())['sight_obstacles'][138]['points'];points=[Vector((p['x'],-p['y']/SIN,p['z_top']/COS))for p in raw]
    a,b=points[1],points[2];axis=b-a;axis.z=0;length=axis.length;axis.normalize();inside=(points[0]+points[3]-points[1]-points[2]);inside.z=0;inside.normalize();up=Vector((0,0,1));m=Mesh();count=28
    # A source-visible recessed opening occupies only part of the facade.
    opening=(1704.,1719.);sill=28.;roof_height=min(a.z,b.z)-.7
    for j in range(count):
        t=(j+.5)/count;center=a.lerp(b,t);center.z=0;center-=inside*.2
        height=sill if opening[0]<center.x<opening[1] else roof_height
        m.box(center+up*height/2,axis,inside,length/count+.06,1.3,height)
    left_t=(a.x-opening[1])/(a.x-b.x);right_t=(a.x-opening[0])/(a.x-b.x);middle=a.lerp(b,(left_t+right_t)/2);middle.z=(sill+roof_height)/2
    m.box(middle+inside*6+up*2,axis,inside,length*(right_t-left_t),1.3,roof_height-sill+4)
    # Closed reveals preserve the source-visible recessed opening instead of a hole to ground.
    for t in [left_t,right_t]:
        side=a.lerp(b,t);side.z=(sill+roof_height)/2;m.box(side+inside*3,axis,inside,.7,6.8,roof_height-sill)
    ledge=a.lerp(b,(left_t+right_t)/2);ledge.z=sill;m.box(ledge+inside*3,axis,inside,length*(right_t-left_t),6.8,.7)
    mesh=bpy.data.meshes.new('Shed front boards and recessed opening');mesh.from_pydata(m.vertices,[],m.faces);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free();mesh.update();obj=bpy.data.objects.new(mesh.name,mesh);bpy.data.collections['Croisement02 Working'].objects.link(obj)
    for key,value in {'asset_group':asset,'asset_name':'Woodcutters Shed','source_node':'building-138','part_name':'Additive front wall','source_role':'Native127 prop pixels; hidden thickness and recess inferred'}.items():obj[key]=value
    mesh.materials.append(mat);mesh.materials.append(inferred);uv=mesh.uv_layers.new(name='Native front projection');grain=mesh.uv_layers.new(name='Inferred board grain');uvnode_native=nodes.new('ShaderNodeUVMap');uvnode_native.uv_map='Native front projection';mat.node_tree.links.new(uvnode_native.outputs['UV'],im.inputs['Vector'])
    for poly in mesh.polygons:
        poly.material_index=0 if poly.normal.dot(RAY)>.1 else 1
        for li in poly.loop_indices:
            v=mesh.vertices[mesh.loops[li].vertex_index].co;uv.data[li].uv=(v.x/1792,1-(-v.y*SIN-v.z*COS)/1152);grain.data[li].uv=(v.dot(axis)/15,v.z/30)
    assert before=={o.name:_geometry(o,protect_appearance=True)for o in objects}
    added_name=obj.name
    bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'),compress=True);digest=sha(dest/'model.blend')
    bpy.ops.wm.open_mainfile(filepath=str(dest/'model.blend'));assert before=={name:_geometry(bpy.data.objects[name],protect_appearance=True)for name in before}
    packet=json.loads((worker/'modified/views.json').read_text())
    for key in ['object_names','render_object_names']:
        if packet.get(key)is not None:packet[key].append(added_name)
    for view in packet['views']:view['crop']={'width':packet['tile_size'][0],'height':packet['tile_size'][1]}
    write_json(dest/'cameras.json',packet);scene=bpy.data.scenes['Croisement02 Refinement'];scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256
    render(dest/'cameras.json',dest/'actual',width=384)
    images=[Image.open(dest/f'actual/view-{i}-textured.png').convert('RGB')for i in range(8)];w,h=images[0].size;sheet=Image.new('RGB',(w*4,h*2))
    for i,image in enumerate(images):sheet.paste(image,((i%4)*w,(i//4)*h))
    sheet.save(dest/'actual/sheet.png')
    crop=[1610,90,1792,285];left,top,right,bottom=crop;data=bpy.data.cameras.new('Exact source');data.type='ORTHO';data.ortho_scale=right-left;data.clip_end=10000;camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);scene.camera=camera;center=Vector(((left+right)/2,-(top+bottom)/2/SIN,0));camera.location=center+RAY*6000;camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler();scene.render.resolution_x=(right-left)*3;scene.render.resolution_y=(bottom-top)*3;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.color_mode='RGBA'
    for o in scene.objects:
        if o.type=='MESH':o.hide_render=o.get('asset_group')!=asset
    scene.render.filepath=str(dest/'source.png');bpy.ops.render.render(write_still=True)
    native=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA').crop(crop).resize(((right-left)*3,(bottom-top)*3),Image.Resampling.NEAREST);native.save(dest/'native-source.png');Image.alpha_composite(native,Image.open(dest/'source.png').convert('RGBA')).save(dest/'source-overlay.png')
    visible=[o for o in scene.objects if o.type=='MESH'and not o.hide_render];tree,owners,_=_tree(visible);audit=json.loads((OUT/'restart2-vegetation/neutral-first-hit-v1/audit.json').read_text());samples=next(r['samples']for r in audit['components']if r['component']==3);hits=[]
    for row in samples:
        x,y=row['pixel'];hit,normal,index,distance=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000,-RAY);hits.append(dict(pixel=[x,y],object=owners[index].name if hit else None))
    write_json(dest/'proposal.json',dict(status='Private additive front-wall proposal; self/root/user review pending',model_sha256=digest,original_model=str(old),original_model_sha256=expected,original_geometry_uv_materials_preserved=True,original_objects=list(before),added_object=added_name,source_crop=crop,opening_source_x=list(opening),opening_sill_height=sill,inferred_recess_depth=6,former_ground_gap_pixels=len(hits),now_shed_first_hit=sum(r['object']is not None for r in hits),hits=hits,limitations=['Original front edge was omitted entirely; native source shows boarded facade and narrow recessed opening.','Door/window interpretation and hidden construction are inferred.','Existing approved roof/posts/sides/stump/textures unchanged; new geometry needs independent approval.','No ground or canonical source permission changed.','Front wall uses native127 excluding stump110 and foreground88. Source-hidden board portions use explicitly inferred repeated own-native board texture (source crop1668,190–1680,210); no stump cap is painted onto the wall.']))
    assert sha(old)==expected and sha(dest/'model.blend')==digest

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
