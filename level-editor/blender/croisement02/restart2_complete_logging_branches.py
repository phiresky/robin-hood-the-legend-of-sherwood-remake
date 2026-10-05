"""Private native-traced branch volume completion; preserve approved fallen log."""
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
    asset='croisement02-logging-clearing-log';old=OUT/'texture-fill-round-1'/asset/'experiment/bake-v1/worker.blend'
    worker=OUT/'scenery-round-1/assets'/asset;dest=OUT/'restart2-vegetation/logging-branches-v5';dest.mkdir(exist_ok=False)
    expected='2d1c4ce16a22fc495bd7fec948dc1196ded35722ed0ed38fb44e7fce6c8c0832';assert sha(old)==expected
    bpy.ops.wm.open_mainfile(filepath=str(old));bpy.context.preferences.filepaths.save_version=0
    scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene
    objects=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')==asset];assert len(objects)==1
    before={o.name:_geometry(o,protect_appearance=True)for o in objects}
    mask_cfg=json.loads((worker/'source-masks.json').read_text());ip=Path(mask_cfg['mask_inventory']);inv=json.loads(ip.read_text());lookup={r['index']:r for r in inv['masks']}
    assignment=next(r for r in mask_cfg['projections']['exterior']['assignments']if r.get('asset_group')==asset)
    domain=np.zeros((1152,1792),bool)
    domain|=full_mask(lookup[108],ip)
    for index in assignment.get('exclude_mask_indices',[]):domain&=~full_mask(lookup[index],ip)
    source=np.array(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA'));source[:,:,3]=domain*255;Image.fromarray(source).save(dest/'front-source.png')
    mat=bpy.data.materials.new('Logging branches source wood');mat.use_nodes=True
    nodes=mat.node_tree.nodes;p=nodes.get('Principled BSDF');im=nodes.new('ShaderNodeTexImage');im.image=bpy.data.images.load(str(dest/'front-source.png'),check_existing=False);im.image.pack();im.interpolation='Closest';p.inputs['Base Color'].default_value=(0,0,0,1);mix=nodes.new('ShaderNodeMixRGB');mix.blend_type='MIX';mix.inputs[1].default_value=(.045,.025,.009,1);mat.node_tree.links.new(im.outputs['Alpha'],mix.inputs[0]);mat.node_tree.links.new(im.outputs['Color'],mix.inputs[2]);mat.node_tree.links.new(mix.outputs[0],p.inputs['Emission Color']);p.inputs['Emission Strength'].default_value=1;p.inputs['Roughness'].default_value=.9
    patch_path=dest/'own-native-board-supplement.png';Image.open(OUT/'animation-references/composite-frame-0.png').crop((1451,299,1462,307)).save(patch_path)
    supplement=nodes.new('ShaderNodeTexImage');supplement.image=bpy.data.images.load(str(patch_path),check_existing=False);supplement.image.pack();supplement.interpolation='Closest';supplement.extension='REPEAT';uvnode=nodes.new('ShaderNodeUVMap');uvnode.uv_map='Inferred board grain';mat.node_tree.links.new(uvnode.outputs['UV'],supplement.inputs['Vector']);mat.node_tree.links.new(supplement.outputs['Color'],mix.inputs[1])
    # Traced branch centerlines in native source coordinates. Heights and rear
    # thickness are inferred; each polyline remains a closed volume.
    paths=[
      [(1457,287,19,6),(1454,300,17,7),(1453,311,14,6),(1463,323,11,5),(1476,335,8,4),(1484,348,5,3),(1489,357,3,1.6)],
      [(1454,302,17,6),(1470,303,13,6),(1485,301,12,5),(1498,306,9,4),(1501,314,5,2.4)],
      [(1456,315,13,6),(1474,317,10,5),(1491,316,8,4),(1503,321,6,3),(1511,321,3,1.4)],
      [(1464,323,11,5),(1479,328,8,5),(1496,334,5,3),(1507,342,3,1.3)],
      [(1456,310,14,5),(1445,318,10,5),(1437,328,6,4),(1441,336,3,2)],
      [(1463,323,11,5),(1457,331,7,5),(1446,341,4,3),(1436,344,2,1.4)],
      [(1476,335,8,4),(1462,342,5,4),(1452,345,3,2)],
      [(1485,301,12,4),(1484,294,7,3),(1487,288,3,1.5)],
      [(1470,303,13,4),(1470,295,7,3),(1468,290,3,1.3)],
      [(1498,307,8,3),(1496,295,5,2),(1492,291,2,1)],
      [(1475,318,10,4),(1480,307,11,3),(1484,301,12,2)],
      [(1490,317,8,4),(1490,327,5,3),(1494,337,3,1.8)],
      [(1457,331,7,4),(1448,331,5,3),(1436,326,3,2),(1430,320,2,1.2)],
      [(1484,347,5,3),(1493,349,4,2.5),(1506,346,3,1.5)],
    ]
    paths += [
      [(1494,323,6,3),(1504,328,5,3.2),(1515,327,3,1.6)],
      [(1475,339,7,4),(1485,343,5,3.5),(1498,348,3,2),(1507,347,2,1)],
      [(1484,348,5,3),(1485,355,4,3),(1490,360,2,1.3)],
      [(1455,338,5,3),(1448,347,4,3),(1440,350,3,2),(1434,347,2,1.5)],
      [(1443,329,5,3),(1437,339,4,2.7),(1430,337,2,1.5)],
      [(1445,307,8,3),(1438,304,5,2.4),(1436,299,2,1.2)],
    ]
    paths += [
      [(1479,310,10,3),(1488,310,9,3),(1497,313,6,2)],
      [(1488,338,5,3),(1498,340,4,3),(1510,343,2,1.4)],
      [(1445,322,8,3),(1450,325,6,3),(1455,326,5,2)],
      [(1454,345,4,2.5),(1461,349,3,2),(1466,349,2,1)],
      [(1460,332,6,3),(1465,336,4,2),(1468,337,2,1)],
    ]
    paths=[[(x,y,max(z,r+.6),r+.6) for x,y,z,r in path] for path in paths]
    m=Mesh();segments=[]
    def world(p):
        x,y,z,r=p;return Vector((x,-(y+z*COS)/SIN,z))
    for path in paths:
        for a,b in zip(path,path[1:]):
            aa,bb=world(a),world(b);segments.append((aa,bb));m.tube(aa,bb,a[3],b[3],12)
    mesh=bpy.data.meshes.new('Logging root and branch tangle');mesh.from_pydata(m.vertices,[],m.faces);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free();mesh.update();obj=bpy.data.objects.new(mesh.name,mesh);bpy.data.collections['Croisement02 Working'].objects.link(obj)
    for key,value in {'asset_group':asset,'asset_name':'Logging Clearing Fallen Log','source_node':'building-028','part_name':'Additive root and branch tangle','source_role':'Native108 wood; hidden thickness and depth inferred'}.items():obj[key]=value
    rear=bpy.data.materials.new('Inferred branch bark');rear.use_nodes=True;bs=rear.node_tree.nodes.get('Principled BSDF');tex=rear.node_tree.nodes.new('ShaderNodeTexImage');tex.image=supplement.image;tex.interpolation='Linear';tex.extension='REPEAT';node=rear.node_tree.nodes.new('ShaderNodeUVMap');node.uv_map='Inferred board grain';rear.node_tree.links.new(node.outputs['UV'],tex.inputs['Vector']);rear.node_tree.links.new(tex.outputs['Color'],bs.inputs['Base Color']);bs.inputs['Roughness'].default_value=.92;rear.node_tree.links.new(tex.outputs['Color'],bs.inputs['Emission Color']);bs.inputs['Emission Strength'].default_value=.65
    mesh.materials.append(mat);mesh.materials.append(rear);uv=mesh.uv_layers.new(name='Native front projection');grain=mesh.uv_layers.new(name='Inferred board grain');uvnode_native=nodes.new('ShaderNodeUVMap');uvnode_native.uv_map='Native front projection';mat.node_tree.links.new(uvnode_native.outputs['UV'],im.inputs['Vector'])
    for poly in mesh.polygons:
        poly.material_index=0 if poly.normal.dot(RAY)>.01 else 1
        for li in poly.loop_indices:
            v=mesh.vertices[mesh.loops[li].vertex_index].co;uv.data[li].uv=(v.x/1792,1-(-v.y*SIN-v.z*COS)/1152);vi=mesh.loops[li].vertex_index;segment=vi//24;ring=(vi%24)//12;theta=(vi%12)/12;grain.data[li].uv=(theta*2,ring*(segments[segment][1]-segments[segment][0]).length/14)
    assert before=={o.name:_geometry(o,protect_appearance=True)for o in objects}
    added_name=obj.name
    bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'),compress=True);digest=sha(dest/'model.blend')
    bpy.ops.wm.open_mainfile(filepath=str(dest/'model.blend'));assert before=={name:_geometry(bpy.data.objects[name],protect_appearance=True)for name in before}
    packet=json.loads((worker/'modified/views.json').read_text())
    for key in ['object_names','render_object_names']:
        if packet.get(key)is not None:packet[key].append(added_name)
    for view in packet['views']:
        view['crop']={'width':packet['tile_size'][0],'height':packet['tile_size'][1]};view['ortho_scale']*=1.45
    write_json(dest/'cameras.json',packet);scene=bpy.data.scenes['Croisement02 Refinement'];scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256
    render(dest/'cameras.json',dest/'actual',width=384)
    images=[Image.open(dest/f'actual/view-{i}-textured.png').convert('RGB')for i in range(8)];w,h=images[0].size;sheet=Image.new('RGB',(w*4,h*2))
    for i,image in enumerate(images):sheet.paste(image,((i%4)*w,(i//4)*h))
    sheet.save(dest/'actual/sheet.png')
    crop=[1400,250,1550,390];left,top,right,bottom=crop;data=bpy.data.cameras.new('Exact source');data.type='ORTHO';data.ortho_scale=right-left;data.clip_end=10000;camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);scene.camera=camera;center=Vector(((left+right)/2,-(top+bottom)/2/SIN,0));camera.location=center+RAY*6000;camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler();scene.render.resolution_x=(right-left)*3;scene.render.resolution_y=(bottom-top)*3;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.color_mode='RGBA'
    for o in scene.objects:
        if o.type=='MESH':o.hide_render=o.get('asset_group')!=asset
    scene.render.filepath=str(dest/'source.png');bpy.ops.render.render(write_still=True)
    native=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA').crop(crop).resize(((right-left)*3,(bottom-top)*3),Image.Resampling.NEAREST);native.save(dest/'native-source.png');Image.alpha_composite(native,Image.open(dest/'source.png').convert('RGBA')).save(dest/'source-overlay.png')
    visible=[o for o in scene.objects if o.type=='MESH'and not o.hide_render];tree,owners,_=_tree(visible);audit=json.loads((OUT/'restart2-vegetation/neutral-first-hit-v1/audit.json').read_text());samples=next(r['samples']for r in audit['components']if r['component']==2);hits=[]
    for row in samples:
        x,y=row['pixel'];hit,normal,index,distance=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000,-RAY);hits.append(dict(pixel=[x,y],object=owners[index].name if hit else None))
    write_json(dest/'proposal.json',dict(status='Private logging branch tangle; reviews pending',model_sha256=digest,
       original_model=str(old),original_model_sha256=expected,original_geometry_uv_materials_preserved=True,
       original_objects=list(before),added_object=added_name,source_crop=crop,traced_source_paths=paths,
       former_ground_gap_pixels=len(hits),now_wood_first_hit=sum(r['object']is not None for r in hits),hits=hits,
       limitations=['Explicit native source centerlines; hidden depths/radii inferred.',
       'No ground or source ownership changes. Existing approved log unchanged.',
       'Physical branches use opaque own-native projected wood with supplemental source bark for unknown sides.']))
    assert sha(old)==expected and sha(dest/'model.blend')==digest

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
