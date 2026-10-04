"""Measure tree07 basal-root coverage independently of its much larger crown."""
import argparse
import hashlib
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,RAY


def preserved_mesh_state(worker, root_and_wood=False):
    bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));bpy.context.view_layer.update()
    result={}
    for obj in bpy.data.collections['Croisement02 Working'].all_objects:
        if obj.type!='MESH' or obj.get('asset_group')!='croisement02-tree-07':continue
        if root_and_wood and obj.get('projection_component')=='crown':continue
        if not root_and_wood and obj.get('root_completion'):continue
        mesh=obj.data
        record=dict(vertices=[list(v.co) for v in mesh.vertices],faces=[list(f.vertices) for f in mesh.polygons],
                    slots=[f.material_index for f in mesh.polygons],smooth=[f.use_smooth for f in mesh.polygons],
                    uv=[[list(v.uv) for v in layer.data] for layer in mesh.uv_layers],matrix=[list(row) for row in obj.matrix_world],materials=[])
        for material in mesh.materials:
            nodes=[]
            for node in material.node_tree.nodes:
                entry=dict(type=node.type,inputs=[])
                for socket in node.inputs:
                    if not hasattr(socket,'default_value'):continue
                    value=socket.default_value
                    if isinstance(value,(str,int,float,bool)):entry['inputs'].append(value)
                    else:
                        try:entry['inputs'].append(list(value))
                        except TypeError:entry['inputs'].append(str(type(value)))
                if node.type=='TEX_IMAGE' and node.image:
                    pixels=np.empty(len(node.image.pixels),dtype=np.float32);node.image.pixels.foreach_get(pixels)
                    entry['image_sha256']=hashlib.sha256(pixels.tobytes()).hexdigest()
                nodes.append(entry)
            record['materials'].append(dict(nodes=nodes,links=[(link.from_node.type,link.from_socket.name,link.to_node.type,link.to_socket.name) for link in material.node_tree.links]))
        result[obj.name]=hashlib.sha256(json.dumps(record,sort_keys=True).encode()).hexdigest()
    return result


