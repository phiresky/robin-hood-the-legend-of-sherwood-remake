"""Replace foliage-derived oak root flares with inferred grounded round roots."""
import argparse,json,math,sys
from pathlib import Path
import bpy,bmesh
from mathutils import Vector,Matrix
from mathutils.kdtree import KDTree
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json
from refinement_workspace import _geometry
from render_slots import acquire,release
from source_projection_bake import bake
from render_multiview_asset import render


def tube(path,n=24):
    vertices=[];faces=[]
    for i,row in enumerate(path):
        center=Vector(row[:3]);radius=row[3];tangent=(Vector(path[min(i+1,len(path)-1)][:3])-Vector(path[max(i-1,0)][:3])).normalized();axis=Vector((0,0,1))
        if abs(tangent.dot(axis))>.95:axis=Vector((1,0,0))
        u=tangent.cross(axis).normalized();v=tangent.cross(u).normalized()
        for j in range(n):vertices.append(tuple(center+radius*(u*math.cos(math.tau*j/n)+v*math.sin(math.tau*j/n))))
    faces.append(tuple(reversed(range(n))))
    for i in range(len(path)-1):
        for j in range(n):a=i*n+j;b=i*n+(j+1)%n;faces.append((a,b,b+n,a+n))
    faces.append(tuple((len(path)-1)*n+j for j in range(n)))
    return vertices,faces


def vertical_base(rows,n=32):
    vertices=[];faces=[]
    for x,y,z,rx,ry in rows:
        vertices.extend((x+rx*math.cos(math.tau*j/n),y+ry*math.sin(math.tau*j/n),z) for j in range(n))
    faces.append(tuple(reversed(range(n))))
    for i in range(len(rows)-1):
        for j in range(n):a=i*n+j;b=i*n+(j+1)%n;faces.append((a,b,b+n,a+n))
    faces.append(tuple((len(rows)-1)*n+j for j in range(n)))
    return vertices,faces


def bridge_base(bm, rows, cut):
    """Loft the rounded base directly to the retained stem's cut boundary."""
    boundary=[v for v in bm.verts if abs(v.co.z-cut)<.001 and any(e.is_boundary for e in v.link_edges)]
    if len(boundary)<8:raise ValueError('Missing retained trunk boundary')
    cx=sum(v.co.x for v in boundary)/len(boundary);cy=sum(v.co.y for v in boundary)/len(boundary)
    boundary.sort(key=lambda v:math.atan2(v.co.y-cy,v.co.x-cx))
    angles=[math.atan2(v.co.y-cy,v.co.x-cx) for v in boundary];rings=[]
    for x,y,z,rx,ry in rows:
        verts=[]
        for a in angles:
            # Broad angular buttresses grow out of the stem, with a smooth
            # vertical decay; no separately attached conical root meshes.
            lobes=sum(length*math.exp(-.5*(math.atan2(math.sin(a-direction),math.cos(a-direction))/.32)**2) for direction,length in [(0.1,14),(1.8,9),(3.7,19),(5.1,12)])
            flare=lobes*max(0.,1-z/48)**2
            verts.append(bm.verts.new((x+(rx+flare)*math.cos(a),y+(ry+flare)*math.sin(a),z)))
        rings.append(verts)
    bm.faces.new(tuple(reversed(rings[0])))
    rings.append(boundary)
    for lower,upper in zip(rings,rings[1:]):
        for i in range(len(boundary)):j=(i+1)%len(boundary);bm.faces.new((lower[i],lower[j],upper[j],upper[i]))


def append_mesh(bm,geometry):
    mesh=bpy.data.meshes.new('Private root primitive');mesh.from_pydata(geometry[0],[],geometry[1]);mesh.update();bm.from_mesh(mesh);bpy.data.meshes.remove(mesh)


