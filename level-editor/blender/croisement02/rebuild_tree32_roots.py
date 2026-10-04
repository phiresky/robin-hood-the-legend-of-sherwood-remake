"""Private native-profile forked roots with source-ray ground support."""
import json,math,sys
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image
from mathutils import Vector,Matrix
from mathutils.kdtree import KDTree
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import _geometry
from tree_geometry import SIN,COS,RAY


def loft(rows,ground,cut,n=48):
    vertices=[];faces=[]
    kernel=np.array([1,2,3,2,1],float);kernel/=kernel.sum()
    centers=np.convolve(np.pad([r[1] for r in rows],2,mode='edge'),kernel,mode='valid')
    radii=np.convolve(np.pad([r[2] for r in rows],2,mode='edge'),kernel,mode='valid')
    for (y,_,_),cx,radius in zip(rows,centers,radii):
        center=Vector((cx,-ground/SIN,(ground-y)/COS));depth=center.dot(RAY)
        if y>=715:depth+=max(0.,(.25+radius*RAY.z-center.z)/RAY.z)
        for j in range(n):
            angle=math.tau*j/n;source=Vector((cx+radius*math.cos(angle),-y*SIN,-y*COS));d=depth+radius*math.sin(angle)
            d=min(d,(cut-.25-source.z)/RAY.z)
            vertices.append(tuple(source+RAY*d))
    faces.append(tuple(reversed(range(n))))
    for i in range(len(rows)-1):
        for j in range(n):a=i*n+j;b=i*n+(j+1)%n;faces.append((a,b,b+n,a+n))
    faces.append(tuple((len(rows)-1)*n+j for j in range(n)))
    return vertices,faces


def add_mesh(bm,geometry):
    mesh=bpy.data.meshes.new('Private native root lobe');mesh.from_pydata(geometry[0],[],geometry[1]);mesh.update();bm.from_mesh(mesh);bpy.data.meshes.remove(mesh)


def check(mesh):
    bm=bmesh.new();bm.from_mesh(mesh);report=dict(vertices=len(bm.verts),faces=len(bm.faces),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-9 for f in bm.faces));bm.free()
    if report['nonmanifold_edges'] or report['degenerate_faces']:raise ValueError(report)
    return report


