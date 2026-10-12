"""Independently verify terrain removal, retained UVs and world placement."""
import json
import math
import sys
from pathlib import Path

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/active/york'
sys.path.insert(0,str(Path(__file__).parent))
from terrain_clip import area, cross, dot
from floor_contacts import height_at


def coordinates(p,a,b,c):
    u,v,w=([q[k]-a[k] for k in range(3)] for q in (b,c,p))
    uu,uv,vv,uw,vw=dot(u,u),dot(u,v),dot(v,v),dot(u,w),dot(v,w)
    det=uu*vv-uv*uv
    if abs(det)<1e-14:return None
    y,z=(vv*uw-uv*vw)/det,(uu*vw-uv*uw)/det
    distance=math.sqrt(sum((w[k]-y*u[k]-z*v[k])**2 for k in range(3)))
    return (1-y-z,y,z),distance


def triangles(obj):
    obj.data.calc_loop_triangles()
    layers=list(obj.data.uv_layers)
    return [([tuple(obj.matrix_world@obj.data.vertices[v].co) for v in t.vertices],
             [tuple(x for layer in layers for x in layer.data[i].uv) for i in t.loops],t.material_index)
            for t in obj.data.loop_triangles]


def terrain_height(x,y,tri):
    a,b,c=tri
    d=(b[1]-c[1])*(a[0]-c[0])+(c[0]-b[0])*(a[1]-c[1])
    if abs(d)<1e-8:return None
    u=((b[1]-c[1])*(x-c[0])+(c[0]-b[0])*(y-c[1]))/d
    v=((c[1]-a[1])*(x-c[0])+(a[0]-c[0])*(y-c[1]))/d
    if min(u,v,1-u-v)<-1e-7:return None
    return u*a[2]+v*b[2]+(1-u-v)*c[2]


def main():
    import bpy
    bpy.ops.wm.open_mainfile(filepath=str(OUT/'grounding/york-grounded.blend'))
    bpy.context.window.scene=bpy.data.scenes['york Refinement']
    bpy.data.collections['york Grounding Originals'].hide_viewport=False
    originals=list(bpy.data.collections['york Grounding Originals'].objects)
    for obj in originals:obj.hide_viewport=False
    bpy.context.view_layer.update()
    working=bpy.data.collections['york Working']
    report=json.loads((OUT/'grounding/report.json').read_text())
    evidence=json.loads((OUT/'grounding/removed-surfaces.json').read_text())
    supports={}
    for obj in working.objects:
        if obj.get('source_node') in report['support_sources'] and obj.type=='MESH' and not obj.hide_render:
            supports[obj['source_node']]=[p for p,_,_ in triangles(obj)
                if abs(cross([p[1][k]-p[0][k] for k in range(3)],[p[2][k]-p[0][k] for k in range(3)])[2])>1e-6]
    removed_samples=0
    floors = report.get('floor_extensions', {})
    for floor in floors.values():
        supports[floor['source']] = floor['triangles']
    for item in evidence:
        for polygon in item['polygons']:
            if polygon['support'].startswith('asset-floor:'):
                if polygon['support'] != floors.get(item['asset'], {}).get('source'):
                    raise ValueError('Floor continuation applied to another asset')
            vertices=polygon['vertices'];p=[sum(v[k]for v in vertices)/len(vertices) for k in range(3)]
            heights=[z for tri in supports[polygon['support']] if (z:=terrain_height(p[0],p[1],tri)) is not None]
            if not heights or p[2]<-.002 or p[2]>max(heights)+.002:
                raise ValueError('Removed an exposed surface: '+item['object'])
            removed_samples+=1
    max_distance=max_uv=0.0;samples=0
    changes={r['object']:r for r in report['changes']}
    for name,record in changes.items():
        obj=working.objects[name]
        old=next(o for o in originals if o.get('source_node')==record['source_node'] and
                 o.get('projection_component')==record['component'])
        source=triangles(old);remaining=triangles(obj)
        source_area=sum(area(p) for p,_,_ in source)
        retained_area=sum(area(p) for p,_,_ in remaining)
        if abs(source_area-retained_area-record['removed_area'])>max(.1,source_area*2e-6):
            raise ValueError('Surface area mismatch after Blender storage: '+name)
        for points,uvs,material in remaining:
            for weights in [(1/3,1/3,1/3),(.8,.1,.1),(.1,.8,.1),(.1,.1,.8)]:
                p=[sum(v[k]*w for v,w in zip(points,weights)) for k in range(3)]
                uv=[sum(v[k]*w for v,w in zip(uvs,weights)) for k in range(len(uvs[0]))]
                candidates=[]
                for original,mapped,slot in source:
                    if slot!=material:continue
                    found=coordinates(p,*original)
                    locations=[]
                    if found is not None and min(found[0])>=-1e-4:
                        locations.append(found)
                    # Float32 storage can move a point just outside a very thin
                    # triangle. Measure its distance to the finite edges too.
                    for i,j in [(0,1),(1,2),(2,0)]:
                        edge=[original[j][k]-original[i][k] for k in range(3)]
                        denominator=dot(edge,edge)
                        t=max(0,min(1,dot([p[k]-original[i][k] for k in range(3)],edge)/denominator)) if denominator else 0
                        distance=math.sqrt(sum((p[k]-original[i][k]-t*edge[k])**2 for k in range(3)))
                        bary=[0,0,0];bary[i]=1-t;bary[j]=t
                        locations.append((bary,distance))
                    for bary,distance in locations:
                        if distance>.003:continue
                        expected=[sum(v[k]*w for v,w in zip(mapped,bary)) for k in range(len(uv))]
                        error=max((abs(a-b) for a,b in zip(uv,expected)),default=0)
                        candidates.append((error,distance))
                if not candidates:raise ValueError('Retained surface moved: '+name+' point '+str(p)+' closest '+str(sorted((coordinates(p,*xyz) for xyz,_,slot in source if slot==material and coordinates(p,*xyz)),key=lambda x:x[1])[:3]))
                error,distance=min(candidates)
                if error>2e-5:raise ValueError('Texture coordinates changed: '+name)
                max_distance=max(max_distance,distance);max_uv=max(max_uv,error);samples+=1
    floor_vertices = 0
    for obj in working.objects:
        if obj.type != 'MESH' or obj.hide_render or obj.get('asset_group') not in floors:
            continue
        floor = floors[obj['asset_group']]
        for v in obj.data.vertices:
            point = obj.matrix_world @ v.co
            if point.z < height_at(floor, point.x, point.y)-.003:
                raise ValueError('Below reviewed floor: '+obj.name)
            floor_vertices += 1
    result={'status':'PASS','changed_meshes':len(changes),'retained_surface_samples':samples,
            'floor_continuations_checked':len(floors),'floor_vertices_checked':floor_vertices,
            'removed_surface_samples':removed_samples,'max_surface_distance':max_distance,'max_uv_error':max_uv}
    (OUT/'grounding/verification.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))


if __name__=='__main__':main()