def main(release_slot=True):
    parser=argparse.ArgumentParser();parser.add_argument('--worker',type=Path,default=tree_workspace(7));parser.add_argument('--preservation-base',type=Path);parser.add_argument('--output',type=Path);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    worker=args.worker.resolve();model_hash=sha(worker/'model.blend');output=args.output.resolve() if args.output else worker/'inspection';output.mkdir(parents=True,exist_ok=True);acquire()
    try:
        original=args.preservation_base.resolve() if args.preservation_base else tree_workspace(7)
        previous=preserved_mesh_state(original);current=preserved_mesh_state(worker)
        changed=[o.name for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')=='croisement02-tree-07' and o.get('source_node') in ('building-058','building-062') and o.get('projection_component')!='crown']
        for name in changed:previous.pop(name);current.pop(name)
        preservation=dict(previous_worker=str(original),previous_model_sha256=sha(original/'model.blend'),model_sha256=model_hash,previous_meshes=previous,current_meshes=current,preserved=previous==current,scope='Selected crown and original upper wood; lower stem058 and junction062 intentionally repaired; retained upper062 positions separately verified')
        if previous!=current:raise ValueError('Existing tree07 mesh, transform, UV, or material changed')
        write_json(output/'root-preservation.json',preservation)
        stem=next(o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')=='croisement02-tree-07' and o.get('source_node')=='building-058' and o.get('projection_component')!='crown')
        support_z=43.949;crossings=[]
        for edge in stem.data.edges:
            a,b=[stem.matrix_world@stem.data.vertices[i].co for i in edge.vertices]
            if (a.z-support_z)*(b.z-support_z)<0:
                point=a.lerp(b,(support_z-a.z)/(b.z-a.z));crossings.append(list(point))
        if not crossings:raise ValueError('Lower stem does not cross the measured bank support plane')
        array=np.asarray(crossings)
        write_json(output/'stem-support-section.json',dict(model_sha256=model_hash,plane_z=support_z,points=crossings,width=float(np.ptp(array[:,0])),depth=float(np.ptp(array[:,1])),scope='Lower wood geometric section at measured bank0 plateau height; actual opaque appearance and local bank overlap require joint visual review'))
        scene=bpy.data.scenes.new('Independent root source coverage');bpy.context.window.scene=scene
        white=bpy.data.materials.new('Opaque wood coverage');white.use_nodes=True
        nodes=white.node_tree.nodes;nodes.clear();emission=nodes.new('ShaderNodeEmission');material_output=nodes.new('ShaderNodeOutputMaterial');white.node_tree.links.new(emission.outputs['Emission'],material_output.inputs['Surface'])
        for original in list(bpy.data.collections['Croisement02 Working'].all_objects):
            if original.type!='MESH' or original.get('asset_group')!='croisement02-tree-07' or original.get('projection_component')=='crown':continue
            obj=original.copy();obj.data=original.data.copy();transform=original.matrix_world.copy();obj.parent=None;obj.matrix_world=transform;obj.hide_render=False;scene.collection.objects.link(obj)
            obj.data.materials.clear();obj.data.materials.append(white)
            for face in obj.data.polygons:face.material_index=0
        box=(725,430,800,545);left,top,right,bottom=box;width=right-left;height=bottom-top
        target=Vector(((left+right)/2,-(top+bottom)/2/SIN,0));data=bpy.data.cameras.new('Native root camera');data.type='ORTHO';data.sensor_fit='HORIZONTAL';data.ortho_scale=width;data.clip_end=20000
        camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);camera.location=target+RAY*5000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();scene.camera=camera
        scene.render.engine='CYCLES';scene.cycles.samples=8;scene.render.resolution_x=width;scene.render.resolution_y=height;scene.render.resolution_percentage=100
        scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
        destination=output/'root-source-coverage';destination.mkdir(exist_ok=True)
        scene.render.filepath=str(destination/'render.png');bpy.ops.render.render(write_still=True,scene=scene.name)
        actual=np.asarray(Image.open(destination/'render.png').convert('RGBA'))[:,:,3]>127
        row=next(r for r in json.loads((OUT/'scenery-domains/inventory.json').read_text())['masks'] if r['index']==7)
        native=Image.new('L',(1792,1152));native.paste(Image.open(row['png']).convert('L'),tuple(row['box_top_left']))
        allocation=Image.new('L',native.size);ImageDraw.Draw(allocation).rectangle((735,430,790,529),fill=255)
        local=(np.asarray(native)>0)&(np.asarray(allocation)>0)
        expected=local[top:bottom,left:right];missing=expected&~actual
        Image.fromarray(local.astype('uint8')*255).save(destination/'domain.png')
        source=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGB').crop(box);difference=np.asarray(source).copy();difference[missing]=[255,40,40]
        Image.fromarray(difference).resize((450,690),Image.Resampling.NEAREST).save(destination/'difference.png')
        source.resize((450,690),Image.Resampling.NEAREST).save(destination/'source.png')
        bpy.context.view_layer.update();deps=bpy.context.evaluated_depsgraph_get();samples=[]
        for y in [445,455,465,475,485,495,505,515,525]:
            for x in [745,750,755,760,765,770,775]:
                hit,location,normal,index,obj,matrix=scene.ray_cast(deps,Vector((x,-y/SIN,0))+RAY*5000,-RAY)
                samples.append(dict(source=[x,y],world=list(location) if hit else None))
        interface=expected[:445-top];interface_missing=missing[:445-top]
        write_json(destination/'report.json',dict(interface_expected_pixels=int(interface.sum()),interface_missing_pixels=int(interface_missing.sum()),interface_source_coverage=float(1-interface_missing.sum()/interface.sum()),model_sha256=model_hash,domain_sha256=sha(destination/'domain.png'),expected_pixels=int(expected.sum()),missing_pixels=int(missing.sum()),source_coverage=float((expected&actual).sum()/expected.sum()),root_expected_pixels=int(expected[492-top:].sum()),root_missing_pixels=int(missing[492-top:].sum()),root_source_coverage=float((expected[492-top:]&actual[492-top:]).sum()/expected[492-top:].sum()),source_crop=box,samples=samples,status='local native7 stem and basal-root measurement; requires visual review'))
        if sha(worker/'model.blend')!=model_hash:raise ValueError('Audit changed worker')
        print(destination)
    finally:
        if release_slot:release()


if __name__=='__main__':main()