def main():
    old=tree_workspace(32);out=OUT/'tree32-root-research/continuous-fork-v2';out.mkdir(parents=True,exist_ok=False);old_hash=sha(old/'model.blend')
    bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'));bpy.context.view_layer.update();bpy.context.preferences.filepaths.save_version=0
    objects=list(bpy.data.collections['Croisement02 Working'].all_objects);wood={int(o['source_node'].split('-')[-1]):o for o in objects if o.type=='MESH' and o.get('asset_group')==old.name and o.get('projection_component')!='crown'}
    if set(wood)!={80,81,82}:raise ValueError('Unexpected native wood owners')
    protected={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o not in wood.values()}
    cut=100.;junction=dict(method='Closed union of original upper80 aboveZ75 and native-profile fork; no single-loop assumption')
    upper_positions=[wood[80].matrix_world@v.co for v in wood[80].data.vertices if (wood[80].matrix_world@v.co).z>120]
    record=next(r for r in json.loads((OUT/'baseline/masks/manifest.json').read_text())['masks'] if r['index']==32)
    mask=np.asarray(Image.open(OUT/'baseline/masks'/record['png']).convert('L'))>0;ox,oy=record['box_top_left']
    ground=next(r['ground_y'] for r in json.loads((OUT/'forest-v4-sources/manifest.json').read_text()) if r['mask']==32)
    profiles={};bm=bmesh.new()
    for role,start,end in [('stem',660,727),('left-root',714,750),('right-root',714,757)]:
        rows=[]
        for y in range(start,end):
            xs=np.where(mask[y-oy])[0]+ox;xs=xs[(xs>=1035)&(xs<=1115)]
            if role=='left-root':xs=xs[xs<=round(1084-min(1.,(y-714)/14)*12)]
            elif role=='right-root':xs=xs[xs>=round(1070+min(1.,(y-714)/14)*3)];xs=xs[xs<=1095] if y>=728 else xs
            if len(xs)<2:continue
            left,right=float(xs.min())-.5,float(xs.max())+.5;rows.append((y,(left+right)/2,(right-left)/2))
        profiles[role]=rows;add_mesh(bm,loft(rows,ground,cut))
    retained=bmesh.new();retained.from_mesh(wood[80].data);bmesh.ops.transform(retained,matrix=wood[80].matrix_world,verts=list(retained.verts))
    bmesh.ops.bisect_plane(retained,geom=list(retained.verts)+list(retained.edges)+list(retained.faces),dist=.0001,plane_co=(0,0,75),plane_no=(0,0,1),clear_inner=True)
    bmesh.ops.holes_fill(retained,edges=[e for e in retained.edges if e.is_boundary],sides=0)
    retained_mesh=bpy.data.meshes.new('Private retained upper wood');retained.to_mesh(retained_mesh);retained.free();bm.from_mesh(retained_mesh);bpy.data.meshes.remove(retained_mesh)
    mesh=bpy.data.meshes.new('Tree32 continuous lower fork union');bm.to_mesh(mesh);bm.free();temp=bpy.data.objects.new(mesh.name,mesh);bpy.context.scene.collection.objects.link(temp)
    bpy.ops.object.select_all(action='DESELECT');temp.select_set(True);bpy.context.view_layer.objects.active=temp
    modifier=temp.modifiers.new('Join overlapping native root lobes','REMESH');modifier.mode='VOXEL';modifier.voxel_size=.6;modifier.use_smooth_shade=True;bpy.ops.object.modifier_apply(modifier=modifier.name)
    group=temp.vertex_groups.new(name='Lower root joins only')
    for v in temp.data.vertices:group.add([v.index],max(0.,min(1.,(120-v.co.z)/30)),'REPLACE')
    modifier=temp.modifiers.new('Relax local union joints','SMOOTH');modifier.vertex_group=group.name;modifier.factor=.3;modifier.iterations=3;bpy.ops.object.modifier_apply(modifier=modifier.name)
    combined=temp.data.copy();bpy.data.objects.remove(temp,do_unlink=True)
    full_report=check(combined)
    for f in combined.polygons:f.use_smooth=True
    normals=KDTree(len(combined.vertices));normal_values=[]
    for v in combined.vertices:normals.insert(v.co,v.index);normal_values.append(tuple(v.normal))
    normals.balance();reports={}
    neutral=bpy.data.materials.new('Unprojected private wood32');neutral.diffuse_color=(.4,.4,.4,1)
    for part,obj in wood.items():
        bm=bmesh.new();bm.from_mesh(combined)
        bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),dist=.0001,plane_co=(0,0,45),plane_no=(0,0,1),clear_inner=part==80,clear_outer=part!=80)
        bmesh.ops.holes_fill(bm,edges=[e for e in bm.edges if e.is_boundary],sides=0)
        if part!=80:
            bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),dist=.0001,plane_co=(1072,0,0),plane_no=(1,0,0),clear_inner=part==81,clear_outer=part==82)
            bmesh.ops.holes_fill(bm,edges=[e for e in bm.edges if e.is_boundary],sides=0)
        bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));mesh=bpy.data.meshes.new(f'Tree32 continuous wood owner{part}');bm.to_mesh(mesh);bm.free();obj.data=mesh;obj.parent=None;obj.matrix_world=Matrix.Identity(4);mesh.materials.append(neutral)
        for f in mesh.polygons:f.use_smooth=True
        mesh.normals_split_custom_set_from_vertices([normal_values[normals.find(v.co)[1]] for v in mesh.vertices]);reports[part]=check(mesh)
    surface=BVHTree.FromPolygons([v.co for v in combined.vertices],[list(f.vertices) for f in combined.polygons]);distances=[surface.find_nearest(p)[3] for p in upper_positions]
    junction['upper_vertex_to_surface_distance']=dict(samples=len(distances),maximum=max(distances),p95=float(np.quantile(distances,.95)))
    if max(distances)>1.5:raise ValueError('Upper wood surface drift exceeds private prototype tolerance')
    if protected!={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o not in wood.values()}:raise ValueError('Crown or other asset changed')
    bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'))
    if sha(old/'model.blend')!=old_hash:raise ValueError('Approved input changed')
    write_json(out/'evidence.json',dict(model_sha256=sha(out/'model.blend'),previous_worker=str(old),previous_model_sha256=old_hash,status='Private unprojected geometry; requires source and solid review',source_profiles=profiles,upper_junction=junction,full_geometry=full_report,parts=reports,preserved_appearance=protected,source_ray_support=True,partition='Native80 aboveZ45,81/82 belowZ45 partitioned atX1072; internal faces only',limitations=['No source32 pixels reclassified as leaves.','Hidden root volume and internal native-owner split are geometric inference.','Upper80 shape has bounded voxel-union deviation; crown and other assets exactly preserved; no approval inherited.']))
    (out/'recipe.py').write_text(Path(__file__).read_text())
    import inspect_tree07_base
    previous=sys.argv;sys.argv=[sys.argv[0],'--','--mask','32','--model',str(out/'model.blend'),'--output-name','continuous-fork-v2-review','--solid-only']
    try:inspect_tree07_base.main()
    finally:sys.argv=previous

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
