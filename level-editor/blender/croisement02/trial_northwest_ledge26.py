"""Privately extend the small source-traced ledge without moving other rock parts."""
import hashlib,json,math
from pathlib import Path
import sys
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector,Matrix
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from correct_bank_foot import surface,digest
from workspace_components import appearance_state
from refinement_workspace import _geometry
from tree_geometry import SIN,COS,RAY
from review_bank_candidate import camera


def hit(tree,x,y):
    return tree.ray_cast(Vector((x,-y/SIN,0))+Vector(RAY)*10000,-Vector(RAY))


def main():
    output=OUT/'restart2-bank321/northwest-ledge26-v2'
    if output.exists():raise FileExistsError(output)
    output.mkdir()
    source=OUT/'restart2-bank321/northwest-edge-ramp-v1/worker.blend'
    bank=OUT/'restart2-bank321/packaged-v1/assets/croisement02-north-woodland-bank/model.blend'
    comp=json.loads((OUT/'restart2-bank321/northwest-remaining119-v1/components.json').read_text())['components'][0]
    target_pixels=comp['pixels']
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(bank));bpy.context.view_layer.update()
        banks=[o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-north-woodland-bank']
        bank_tree=surface(banks);bank_matrices={o.name:[list(r) for r in o.matrix_world] for o in banks}
        bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();bpy.context.preferences.filepaths.save_version=0
        scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene
        rocks=[o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-northwest-rock-outcrop']
        obj=next(o for o in rocks if o.get('source_node')=='building-035');name=obj.name
        before_tree=surface(rocks);part_tree=surface([obj])
        foreign={o.name:digest(_geometry(o)) for o in scene.objects if o.type=='MESH' and o!=obj}
        other_appearance={o.name:digest(appearance_state(o,{})) for o in rocks if o!=obj}
        before=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);screen=np.column_stack((before[:,0],-before[:,1]*SIN-before[:,2]*COS))
        old_active_uv=obj.data.uv_layers.active_index;old_render_uv=next((layer.name for layer in obj.data.uv_layers if layer.active_render),None)
        old_uvs={layer.name:np.array([loop.uv[:] for loop in layer.data]) for layer in obj.data.uv_layers}
        image_hash=lambda im:hashlib.sha256(np.array(im.pixels[:],dtype=np.float32).tobytes()).hexdigest()
        old_images={im.name:image_hash(im) for im in bpy.data.images if im.type=='IMAGE' and im.has_data}
        profile=[]
        for y in range(104,121):
            xs=[x for x,yy in target_pixels if yy==y]
            # Bracket the existing right edge at the exact source row center.
            lo,hi=100.,115.
            if hit(part_tree,lo,y+.5)[0] is None:raise ValueError('Missing inner ledge anchor')
            for _ in range(18):
                mid=(lo+hi)/2
                if hit(part_tree,mid,y+.5)[0] is None:hi=mid
                else:lo=mid
            edge=(lo+hi)/2;desired=max(xs)+.75 if xs else edge
            profile.append(dict(y=y+.5,edge=edge,delta=max(0.,desired-edge)))
        ys=np.array([r['y'] for r in profile]);edges=np.array([r['edge'] for r in profile]);deltas=np.array([r['delta'] for r in profile])
        inverse=obj.matrix_world.inverted();moved=[]
        for i,vertex in enumerate(obj.data.vertices):
            x,y=screen[i]
            if not 104.5<=y<=120.5 or x<=100:continue
            edge=float(np.interp(y,ys,edges));delta=float(np.interp(y,ys,deltas));weight=max(0.,min(1.,(x-100)/max(.001,edge-100)));dx=delta*weight
            if dx<=.00001:continue
            point=Vector(before[i]);point.x+=dx;vertex.co=inverse@point;moved.append(dict(vertex=i,world_dx=float(dx),source_before=[float(x),float(y)]))
        bpy.context.view_layer.update()
        after=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);moved_ids={r['vertex'] for r in moved}
        if not moved:raise ValueError('No ledge vertices changed')
        # Restore native artwork on changed faces through an additional UV set.
        # Existing atlases, UV layers and all unrelated material slots remain exact.
        domain=np.array(Image.open(OUT/'northwest-rock-source-revision/domain-380.png').convert('L'))>0
        local_domain=np.zeros_like(domain);local_domain[103:122,98:117]=domain[103:122,98:117]
        Image.fromarray(local_domain.astype('uint8')*255).save(output/'native-overlay-domain.png')
        source_image=bpy.data.images.load(str(OUT/'animation-references/composite-frame-0.png'),check_existing=False);source_image.name='NW ledge exact native source';source_image.pack()
        mask=bpy.data.images.load(str(output/'native-overlay-domain.png'),check_existing=False);mask.name='NW ledge native domain';mask.colorspace_settings.name='Non-Color';mask.pack()
        uv=obj.data.uv_layers.new(name='Ledge source projection',do_init=False)
        obj.data.uv_layers.active_index=old_active_uv
        if old_render_uv:obj.data.uv_layers[old_render_uv].active_render=True
        obj.data.update()
        for face in obj.data.polygons:
            for loop in face.loop_indices:
                point=after[obj.data.loops[loop].vertex_index];uv.data[loop].uv=(point[0]/1792,1-(-point[1]*SIN-point[2]*COS)/1152)
        clones={};changed_faces=[]
        for face in obj.data.polygons:
            if not any(i in moved_ids for i in face.vertices):continue
            normal=obj.matrix_world.to_3x3().inverted().transposed()@face.normal
            if normal.normalized().dot(Vector(RAY))<=.05:continue
            old=face.material_index
            if old not in clones:
                mat=obj.data.materials[old].copy();mat.name+=' / exact ledge source';nodes=mat.node_tree.nodes;links=mat.node_tree.links
                dest=next(n for n in nodes if n.type=='OUTPUT_MATERIAL' and n.is_active_output)
                original=dest.inputs['Surface'].links[0].from_socket
                mapping=nodes.new('ShaderNodeUVMap');mapping.uv_map=uv.name
                color=nodes.new('ShaderNodeTexImage');color.image=source_image;color.interpolation='Closest';links.new(mapping.outputs['UV'],color.inputs['Vector'])
                alpha=nodes.new('ShaderNodeTexImage');alpha.image=mask;alpha.interpolation='Closest';links.new(mapping.outputs['UV'],alpha.inputs['Vector'])
                emission=nodes.new('ShaderNodeEmission');links.new(color.outputs['Color'],emission.inputs['Color'])
                mix=nodes.new('ShaderNodeMixShader');links.new(alpha.outputs['Color'],mix.inputs[0]);links.new(original,mix.inputs[1]);links.new(emission.outputs[0],mix.inputs[2]);links.new(mix.outputs[0],dest.inputs['Surface'])
                clones[old]=len(obj.data.materials);obj.data.materials.append(mat)
            face.material_index=clones[old];changed_faces.append(face.index)
        for layer_name,values in old_uvs.items():
            if not np.array_equal(values,np.array([loop.uv[:] for loop in obj.data.uv_layers[layer_name].data])):raise ValueError('Original UV changed')
        for image_name,state in old_images.items():
            if image_hash(bpy.data.images[image_name])!=state:raise ValueError('Original image data changed')
        after_tree=surface(rocks);rows=[];new_foreign=[];regressions=[]
        for y in range(103,123):
            for x in range(98,118):
                p,_,_,d=hit(after_tree,x+.5,y+.5);q,_,_,old_d=hit(before_tree,x+.5,y+.5);b,_,_,bd=hit(bank_tree,x+.5,y+.5)
                before_visible=q is not None and(b is None or old_d<bd);after_visible=p is not None and(b is None or d<bd)
                if [x,y] in target_pixels:rows.append(dict(pixel=[x,y],before=before_visible,after=after_visible))
                if after_visible and not before_visible and not domain[y,x]:new_foreign.append([x,y])
                if before_visible and not after_visible:regressions.append([x,y])
        if foreign!={o.name:digest(_geometry(o)) for o in scene.objects if o.type=='MESH' and o!=obj}:raise ValueError('Unrelated geometry changed')
        if other_appearance!={o.name:digest(appearance_state(o,{})) for o in rocks if o!=obj}:raise ValueError('Unrelated rock appearance changed')
        # The approved support audit identifies the actual sloping underside by
        # vertex indices, avoiding a global minimum-Z shortcut on a raised edge.
        approved_support=json.loads((source.parent/'underside-support.json').read_text())
        bottom_triangles={tuple(r['triangle']) for r in approved_support['rows'] if r['kind']=='bottom-face-sample'}
        support_rows=[]
        for triangle in bottom_triangles:
            if not any(i in moved_ids for i in triangle):continue
            for a in range(5):
                for b in range(5-a):
                    point=np.array([a,b,4-a-b])/4@after[list(triangle)]
                    top,_,_,_=bank_tree.ray_cast(Vector((float(point[0]),float(point[1]),2000)),Vector((0,0,-1)))
                    support_rows.append(dict(world=point.tolist(),gap=float(point[2]-top.z) if top is not None else None))
        support_gaps=[r['gap'] for r in support_rows if r['gap'] is not None]
        write_json(output/'support.json',dict(samples=len(support_rows),missing_bank=sum(r['gap'] is None for r in support_rows),positive_gaps=sum(g>.001 for g in support_gaps),max_gap=max(support_gaps) if support_gaps else None,base_geometry_unchanged=not support_rows,prior_support_sha256=sha(source.parent/'underside-support.json'),rows=support_rows))
        bpy.ops.wm.save_as_mainfile(filepath=str(output/'worker.blend'))
        write_json(output/'validation.json',dict(status='Private ledge trial; independent source/contact review required',model_sha256=sha(output/'worker.blend'),source_sha256=sha(source),bank_sha256=sha(bank),moved_vertices=moved,changed_faces=len(changed_faces),profile=profile,maximum_source_x_displacement=max(r['world_dx'] for r in moved),world_y_z_unchanged=bool(np.max(abs(after[:,1:]-before[:,1:]))<.0001),old_uvs_and_image_pixels_exact=True,old_active_uv_preserved=obj.data.uv_layers.active_index==old_active_uv,original_image_count=len(old_images),other_geometry_and_rock_appearance_exact=True,overlay_native_pixels=int(local_domain.sum()),target_pixels=len(rows),target_recovered=sum(r['after'] for r in rows),target_results=rows,new_nonrock_domain_first_hits=new_foreign,regressions=regressions,source_ownership_changed=False,user_approval=None))
        names=list(bank_matrices)
        with bpy.data.libraries.load(str(bank),link=False) as (available,loaded):loaded.objects=names.copy()
        for name_,item in zip(names,loaded.objects):scene.collection.objects.link(item);item.parent=None;item.matrix_world=Matrix(bank_matrices[name_])
        bpy.context.view_layer.update()
        for item in scene.objects:
            if item.type=='MESH':item.hide_render=item not in rocks+loaded.objects
        camera(scene,Vector((896,-576/SIN,0)),Vector(RAY),1792,1152,1792);scene.render.filepath=str(output/'native-contact.png');bpy.ops.render.render(write_still=True,scene=scene.name)
        for i,angle in enumerate([math.pi/4,-math.pi/4]):
            camera(scene,Vector((35,-230,55)),Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN)),640,480,360);scene.render.filepath=str(output/f'oblique-contact-{i}.png');bpy.ops.render.render(write_still=True,scene=scene.name)
        print(json.dumps(dict(recovered=sum(r['after'] for r in rows),target=len(rows),new_foreign=new_foreign,regressions=regressions)))
    finally:release()


if __name__=='__main__':main()