def main(solid_only=False,output_name="candidate-v13"):
    original=tree_workspace(35);directory=OUT/'tree35-root-research'/output_name;directory.mkdir(exist_ok=False);original_hash=sha(original/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(original/'model.blend'));bpy.context.preferences.filepaths.save_version=0
    objects=list(bpy.data.collections['Croisement02 Working'].all_objects);wood=[o for o in objects if o.type=='MESH' and o.get('asset_group')==original.name and o.get('projection_component')!='crown'];outside={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o not in wood};records=[]
    for obj in wood:
        part=int(obj['source_node'].split('-')[-1])
        if part==91:continue
        cut=80
        bm=bmesh.new();bm.from_mesh(obj.data);bmesh.ops.transform(bm,matrix=obj.matrix_world,verts=list(bm.verts));bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),dist=.0001,plane_co=(0,0,cut),plane_no=(0,0,1),clear_inner=True,clear_outer=False)
        boundary=[e for e in bm.edges if e.is_boundary]
        if part!=90:bmesh.ops.holes_fill(bm,edges=boundary,sides=0)
        if part==90:
            base=[(1406,-1430,.05,26,28),(1406,-1430,3,25,27),(1406,-1430,8,24,26),(1405,-1430,16,22,24),(1405,-1430,27,20,22),(1405,-1430,45,19,21),(1407,-1430,65,22,24)]
            paths=[]
            bridge_base(bm,base,cut)
        else:
            base=[(1388,-1430,5,10,11),(1388,-1430,15,10,11),(1387,-1430,27,8,9),(1385,-1430,40,5,6),(1384,-1430,49,1,1)]
            paths=[]
            append_mesh(bm,vertical_base(base))
        for path in paths:append_mesh(bm,tube(path))
        bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));mesh=bpy.data.meshes.new('Rounded inferred root base');bm.to_mesh(mesh);bm.free();obj.data=mesh;obj.matrix_world=Matrix.Identity(4)
        bpy.ops.object.select_all(action='DESELECT');obj.select_set(True);bpy.context.view_layer.objects.active=obj
        mod=obj.modifiers.new('Continuous trunk and grounded root union','REMESH');mod.mode='VOXEL';mod.voxel_size=.7;mod.use_smooth_shade=True;bpy.ops.object.modifier_apply(modifier=mod.name)
        group=obj.vertex_groups.new(name='Rounded joins')
        for vertex in obj.data.vertices:group.add([vertex.index],1. if vertex.co.z<55 else .4,'REPLACE')
        mod=obj.modifiers.new('Relax inferred joins and inherited collars','SMOOTH');mod.vertex_group=group.name;mod.factor=.35;mod.iterations=40;bpy.ops.object.modifier_apply(modifier=mod.name)
        bm=bmesh.new();bm.from_mesh(obj.data);nonmanifold=sum(not e.is_manifold for e in bm.edges);degenerate=sum(f.calc_area()<1e-9 for f in bm.faces);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(obj.data);bm.free()
        if nonmanifold or degenerate:raise ValueError('Invalid root union')
        for face in obj.data.polygons:face.use_smooth=True
        records.append(dict(part=part,cut_height=cut,base_profile=base,inferred_root_paths=paths,vertices=len(obj.data.vertices),faces=len(obj.data.polygons),nonmanifold_edges=nonmanifold,degenerate_faces=degenerate,min_z=min(v.co.z for v in obj.data.vertices)))
    # Split the one continuous exterior into two closed native-part owners.
    # Their shared cut lies inside the trunk; neither part adds an outer patch.
    mainwood=next(o for o in wood if o['source_node'].endswith('090'))
    sidewood=next(o for o in wood if o['source_node'].endswith('091'))
    normal_tree=KDTree(len(mainwood.data.vertices));surface_normals=[]
    for vertex in mainwood.data.vertices:
        normal_tree.insert(vertex.co,vertex.index);surface_normals.append(tuple(vertex.normal))
    normal_tree.balance()
    sidewood.data=mainwood.data.copy();sidewood.matrix_world=Matrix.Identity(4)
    for obj,keep_left in [(mainwood,False),(sidewood,True)]:
        bm=bmesh.new();bm.from_mesh(obj.data)
        bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),dist=.0001,plane_co=(1384,0,0),plane_no=(1,0,1),clear_inner=not keep_left,clear_outer=keep_left)
        bmesh.ops.holes_fill(bm,edges=[e for e in bm.edges if e.is_boundary],sides=0)
        bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(obj.data);bm.free()
        # Identical exterior normals on either side of the internal partition
        # avoid an artificial visible seam between native source owners.
        obj.data.normals_split_custom_set_from_vertices([surface_normals[normal_tree.find(v.co)[1]] for v in obj.data.vertices])
    records=[dict(part=int(o['source_node'].split('-')[-1]),vertices=len(o.data.vertices),faces=len(o.data.polygons),min_z=min(v.co.z for v in o.data.vertices),method='Continuous angular buttress loft, partitioned by internal x+z=1384 plane') for o in wood]
    cfg=json.loads((original/'workspace.json').read_text());domain=OUT/'tree35-root-research/source-domain-v1'
    if not solid_only:
        bake('Croisement02',cfg['source_path'],directory/'source-ownership.json',receiver_nodes=sorted({o['source_node'] for o in wood}),receiver_object_names=[o.name for o in wood],occluder_nodes=sorted({o['source_node'] for o in wood}),projection_label='exterior',preserve_authored=False,source_mask_manifest=str(domain/'source-masks.json'),provenance_directory=str(directory/'source-provenance'))
    if outside!={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o not in wood}:raise ValueError('Crown or other owner changed')
    bpy.ops.wm.save_as_mainfile(filepath=str(directory/'model.blend'))
    scene=bpy.data.scenes['Croisement02 Refinement'];scene.render.engine='CYCLES';scene.cycles.samples=8
    for scope,path in [('roots',OUT/'tree35-root-research/baseline-v2/views.json'),('full',original/'modified/views.json')]:
        if solid_only and scope=='full':continue
        packet=json.loads(path.read_text());packet.pop('render_object_names',None);packet['source_blend']=str(directory/'model.blend')
        for view in packet['views']:view['crop']=dict(width=320,height=320)
        write_json(directory/f'{scope}-views.json',packet)
        for obj in objects:
            if obj.type=='MESH' and obj.get('asset_group')==original.name:obj.hide_render=scope=='roots' and obj not in wood
        render(directory/f'{scope}-views.json',directory/scope,modes=('solid',) if solid_only else ('textured','solid'),width=320)
        for mode in (['solid'] if solid_only else ['textured','solid']):
            sheet=Image.new('RGB',(1280,640))
            for i in range(8):sheet.paste(Image.open(directory/f'{scope}/view-{i}-{mode}.png'),((i%4)*320,(i//4)*320))
            sheet.save(directory/f'{scope}-{mode}.png')
    if sha(original/'model.blend')!=original_hash:raise ValueError('Approved original changed')
    write_json(directory/'evidence.json',dict(status='private reconstructed roots; self-review pending',solid_only_preview=solid_only,model_sha256=sha(directory/'model.blend'),original_model=str(original/'model.blend'),original_model_sha256=original_hash,outside_appearance_fingerprints=outside,root_geometry=records,source_domain_sha256=sha(domain/'source-review.json'),crown_and_other_owner_appearance_unchanged=True,reference_assets=['leicester-southeast-cottage-tree','leicester-moat-bank-tree'],limitations=['Old low flares discarded. New rounded buttresses are continuous angular stem profiles, inferred from grounded trunk construction rather than foliage silhouettes.','Actual visible wood above cut retained before union; modest smoothing also relaxes inherited upper collars.','Gray hidden wood requires new approval and fill; no prior texture approval transferred.','No canonical or approved files changed.']))

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--solid-only',action='store_true');parser.add_argument('--output-name',default='candidate-v13');args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    acquire()
    try:main(args.solid_only,args.output_name)
    finally:release()
