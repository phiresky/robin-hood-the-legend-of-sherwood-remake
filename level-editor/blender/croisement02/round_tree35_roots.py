"""Private source-ray depth correction for thin inferred oak35 root flares."""
import json,sys
from pathlib import Path
import bpy,bmesh
import numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import RAY
from refinement_workspace import _geometry
from render_multiview_asset import render
from source_projection_bake import bake


def main():
    original=tree_workspace(35);directory=OUT/'tree35-root-research/candidate-v4';directory.mkdir(parents=True,exist_ok=False)
    original_hash=sha(original/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(original/'model.blend'));bpy.context.preferences.filepaths.save_version=0
    objects=list(bpy.data.collections['Croisement02 Working'].all_objects);wood=[o for o in objects if o.type=='MESH' and o.get('asset_group')==original.name and o.get('projection_component')!='crown'];outside={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o not in wood};reports=[]
    ray=np.array(RAY)
    for obj in wood:
        donor=obj.copy();donor.data=obj.data.copy();bpy.context.scene.collection.objects.link(donor);donor.hide_render=True
        coordinates=np.array([tuple(obj.matrix_world@v.co) for v in obj.data.vertices]);normals=np.array([tuple((obj.matrix_world.to_3x3()@v.normal).normalized()) for v in obj.data.vertices]);before=coordinates.copy();z=coordinates[:,2].copy();weight=np.clip((45-z)/35,0,1);weight=weight*weight*(3-2*weight)
        delta=10*weight*(1+np.clip(normals@ray,-1,1))/2
        depth=coordinates@ray;ends=np.array([list(e.vertices) for e in obj.data.edges]);counts=np.bincount(ends.ravel(),minlength=len(depth));current=depth+delta
        for iteration in range(20):
            total=np.bincount(ends[:,0],weights=current[ends[:,1]],minlength=len(depth))+np.bincount(ends[:,1],weights=current[ends[:,0]],minlength=len(depth));average=total/np.maximum(counts,1);current+=.35*weight*(average-current);current=np.maximum(current,depth)
        delta=current-depth;coordinates+=delta[:,None]*ray
        inverse=obj.matrix_world.inverted()
        for vertex,point in zip(obj.data.vertices,coordinates):vertex.co=inverse@Vector(point)
        for face in obj.data.polygons:face.use_smooth=True
        obj.data.update();bm=bmesh.new();bm.from_mesh(obj.data);nonmanifold=sum(not edge.is_manifold for edge in bm.edges);degenerate=sum(face.calc_area()<1e-9 for face in bm.faces);bm.free()
        projected=coordinates-before-np.outer((coordinates-before)@ray,ray)
        if np.abs(projected).max()>1e-4 or nonmanifold or degenerate:raise ValueError('Root correction changed source projection or topology')
        # Union the post-ground deformation before relaxation: earlier voxel
        # joins occurred before that deformation and retained folded ridges.
        bpy.ops.object.select_all(action='DESELECT');obj.select_set(True);bpy.context.view_layer.objects.active=obj
        modifier=obj.modifiers.new('Continuous root wood union','REMESH');modifier.mode='VOXEL';modifier.voxel_size=.65;modifier.use_smooth_shade=True;bpy.ops.object.modifier_apply(modifier=modifier.name)
        group=obj.vertex_groups.new(name='Inferred low root relaxation')
        for v in obj.data.vertices:
            wz=(obj.matrix_world@v.co).z;amount=max(0.,min(1.,(45-wz)/30))
            if amount:group.add([v.index],amount,'REPLACE')
        modifier=obj.modifiers.new('Round low root joins','SMOOTH');modifier.vertex_group=group.name;modifier.factor=.45;modifier.iterations=30;bpy.ops.object.modifier_apply(modifier=modifier.name)
        modifier=obj.modifiers.new('Retain approved wood mapping','DATA_TRANSFER');modifier.object=donor;modifier.use_loop_data=True;modifier.data_types_loops={'UV'};modifier.loop_mapping='POLYINTERP_NEAREST'
        bpy.ops.object.datalayout_transfer(modifier=modifier.name);bpy.ops.object.modifier_apply(modifier=modifier.name)
        old_surface=BVHTree.FromPolygons([v.co for v in donor.data.vertices],[list(f.vertices) for f in donor.data.polygons])
        for face in obj.data.polygons:
            face.use_smooth=True;hit=old_surface.find_nearest(face.center)
            if hit[2] is None:raise ValueError('No approved material donor face')
            face.material_index=donor.data.polygons[hit[2]].material_index
        bpy.data.objects.remove(donor,do_unlink=True)
        bm=bmesh.new();bm.from_mesh(obj.data);joined_nonmanifold=sum(not e.is_manifold for e in bm.edges);joined_degenerate=sum(f.calc_area()<1e-9 for f in bm.faces);bm.free()
        if joined_nonmanifold or joined_degenerate:raise ValueError('Joined roots invalid')
        reports.append(dict(object=obj.name,vertices=len(coordinates),changed_vertices=int(np.count_nonzero(delta>1e-5)),max_depth_shift=float(delta.max()),max_screen_coordinate_shift=float(np.abs(projected).max()),upper_vertices_unchanged_before_union=bool(np.all(delta[z>=45]==0)),boundary_edges=joined_nonmanifold,degenerate_faces=joined_degenerate,joined_vertices=len(obj.data.vertices),min_z_before=float(z.min()),min_z_after=float(coordinates[:,2].min())))
    if outside!={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o not in wood}:raise ValueError('Outside scope changed')
    config=json.loads((original/'workspace.json').read_text())
    bake('Croisement02',config['source_path'],directory/'source-ownership.json',receiver_nodes=sorted({o['source_node'] for o in wood}),receiver_object_names=[o.name for o in wood],occluder_nodes=sorted({o['source_node'] for o in wood}),projection_label='exterior',preserve_authored=False,source_mask_manifest=config['source_mask_manifest'])
    if outside!={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o not in wood}:raise ValueError('Source bake changed crown or another owner')
    bpy.ops.wm.save_as_mainfile(filepath=str(directory/'model.blend'))
    packet=json.loads((OUT/'tree35-root-research/baseline-v2/views.json').read_text());packet['source_blend']=str(directory/'model.blend');write_json(directory/'views.json',packet)
    for obj in objects:
        if obj.type=='MESH' and obj.get('asset_group')==original.name:obj.hide_render=obj not in wood
    scene=bpy.data.scenes[packet['scene_name']];scene.render.engine='CYCLES';scene.cycles.samples=8
    render(directory/'views.json',directory/'views',modes=('textured','solid'),width=320)
    for mode in ['textured','solid']:
        sheet=Image.new('RGB',(1280,640))
        for i in range(8):sheet.paste(Image.open(directory/f'views/view-{i}-{mode}.png'),((i%4)*320,(i//4)*320))
        sheet.save(directory/f'{mode}.png')
    if sha(original/'model.blend')!=original_hash:raise ValueError('Approved model changed')
    write_json(directory/'evidence.json',dict(status='private root-depth experiment; not approved',model_sha256=sha(directory/'model.blend'),original_model=str(original/'model.blend'),original_model_sha256=original_hash,root_geometry=reports,outside_appearance_unchanged=True,crown_and_other_owner_appearance_unmodified=True,changed_wood_uses_native_source_projection=True,source_screen_coordinates_preserved_before_union=True,reference_assets=['leicester-southeast-cottage-tree','leicester-moat-bank-tree'],limitations=['Depth inference and relaxation affect low roots; .65 voxel union also resamples upper wood. Native view must be checked for front-surface preservation.','Changed wood uses native-source projection; unseen surfaces remain neutral for future fill, without transferring prior texture approval.','No canonical worker or user approval changed.']))

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
