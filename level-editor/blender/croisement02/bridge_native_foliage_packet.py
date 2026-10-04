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
from catalog import tree_workspace
from render_slots import acquire,release
from refinement_review import _tree,_save,_tile
from refinement_workspace import _geometry
from review_evidence import sha


def main(number, source, output):
    if output.exists(): raise ValueError('Use a new immutable derived packet directory')
    manifest=json.loads((source/'views.json').read_text())
    worker=tree_workspace(number)
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
        scene=bpy.data.scenes[manifest['scene_name']];bpy.context.window.scene=scene
        objects=[scene.objects[name] for name in manifest['object_names']]
        before={o.name:_geometry(o,protect_appearance=True) for o in objects}
        tree,owners,_=_tree(objects)
        records=[];images={};protected=[]
        for obj in objects:
            evaluated=obj.evaluated_get(bpy.context.evaluated_depsgraph_get());mesh=evaluated.to_mesh()
            try:
                mesh.calc_loop_triangles();ownership=mesh.color_attributes.get('Source ownership')
                for triangle in mesh.loop_triangles:
                    material=mesh.materials[triangle.material_index] if mesh.materials else None
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
                    records.append((np.array([obj.matrix_world@mesh.vertices[i].co for i in triangle.vertices]),np.array([uv.data[i].uv[:] for i in triangle.loops]),images[image.name]))
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
            for y in range(height):
                for x in range(width):
                    origin=matrix@Vector((left+(x+.5)*(right-left)/width,bottom+(y+.5)*(top-bottom)/height,0))
                    hit,normal,triangle,_=tree.ray_cast(origin,direction)
                    if hit is None or records[triangle] is None:continue
                    points,uvs,((iw,ih),pixels)=records[triangle]
                    basis=np.stack([points[1]-points[0],points[2]-points[0]],axis=1)
                    weights=np.linalg.lstsq(basis,np.array(hit)-points[0],rcond=None)[0]
                    uv=uvs[0]*(1-weights.sum())+uvs[1]*weights[0]+uvs[2]*weights[1]
                    sx,sy=int(np.floor(uv[0]*iw))%iw,int(np.floor(uv[1]*ih))%ih
                    sample=pixels[sy,sx];assert sample[3]>=.5
                    colors[y,x]=[*sample[:3],1];known[y,x]=[1,1,1,1];selected[y,x]=True
            assert np.array_equal(colors[~selected],original[~selected]) and np.array_equal(known[~selected],original_known[~selected])
            _save(output/'views'/f'view-{index}-textured.png',width,height,array('f',colors.ravel()))
            _save(output/'views'/f'view-{index}-known.png',width,height,array('f',known.ravel()))
            textured.append(array('f',colors.ravel()));reports.append(dict(view=index,protected_front_pixels=int(selected.sum()),previously_unknown_front_pixels=int((selected&(original_known[:,:,0]<.5)).sum()),outside_observed_front_unchanged=True))
            bpy.data.cameras.remove(data);print(reports[-1],flush=True)
            if 'counts' in view:
                transferred=reports[-1]['previously_unknown_front_pixels'];view['counts']['source']+=transferred;view['counts']['unknown']-=transferred
            view['ownership_sha256']=sha(output/'views'/f'view-{index}-known.png')
        _tile(textured,width,height,output/'textured.png')
        assert before=={o.name:_geometry(o,protect_appearance=True) for o in objects}
        manifest['native_foliage_ownership_bridge']=dict(version=1,model_sha256=sha(worker/'model.blend'),source_packet=str(source),source_manifest_sha256=sha(source/'views.json'),rule='First visible physical hit with approved foliage_observed material and unanimous per-corner ownership=1 samples its exact native atlas UV. All other ownership and RGB remain unchanged.',protected_atlases=protected,views=reports)
        (output/'views.json').write_text(json.dumps(manifest,indent=2)+'\n')
        (output/'native-foliage-preservation.json').write_text(json.dumps(dict(status='PASS',geometry_appearance_unchanged=True,**manifest['native_foliage_ownership_bridge']),indent=2)+'\n')
        derivation=json.loads((source/'derivation.json').read_text())
        derivation.update(native_foliage_bridge=manifest['native_foliage_ownership_bridge'],original_source_derivation_sha256=sha(source/'derivation.json'),source_review='pending coordinator inspection of native-front derivative')
        derivation['artifacts']={str(p.relative_to(output)):sha(p) for p in sorted(output.rglob('*')) if p.is_file() and p.name!='derivation.json'}
        (output/'derivation.json').write_text(json.dumps(derivation,indent=2)+'\n')
    finally:release()

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('tree',type=int);parser.add_argument('source',type=Path);parser.add_argument('output',type=Path);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);main(args.tree,args.source.resolve(),args.output.resolve())
