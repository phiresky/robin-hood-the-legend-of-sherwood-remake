"""Restore approved observed foliage RGB to a derived source-only packet.

The generic source renderer reprojects whole-map artwork. Native foliage has
explicit per-corner ownership and a separate approved atlas; preserve that
observed front instead of presenting it to a fill service as unknown.
"""
import argparse, json, shutil, sys
from array import array
from pathlib import Path
import bpy
from mathutils import Matrix, Vector
import numpy as np
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender'),str(Path(__file__).parent)]
from catalog import OUT,tree_workspace
from render_slots import acquire,release
from refinement_review import _tree,_save,_tile
from refinement_workspace import _geometry
from review_evidence import sha


def main(number, source, output, zero_observed_partition=None):
    if output.exists(): raise ValueError('Use a new immutable derived packet directory')
    manifest=json.loads((source/'views.json').read_text())
    worker=tree_workspace(number)
    derivation_source=json.loads((source/'derivation.json').read_text())
    asset=f'croisement02-tree-{number:02d}'
    decisions=[row for row in json.loads((OUT/'user-feedback.json').read_text())['records'] if row['asset_id']==asset]
    if not decisions or decisions[-1]['decision']!='approved':raise ValueError('Explicit current geometry approval required')
    model_hash=sha(worker/'model.blend')
    zero_authority = None
    if zero_observed_partition is not None:
        zero_observed_partition = zero_observed_partition.resolve(strict=True)
        partition = json.loads(zero_observed_partition.read_text())
        if partition['observed_foliage_pixels'] != 0:
            raise ValueError('Expected explicitly zero observed foliage')
        zero_authority = dict(path=str(zero_observed_partition), sha256=sha(zero_observed_partition))
    if manifest['asset_id']!=asset or derivation_source['asset_id']!=asset:raise ValueError('Native-front bridge asset differs from source packet')
    if derivation_source['status']!='PASS' or derivation_source['model_sha256']!=model_hash or decisions[-1]['model_sha256']!=model_hash:raise ValueError('Native-front bridge must use exact approved source geometry')
    for relative,expected in derivation_source['artifacts'].items():
        if sha(source/relative)!=expected:raise ValueError('Derived source packet changed: '+relative)
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
        scene=bpy.data.scenes[manifest['scene_name']];bpy.context.window.scene=scene
        objects=[scene.objects[name] for name in manifest['object_names']]
        before={o.name:_geometry(o,protect_appearance=True) for o in objects}
        tree,owners,_=_tree(objects)
        records=[];images={};protected=[];foliage_faces=[]
        for obj in objects:
            evaluated=obj.evaluated_get(bpy.context.evaluated_depsgraph_get());mesh=evaluated.to_mesh()
            try:
                mesh.calc_loop_triangles();ownership=mesh.color_attributes.get('Source ownership')
                for triangle in mesh.loop_triangles:
                    material=mesh.materials[triangle.material_index] if mesh.materials else None
                    foliage_faces.append(bool(material and material.get('foliage_physical_opacity')))
                    if not material or not material.get('foliage_observed'):
                        records.append(None);continue
                    assert material.get('foliage_physical_opacity') and material.get('source_ownership_channel')=='vertex-color-r'
                    assert ownership is not None and all(ownership.data[i].color[0]==1 for i in triangle.loops), 'Observed foliage must remain explicitly protected'
                    textures=[n for n in material.node_tree.nodes if n.type=='TEX_IMAGE' and n.image]
                    assert len(textures)==1
                    texture=textures[0];assert texture.interpolation=='Closest'
                    uvname=texture.inputs['Vector'].links[0].from_node.uv_map;uv=mesh.uv_layers[uvname]
                    image=texture.image
                    if image.name not in images:
                        pixels=np.empty(len(image.pixels),np.float32);image.pixels.foreach_get(pixels)
                        images[image.name]=(tuple(image.size),pixels.reshape(image.size[1],image.size[0],4))
                        protected.append(dict(object=obj.name,material=material.name,image=image.name,image_pixels_sha256=__import__('hashlib').sha256(pixels.tobytes()).hexdigest()))
                    records.append((tuple(obj.matrix_world@mesh.vertices[i].co for i in triangle.vertices),tuple(uv.data[i].uv[:] for i in triangle.loops),images[image.name],texture.extension))
            finally:evaluated.to_mesh_clear()
        assert len(records)==len(owners)
        shutil.copytree(source,output)
        width,height=manifest['tile_size'];scene.render.resolution_x=width;scene.render.resolution_y=height;scene.render.pixel_aspect_x=scene.render.pixel_aspect_y=1
        textured=[];reports=[]
        def load(path):
            image=bpy.data.images.load(str(path),check_existing=False)
            try:
                pixels=np.empty(len(image.pixels),np.float32);image.pixels.foreach_get(pixels);return pixels.reshape(height,width,4)
            finally:bpy.data.images.remove(image)
        for view in manifest['views']:
            index=view['index'];data=bpy.data.cameras.new('Native foliage ownership audit');data.type='ORTHO';data.ortho_scale=view['ortho_scale']
            frame=data.view_frame(scene=scene);left,right=min(p.x for p in frame),max(p.x for p in frame);bottom,top=min(p.y for p in frame),max(p.y for p in frame)
            matrix=Matrix(view['camera_matrix_world']);direction=matrix.to_3x3()@Vector((0,0,-1))
            colors=load(source/'views'/f'view-{index}-textured.png');known=load(source/'views'/f'view-{index}-known.png');original=colors.copy();original_known=known.copy();selected=np.zeros((height,width),bool)
            zero_checked = 0
            for y in range(height):
                for x in range(width):
                    origin=matrix@Vector((left+(x+.5)*(right-left)/width,bottom+(y+.5)*(top-bottom)/height,0))
                    hit,normal,triangle,_=tree.ray_cast(origin,direction)
                    if hit is None:continue
                    if zero_authority and foliage_faces[triangle]:
                        if original_known[y,x,0] >= .5:
                            raise ValueError('Off-map inferred foliage was incorrectly marked as observed source')
                        zero_checked += 1
                    if records[triangle] is None:continue
                    points,uvs,((iw,ih),pixels),extension=records[triangle]
                    # Match the physical-opacity ray's arithmetic exactly. A
                    # different precision at a texel boundary can select the
                    # transparent neighbor of the texel that accepted the ray.
                    a,b,c=points;ab,ac,ap=b-a,c-a,hit-a
                    aa,bb,cc=ab.dot(ab),ab.dot(ac),ac.dot(ac)
                    determinant=aa*cc-bb*bb
                    assert abs(determinant)>=1e-20
                    u=(cc*ap.dot(ab)-bb*ap.dot(ac))/determinant
                    v=(aa*ap.dot(ac)-bb*ap.dot(ab))/determinant
                    uv=[uvs[0][i]*(1-u-v)+uvs[1][i]*u+uvs[2][i]*v for i in range(2)]
                    sx,sy=int(np.floor(uv[0]*iw)),int(np.floor(uv[1]*ih))
                    if extension=='REPEAT':sx,sy=sx%iw,sy%ih
                    elif extension=='CLIP':assert 0<=sx<iw and 0<=sy<ih
                    else:sx,sy=min(iw-1,max(0,sx)),min(ih-1,max(0,sy))
                    sample=pixels[sy,sx];assert sample[3]>=.5
                    colors[y,x]=[*sample[:3],1];known[y,x]=[1,1,1,1];selected[y,x]=True
            assert np.array_equal(colors[~selected],original[~selected]) and np.array_equal(known[~selected],original_known[~selected])
            _save(output/'views'/f'view-{index}-textured.png',width,height,array('f',colors.ravel()))
            _save(output/'views'/f'view-{index}-known.png',width,height,array('f',known.ravel()))
            textured.append(array('f',colors.ravel()));reports.append(dict(view=index,protected_front_pixels=int(selected.sum()),previously_unknown_front_pixels=int((selected&(original_known[:,:,0]<.5)).sum()),outside_observed_front_unchanged=True))
            if zero_authority:
                if selected.any():raise ValueError('Off-map foliage unexpectedly has observed texels')
                reports[-1]['inferred_foliage_pixels_checked_unknown'] = zero_checked
            bpy.data.cameras.remove(data);print(reports[-1],flush=True)
            if 'counts' in view:
                transferred=reports[-1]['previously_unknown_front_pixels'];view['counts']['source']+=transferred;view['counts']['unknown']-=transferred
            view['ownership_sha256']=sha(output/'views'/f'view-{index}-known.png')
        _tile(textured,width,height,output/'textured.png')
        assert before=={o.name:_geometry(o,protect_appearance=True) for o in objects}
        if sha(worker/'model.blend')!=model_hash:raise ValueError('Approved model changed during native-front bridge')
        manifest['native_foliage_ownership_bridge']=dict(version=1,model_sha256=sha(worker/'model.blend'),source_packet=str(source),source_manifest_sha256=sha(source/'views.json'),rule='First visible physical hit with approved foliage_observed material and unanimous per-corner ownership=1 samples its exact native atlas UV. All other ownership and RGB remain unchanged.',protected_atlases=protected,views=reports)
        if zero_authority:
            if sha(zero_observed_partition) != zero_authority['sha256']:raise ValueError('Off-map authority changed')
            manifest['native_foliage_ownership_bridge']['zero_observed_authority'] = zero_authority
        (output/'views.json').write_text(json.dumps(manifest,indent=2)+'\n')
        (output/'native-foliage-preservation.json').write_text(json.dumps(dict(status='PASS',geometry_appearance_unchanged=True,**manifest['native_foliage_ownership_bridge']),indent=2)+'\n')
        derivation=json.loads((source/'derivation.json').read_text())
        derivation.update(native_foliage_bridge=manifest['native_foliage_ownership_bridge'],original_source_derivation_sha256=sha(source/'derivation.json'),source_review='pending coordinator inspection of native-front derivative')
        derivation['artifacts']={str(p.relative_to(output)):sha(p) for p in sorted(output.rglob('*')) if p.is_file() and p.name!='derivation.json'}
        (output/'derivation.json').write_text(json.dumps(derivation,indent=2)+'\n')
    finally:release()

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('tree',type=int);parser.add_argument('source',type=Path);parser.add_argument('output',type=Path);parser.add_argument('--zero-observed-partition',type=Path);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);main(args.tree,args.source.resolve(),args.output.resolve(),args.zero_observed_partition)
