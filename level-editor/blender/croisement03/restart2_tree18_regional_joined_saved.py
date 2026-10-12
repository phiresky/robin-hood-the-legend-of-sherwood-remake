"""Private complete regional geometry diagnostic; no observed ownership expansion."""
import hashlib
import json
import math
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector
sys.path.insert(0,str(Path(__file__).resolve().parent))
import restart2_tree01_canopy_saved as guard

B,S,C=guard.B,guard.S,guard.C
OUT=B/'tree18-regional-joined-saved-v1'
RAY=Vector((0,-C,S))


def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()


def main():
    bank=B/'bank-coupled-left-full-source-v3/worker.blend'
    wood=B/'tree18-united-wood-saved-v1/worker.blend'
    crown_path=B/'tree18-regional-crown-cpu-v1/geometry.json'
    green_path=B/'tree18-regional-green-cpu-v2/geometry.json'
    exclusions=green_path.parent/'independent-projected-exclusions.json'
    assert json.loads(exclusions.read_text())['status']=='PASS_SCOPED_PROJECTED_EXCLUSIONS'
    assert json.loads(exclusions.read_text())['geometry_sha256']==sha(green_path)
    assert json.loads((wood.parent/'receipt.json').read_text())['status']=='PASS_SAVED_UNION_REQUIRES_VISUAL_CONTACT_REVIEW'
    crown,green=[json.loads(p.read_text())for p in (crown_path,green_path)]
    audit_path=B/'bank-foreground-source-audit-v2/receipt.json';audit=json.loads(audit_path.read_text())
    atlas=crown_path.parent/'own-native-gold-atlas.png';assert sha(atlas)==crown['atlas_sha256']
    pins={str(p):sha(p)for p in [bank,wood,crown_path,green_path,exclusions,audit_path,atlas,Path(__file__)]}
    for recipe in (crown,green):
        for path,digest in recipe['pins'].items():assert sha(Path(path))==digest; pins[path]=digest
    guard.resources();guard.acquire(slots=2)
    try:
        guard.resources();assert not OUT.exists();OUT.mkdir()
        bpy.ops.wm.open_mainfile(filepath=str(bank));scene=bpy.context.scene
        banks=[o for o in scene.objects if o.type=='MESH' and o.name.startswith('Candidate bank ')]
        assert len(banks)==3
        for o in scene.objects:
            if o.type=='MESH':o.hide_render=o not in banks
        with bpy.data.libraries.load(str(wood),link=False)as(src,dst):dst.objects=[n for n in src.objects if n=='Provisional Tree18 united wood']
        assert len(dst.objects)==1;trunk=dst.objects[0];scene.collection.objects.link(trunk);trunk.hide_render=False
        def mesh(name,data,material):
            m=bpy.data.meshes.new(name);m.from_pydata(data['vertices'],[],data['faces']);m.update();o=bpy.data.objects.new(name,m);scene.collection.objects.link(o);m.materials.append(material);o['appearance_scope']='Private inferred regional diagnostic; no observed source assignment.';return o
        def material(name,color):
            m=bpy.data.materials.new(name);m.diffuse_color=(*color,1);m.use_nodes=True;m.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(*color,1);return m
        unknown=material('UNASSIGNED regional plant geometry',(.35,.35,.35))
        leafmat=bpy.data.materials.new('Own-native inferred gold rear leaves');leafmat.use_nodes=True
        n,l=leafmat.node_tree.nodes,leafmat.node_tree.links;n.clear();tex=n.new('ShaderNodeTexImage');tex.image=bpy.data.images.load(str(atlas),check_existing=True);tex.image.pack();tex.interpolation='Closest';tex.extension='CLIP'
        emission=n.new('ShaderNodeEmission');transparent=n.new('ShaderNodeBsdfTransparent');alpha=n.new('ShaderNodeMixShader');back=n.new('ShaderNodeMixShader');geometry=n.new('ShaderNodeNewGeometry');output=n.new('ShaderNodeOutputMaterial')
        l.new(tex.outputs['Color'],emission.inputs[0]);l.new(tex.outputs['Alpha'],alpha.inputs[0]);l.new(transparent.outputs[0],alpha.inputs[1]);l.new(emission.outputs[0],alpha.inputs[2]);l.new(geometry.outputs['Backfacing'],back.inputs[0]);l.new(alpha.outputs[0],back.inputs[1]);l.new(transparent.outputs[0],back.inputs[2]);l.new(back.outputs[0],output.inputs['Surface'])
        canopy=mesh('Tree18 inferred rear crown only',crown,leafmat);uv=canopy.data.uv_layers.new(name='Exact own-native donor UV')
        for loop in canopy.data.loops:uv.data[loop.index].uv=crown['uvs'][loop.vertex_index]
        plants=[mesh('Regional '+part['id'],part,unknown)for part in green['parts']]
        # Native observed pixels retain their independent single owner. This
        # editor-plane diagnostic is not a source of physical inferred depth.
        row=audit['frames'][0];x,y,w,h=row['bbox'];assert sha(Path(row['source']))==row['source_sha256'];pins[row['source']]=row['source_sha256']
        plane_y=-462/S;native_material=bpy.data.materials.new('Exact native Arbre05 diagnostic owner');native_material.use_nodes=True;n,l=native_material.node_tree.nodes,native_material.node_tree.links;n.clear();tex=n.new('ShaderNodeTexImage');tex.image=bpy.data.images.load(row['source'],check_existing=True);tex.image.pack();tex.interpolation='Closest';emission=n.new('ShaderNodeEmission');transparent=n.new('ShaderNodeBsdfTransparent');mix=n.new('ShaderNodeMixShader');output=n.new('ShaderNodeOutputMaterial');l.new(tex.outputs['Color'],emission.inputs[0]);l.new(tex.outputs['Alpha'],mix.inputs[0]);l.new(transparent.outputs[0],mix.inputs[1]);l.new(emission.outputs[0],mix.inputs[2]);l.new(mix.outputs[0],output.inputs['Surface'])
        native=mesh('Native Arbre05 diagnostic single owner',{'vertices':[(x,plane_y,(462-y)/C),(x+w,plane_y,(462-y)/C),(x+w,plane_y,(462-y-h)/C),(x,plane_y,(462-y-h)/C)],'faces':[(0,3,2,1)]},native_material);uv=native.data.uv_layers.new(name='Exact native pixel coordinates');coords=[(0,1),(1,1),(1,0),(0,0)]
        for loop in native.data.loops:uv.data[loop.index].uv=coords[loop.vertex_index]
        scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.use_denoising=False;scene.cycles.transparent_max_bounces=256;scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard'
        bpy.context.preferences.filepaths.save_version=0;model=OUT/'worker.blend';bpy.ops.wm.save_as_mainfile(filepath=str(model));assert model.stat().st_size<=8*1024**2
        bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;unknown=bpy.data.materials['UNASSIGNED regional plant geometry']
        selected=[o for o in scene.objects if o.type=='MESH' and not o.hide_render];points=np.array([list(o.matrix_world@v.co)for o in selected for v in o.data.vertices])
        camera=bpy.data.objects.new('Regional review camera',bpy.data.cameras.new('Regional review camera'));scene.collection.objects.link(camera);camera.data.type='ORTHO';camera.data.clip_end=10000;scene.camera=camera
        scene.render.resolution_x=384;scene.render.resolution_y=384;scene.render.resolution_percentage=100
        original=Vector(points.mean(0));cameras=[]
        for index in range(8):
            angle=math.radians(index*45);direction=Vector((math.sin(angle)*C,-math.cos(angle)*C,S));rotation=(-direction).to_track_quat('-Z','Y');local=np.array([list(rotation.conjugated()@(Vector(p)-original))for p in points]);middle=(local[:,:2].min(0)+local[:,:2].max(0))/2;target=original+rotation@Vector((*middle,0));cameras.append((target+direction*5000,rotation.to_euler(),float(max(np.ptp(local[:,0]),np.ptp(local[:,1]))*1.12)))
        for kind in ['actual','solid']:
            sheet=Image.new('RGB',(1536,768),'#333333')
            scene.view_layers[0].material_override=unknown if kind=='solid'else None
            scene.objects['Native Arbre05 diagnostic single owner'].hide_render=kind=='solid'
            for index,(location,rotation,scale)in enumerate(cameras):
                camera.location=location;camera.rotation_euler=rotation;camera.data.ortho_scale=scale;scene.render.filepath=str(OUT/f'{kind}-{index*45:03}.png');bpy.ops.render.render(write_still=True);im=Image.open(scene.render.filepath).convert('RGBA');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),(index%4*384,index//4*384))
            sheet.save(OUT/f'{kind}-eight.png')
        scene.view_layers[0].material_override=None;scene.objects['Native Arbre05 diagnostic single owner'].hide_render=False
        box=(95,145,650,605);scene.render.resolution_x=666;scene.render.resolution_y=552;center=((box[0]+box[2])/2,(box[1]+box[3])/2);target=Vector((center[0],-center[1]/S,0));camera.location=target+RAY*5000;camera.rotation_euler=(-RAY).to_track_quat('-Z','Y').to_euler();camera.data.ortho_scale=box[2]-box[0]
        scene.render.filepath=str(OUT/'native-combined.png');bpy.ops.render.render(write_still=True)
        comparison=Image.new('RGB',(1332,552),'#333333');comparison.paste(Image.open(B.parent/'baseline/covered.png').convert('RGB').crop(box).resize((666,552),Image.Resampling.NEAREST),(0,0));im=Image.open(OUT/'native-combined.png').convert('RGBA');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);comparison.paste(bg.convert('RGB'),(666,0));comparison.save(OUT/'native-comparison.png')
        # Convergence is checked on the exact same saved geometry and camera.
        scene.cycles.transparent_max_bounces=512;scene.render.filepath=str(OUT/'native-512.png');bpy.ops.render.render(write_still=True)
        a=np.array(Image.open(OUT/'native-combined.png'));b=np.array(Image.open(OUT/'native-512.png'));delta=np.abs(a.astype(int)-b.astype(int))
        assert all(sha(Path(p))==h for p,h in pins.items())
        receipt={'status':'HOLD_REGIONAL_DIAGNOSTIC_REQUIRES_PERSONAL_REVIEW','pins':pins,'model_sha256':sha(model),'model_bytes':model.stat().st_size,'green_parts':[p['id']for p in green['parts']],'green_material_assignment':'Unassigned gray geometry only; no leaf source domain approved.','inferred_crown_width':crown['width'],'inferred_crown_depth':crown['depth'],'native_phase_shown':0,'transparent256_512_max_channel_delta':int(delta.max()),'transparent256_512_changed_pixels':int(np.any(delta,axis=2).sum()),'limits':['Complete regional geometry proposal, not final textured appearance or approval. Gray green-plant geometry is explicitly separate from gray unclassified rock.','Only phase00 rendered here; all14 opaque foreground compatibility and complete neighboring source ownership remain mandatory.','Retained94–97 context meshes remain hidden in these isolated views, not certified. Saved physical intersections/contact validation remains required.','Gold rear foliage derives only from own native donor atlas. Green source appearance is unassigned; no synthetic fill or pixel ownership expansion.','Solid views hide the native observed plane to reveal geometry; actual views preserve its single full source owner.']};(OUT/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');assert sum(p.stat().st_size for p in OUT.rglob('*')if p.is_file())<=32*1024**2
    finally:guard.release()


if __name__=='__main__':main()
