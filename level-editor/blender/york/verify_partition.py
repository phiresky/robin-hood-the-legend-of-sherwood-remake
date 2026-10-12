"""Check split shared surfaces and interpolated UVs against the retained source."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/active/york'


def main():
    import bpy
    from mathutils.geometry import closest_point_on_tri
    bpy.ops.wm.open_mainfile(filepath=str(OUT / 'grouped/york-grouped.blend'))
    catalog = json.loads((ROOT/'level-editor/refinement/catalogs/york.json').read_text())
    results = []
    for spec in catalog['partitions']:
        objects = [o for o in bpy.data.collections['york Working'].objects
                   if o.type == 'MESH' and o.get('source_node') == spec['source_node']]
        original = next(o for o in objects if not o.get('projection_component'))
        bpy.context.window.scene = bpy.data.scenes['york Refinement']
        original.hide_viewport = False
        bpy.context.view_layer.update()
        original.data.calc_loop_triangles()
        source = []
        for tri in original.data.loop_triangles:
            positions = [original.matrix_world @ original.data.vertices[i].co for i in tri.vertices]
            uvs = [original.data.uv_layers.active.data[i].uv.copy() for i in tri.loops]
            source.append((positions, uvs, tri.material_index))
        count = 0
        maximum_distance = maximum_uv = 0.0
        for obj in objects:
            if obj == original:
                continue
            obj.data.calc_loop_triangles()
            for tri in obj.data.loop_triangles:
                points = [obj.matrix_world @ obj.data.vertices[i].co for i in tri.vertices]
                uvs = [obj.data.uv_layers.active.data[i].uv.copy() for i in tri.loops]
                # Interior samples avoid ambiguity at original UV seams.
                for weights in [(1/3,1/3,1/3),(.8,.1,.1),(.1,.8,.1),(.1,.1,.8)]:
                    p = sum((v*w for v,w in zip(points,weights)), points[0]*0)
                    uv = sum((v*w for v,w in zip(uvs,weights)), uvs[0]*0)
                    candidates = []
                    for (a,b,c), mapped, material in source:
                        if material != tri.material_index:
                            continue
                        # Double precision avoids cancellation on long, thin source triangles.
                        x,y,z = [tuple(float(q[i])-float(a[i]) for i in range(3)) for q in (b,c,p)]
                        dot = lambda u,v: sum(j*k for j,k in zip(u,v))
                        xx,xy,yy,xz,yz = dot(x,x),dot(x,y),dot(y,y),dot(x,z),dot(y,z)
                        determinant = xx*yy-xy*xy
                        if abs(determinant)<1e-12:
                            continue
                        v,w = (yy*xz-xy*yz)/determinant,(xx*yz-xy*xz)/determinant
                        distance = sum((z[i]-v*x[i]-w*y[i])**2 for i in range(3))**.5
                        if distance > .002 or min(1-v-w,v,w) < -1e-5:
                            continue
                        expected = mapped[0]*(1-v-w)+mapped[1]*v+mapped[2]*w
                        candidates.append(((expected-uv).length,distance))
                    if not candidates:
                        raise ValueError('Partition sample lies outside original surface: '+obj.name+' at '+str(list(p))+' nearest '+str(min((closest_point_on_tri(p,*xyz)-p).length for xyz,_,_ in source)))
                    error,distance = min(candidates)
                    if error > 2e-5:
                        raise ValueError(f'Partition changed UV mapping: {obj.name}: {error}')
                    maximum_distance=max(maximum_distance,distance)
                    maximum_uv=max(maximum_uv,error)
                    count+=1
        result={'status':'PASS','samples':count,'max_surface_distance':maximum_distance,
                'max_uv_error':maximum_uv,'components':len(objects)-1}
        result['source_node'] = spec['source_node']
        results.append(result)
    report = {'status':'PASS','partitions':results}
    (OUT/'grouped/partition-verification.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))


if __name__ == '__main__':
    main()
