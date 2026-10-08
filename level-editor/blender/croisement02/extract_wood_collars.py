"""Read-only extraction of bounded retained collar loops from pinned workers.

Run only after the coordinator releases the Blender lane. This script never
saves a Blender file; it writes bounded JSON for subsequent CPU collar fitting.
"""
import argparse,hashlib,json,sys
from pathlib import Path
import bpy,bmesh
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement')]
from render_slots import acquire,release
OUT=ROOT/'level-editor/work/croisement02-refinement'
INPUTS={32:('root-stem-round-3/assets/croisement02-tree-32','00ab00761141d7c4c564331745ae680afef0fd20edd40e110c9301ec986f185b','building-080',[105.,115.,125.,135.]),38:('root-stem-round-2/assets/croisement02-tree-38','b245e87e24341e78415567d31327f353d575bb988da597ae34b8a06178fab054','building-094',[115.,125.,135.,145.])}
MAX_VERTICES_PER_CUT=4096
MAX_OUTPUT_BYTES=8*1024*1024

def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()

def loops_from_edges(edges):
    neighbours={}
    for edge in edges:
        a,b=(v.index for v in edge.verts);neighbours.setdefault(a,set()).add(b);neighbours.setdefault(b,set()).add(a)
    invalid={str(k):len(v) for k,v in neighbours.items() if len(v)!=2}
    if invalid:return [],invalid
    remaining=set(neighbours);loops=[]
    while remaining:
        first=min(remaining);current=first;previous=None;loop=[]
        while True:
            if current not in remaining:raise ValueError('Boundary walk revisited a non-closing vertex')
            loop.append(current);remaining.remove(current)
            following=min(neighbours[current]-({previous} if previous is not None else set()))
            if following==first:break
            previous,current=current,following
        if len(loop)<3:raise ValueError('Boundary loop has fewer than three vertices')
        loops.append(loop)
    return loops,{}

def extract(obj,height):
    bm=bmesh.new()
    try:
        bm.from_mesh(obj.data);bmesh.ops.transform(bm,matrix=obj.matrix_world,verts=list(bm.verts))
        bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),dist=1e-6,plane_co=(0,0,height),plane_no=(0,0,1),clear_inner=True,clear_outer=False)
        bm.verts.ensure_lookup_table();bm.verts.index_update();bm.normal_update()
        edges=[e for e in bm.edges if e.is_boundary and all(abs(v.co.z-height)<1e-4 for v in e.verts)]
        ids={v.index for e in edges for v in e.verts}
        if len(ids)>MAX_VERTICES_PER_CUT:raise ValueError('Boundary exceeds bounded extraction limit')
        loops,invalid=loops_from_edges(edges);vertices={}
        for index in sorted(ids):
            vertex=bm.verts[index];adjacent=sorted({e.other_vert(vertex).index for e in vertex.link_edges if e.other_vert(vertex).co.z>height+1e-4})
            normal=vertex.normal.copy();tangent=Vector((0,0,1))-normal*normal.z
            vertices[str(index)]={'position':list(vertex.co),'geometric_normal':list(normal),'upward_surface_tangent':list(tangent.normalized()) if tangent.length>1e-8 else None,'adjacent_retained_vertices':[{'index':j,'position':list(bm.verts[j].co)} for j in adjacent]}
        return {'height':height,'boundary_edges':len(edges),'boundary_vertices':len(ids),'ordered_loops':loops,'invalid_boundary_degrees':invalid,'vertices':vertices,'status':'closed loops extracted; correspondence/tangent continuity not yet approved' if loops and not invalid else 'unsuitable cut: empty or non-simple boundary','normal_scope':'Geometric retained-face normals after opening the cut; not material shading/custom split normals. No cap was added.'}
    finally:bm.free()

def extract_band(obj,lower,upper):
    """Read exact world triangles intersecting a requested transition band."""
    obj.data.calc_loop_triangles()
    points=[obj.matrix_world@v.co for v in obj.data.vertices]
    triangles=[t for t in obj.data.loop_triangles if min(points[i].z for i in t.vertices)<=upper and max(points[i].z for i in t.vertices)>=lower]
    if len(triangles)>30000:raise ValueError('Local band exceeds triangle extraction bound')
    ids=sorted({i for t in triangles for i in t.vertices});mapping={old:new for new,old in enumerate(ids)}
    return dict(lower=lower,upper=upper,vertices=[list(points[i]) for i in ids],original_vertex_ids=ids,triangles=[[mapping[i] for i in t.vertices] for t in triangles],original_polygon_ids=[t.polygon_index for t in triangles],scope='Exact unmodified world geometry; triangles intersecting the band, not clipped or remeshed')

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--output',type=Path,required=True);parser.add_argument('--tree',type=int,choices=(32,38));parser.add_argument('--cuts',type=float,nargs='+');parser.add_argument('--band',type=float,nargs=2);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    if (args.cuts or args.band) and args.tree is None:raise ValueError('Custom extraction requires one explicit tree')
    if args.cuts and (len(args.cuts)>6 or not all(0<z<500 for z in args.cuts)):raise ValueError('Cuts outside bounded scope')
    if args.band and not 0<args.band[0]<args.band[1]<500:raise ValueError('Invalid local band')
    destination=args.output.resolve()
    if destination.exists():raise FileExistsError(destination)
    expected={n:(OUT/relative/'model.blend',digest,node,args.cuts or cuts) for n,(relative,digest,node,cuts) in INPUTS.items() if args.tree is None or n==args.tree}
    for path,digest,_,_ in expected.values():
        if sha(path)!=digest:raise ValueError('Pinned input changed: '+str(path))
    records=[];acquire()
    try:
        for index,(path,digest,node,cuts) in expected.items():
            bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update()
            matches=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==f'croisement02-tree-{index}' and o.get('source_node')==node and o.get('projection_component')!='crown']
            if len(matches)!=1:raise ValueError('Expected one exact retained wood owner')
            obj=matches[0];records.append({'tree':index,'worker':str(path.parent),'model_sha256':digest,'source_node':node,'object':obj.name,'cuts':[extract(obj,z) for z in cuts]})
            if args.band:records[-1]['band']=extract_band(obj,*args.band)
        for path,digest,_,_ in expected.values():
            if sha(path)!=digest:raise ValueError('Input changed during read-only extraction')
        report={'status':'read-only retained boundary extraction; no mesh/model save or readiness claim','recipe_sha256':sha(Path(__file__)),'max_output_bytes':MAX_OUTPUT_BYTES,'records':records}
        encoded=(json.dumps(report,indent=2)+'\n').encode()
        if len(encoded)>MAX_OUTPUT_BYTES:raise ValueError('JSON exceeds bounded output size')
        destination.parent.mkdir(parents=True,exist_ok=True)
        with destination.open('xb') as handle:handle.write(encoded)
        print(destination,len(encoded))
    finally:release()

if __name__=='__main__':main()
