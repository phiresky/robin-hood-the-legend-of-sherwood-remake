"""Read selected timber receivers and their existing ground without copying a scene."""
import hashlib,json,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';SOURCE=WORK/'grounding/york-grounded.blend';OUT=WORK/'restart2/timber-props-source-study-v1/support-preflight-v1.json'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
groups=('Riverside timber stack','Riverside loose planks','West town raised terrain')
with bpy.data.libraries.load(str(SOURCE),link=False) as (source,destination):
    destination.objects=[n for n in source.objects if n in groups or n.startswith(tuple(g+' / ' for g in groups))]
for obj in destination.objects:bpy.context.scene.collection.objects.link(obj)
bpy.context.view_layer.update()
ground=[o for o in destination.objects if o.type=='MESH' and not o.hide_render and o.get('source_node')=='building-086'];assert ground
vertices=[];faces=[]
for obj in ground:
    offset=len(vertices);vertices.extend(obj.matrix_world@v.co for v in obj.data.vertices);faces.extend(tuple(offset+i for i in p.vertices)for p in obj.data.polygons)
tree=BVHTree.FromPolygons(vertices,faces);rows=[]
for node in ('building-007','building-008'):
    objects=[o for o in destination.objects if o.type=='MESH'and not o.hide_render and o.get('source_node')==node];assert len(objects)==1
    obj=objects[0];points=[obj.matrix_world@v.co for v in obj.data.vertices];bottom=min(v.z for v in points);feet=[v for v in points if abs(v.z-bottom)<.002];samples=[]
    for p in feet:
        hit=tree.ray_cast(Vector((p.x,p.y,1000)),Vector((0,0,-1)))
        samples.append({'foot_world':list(p),'ground_hit':list(hit[0])if hit[0]is not None else None,'normal':list(hit[1])if hit[1]is not None else None,'foot_minus_receiver_z':p.z-hit[0].z if hit[0]is not None else None})
    rows.append({'source_node':node,'object':obj.name,'parent':obj.parent.name,'vertices_world':[list(v)for v in points],'faces':[list(p.vertices)for p in obj.data.polygons],'bottom_vertex_samples':samples})
report={'status':'Read-only baseline support evidence; not refined geometry or approved footprints','source_sha256':hashlib.sha256(SOURCE.read_bytes()).hexdigest(),'ground_objects':[o.name for o in ground],'selection':'Only three named asset groups and their objects linked; evaluated after parent groups were linked','props':rows,'limitations':['Original proxy footprint is a constraint, not a semantic board/log outline.','Hidden import duplicates excluded.','New board feet must be checked against these actual surfaces after source tracing.']};OUT.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps([{'source_node':r['source_node'],'sample_count':len(r['bottom_vertex_samples']),'misses':sum(s['ground_hit']is None for s in r['bottom_vertex_samples']),'deltas':[s['foot_minus_receiver_z']for s in r['bottom_vertex_samples']]}for r in rows]))
