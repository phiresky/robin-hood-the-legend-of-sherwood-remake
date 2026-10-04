"""Read-only source visibility of the north fringe behind unchanged crowns."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import RAY,SIN,COS
from render_slots import acquire,release

def main():
    dest=OUT/'understory-candidates/north-fringe22-visibility-v3';dest.mkdir(exist_ok=False)
    bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene
    fringe=OUT/'understory-round-23/assets/croisement02-canopy-fringe-22'
    proof=json.loads((OUT/'understory-candidates/north-fringe22-audit/evidence.json').read_text())
    paths=[fringe]+[Path(r['worker']) for r in proof['selected_crowns']];records=[];plants=[];original_materials={}
    for n,path in enumerate(paths):
        digest=sha(path/'model.blend');audit=json.loads((path/'inspection/saved-model-audit.json').read_text())
        if audit['model_sha256']!=digest or audit['status']!='PASS':raise ValueError('Invalid worker audit')
        with bpy.data.libraries.load(str(path/'model.blend'),link=False) as (src,dst):dst.objects=[r['object'] for r in audit['objects']]
        for obj in dst.objects:
            scene.collection.objects.link(obj);parent=obj.parent
            while parent:
                if not parent.users_collection:scene.collection.objects.link(parent)
                parent=parent.parent
        bpy.context.view_layer.update()
        for obj in dst.objects:
            matrix=obj.matrix_world.copy();obj.parent=None;obj.matrix_world=matrix;obj.hide_render=False
            if n==0:plants.append((obj,matrix))
            original_materials[obj]=list(obj.data.materials)
            for slot,material in enumerate(list(obj.data.materials)):
                if material is None:continue
                copy=material.copy();obj.data.materials[slot]=copy;color=(1,1,1,1) if n==0 and slot==0 else (0,0,0,1)
                if copy.use_nodes:
                    for node in copy.node_tree.nodes:
                        sockets=[node.inputs.get('Color')] if node.type=='EMISSION' else [node.inputs.get('Base Color'),node.inputs.get('Emission Color')] if node.type=='BSDF_PRINCIPLED' else []
                        for socket in sockets:
                            if socket is not None:
                                for link in list(socket.links):copy.node_tree.links.remove(link)
                                socket.default_value=color
                        if node.type=='BSDF_PRINCIPLED':node.inputs['Emission Strength'].default_value=1
        records.append(dict(path=str(path),model_sha256=digest))
    scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256
    scene.render.resolution_x=57;scene.render.resolution_y=36;scene.render.resolution_percentage=100
    scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA'
    scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
    data=bpy.data.cameras.new('Exact native diagnostic');data.type='ORTHO';data.ortho_scale=57;data.clip_end=20000
    camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);scene.camera=camera
    x,y=1339.5,18;center=Vector((x,-y*SIN,-y*COS));camera.location=center+RAY*5000;camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler()
    domain=np.asarray(Image.open(OUT/'understory-candidates/north-fringe22-audit/domain-480.png').convert('L'))[0:36,1311:1368]>127
    results=[]
    for offset in [0,-40,-80,-120,-160,-200,-240]:
        for obj,matrix in plants:obj.matrix_world=matrix.copy();obj.location+=RAY*offset
        bpy.context.view_layer.update();output=dest/f'offset-{abs(offset):03}.png';scene.render.filepath=str(output);bpy.ops.render.render(write_still=True)
        image=np.asarray(Image.open(output).convert('RGBA'));white=(image[:,:,:3].min(axis=2)>127)&(image[:,:,3]>127)
        results.append(dict(ray_offset=offset,visible_owned_pixels=int((white&domain).sum()),expected_owned_pixels=int(domain.sum()),fraction=float((white&domain).sum()/domain.sum()),image=output.name,sha256=sha(output)))
    for obj,materials in original_materials.items():
        for slot,material in enumerate(materials):obj.data.materials[slot]=material
    scene.render.resolution_x=180;scene.render.resolution_y=150;data.ortho_scale=180
    x,y=1350,25;center=Vector((x,-y*SIN,-y*COS));camera.location=center+RAY*5000;camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler()
    actual=[]
    for offset in [0,-40,-80,-120,-160]:
        for obj,matrix in plants:obj.matrix_world=matrix.copy();obj.location+=RAY*offset
        bpy.context.view_layer.update();output=dest/f'actual-{abs(offset):03}.png';scene.render.filepath=str(output);bpy.ops.render.render(write_still=True)
        actual.append(dict(ray_offset=offset,image=output.name,sha256=sha(output)))
    write_json(dest/'actual-evidence.json',dict(workers=records,views=actual,native_box=[1260,-50,1440,100]))
    for row in records:
        if sha(Path(row['path'])/'model.blend')!=row['model_sha256']:raise ValueError('Input mutated')
    write_json(dest/'evidence.json',dict(status='Read-only diagnostic; no saved model changes',workers=records,results=results,domain_sha256=sha(OUT/'understory-candidates/north-fringe22-audit/domain-480.png'),note='White is observed fringe slot0, all other surfaces black. Texture alpha and one-sided transparency chains preserved. Source-ray displacement preserves projection; neighbor alpha determines actual occlusion.'))
    print(json.dumps(results),flush=True)
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
