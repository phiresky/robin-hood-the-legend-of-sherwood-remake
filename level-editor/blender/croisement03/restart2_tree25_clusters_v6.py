"""Add small crossing leaf planes to give the private clusters side volume."""
import hashlib
import json
import math
import random
from pathlib import Path
import shutil
import sys
import bpy
import bmesh
from mathutils import Vector, Quaternion

ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from refinement_workspace import _geometry
from workspace_components import appearance_state


def sha(p): return hashlib.sha256(p.read_bytes()).hexdigest()


def main():
    assert shutil.disk_usage(ROOT).free>25*1024**3
    e=ROOT/'level-editor/work/croisement03-refinement/restart2/texture-batch-v7/croisement03-tree-25/experiment'
    source=e/'cluster-geometry-v4'; out=e/'cluster-geometry-v6'; assert not out.exists()
    report=json.loads((source/'construction.json').read_text()); assert sha(source/'worker.blend')==report['model_sha256']
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(source/'worker.blend')); bpy.context.preferences.filepaths.save_version=0
        obj=next(o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement03-tree-25')
        mesh=obj.data; materials=appearance_state(obj)['materials']
        outside={o.name:_geometry(o,protect_appearance=True) for o in bpy.data.objects if o.type=='MESH' and o!=obj}
        slot=next(i for i,m in enumerate(mesh.materials) if m.name=='Tree25 small clusters / inferred volume')
        ray=Vector((0,-math.cos(math.radians(35)),math.sin(math.radians(35))))
        up=Vector((0,-math.sin(math.radians(35)),-math.cos(math.radians(35))))
        bm=bmesh.new(); bm.from_mesh(mesh); uv=bm.loops.layers.uv['Foliage UV']; all_uv=list(bm.loops.layers.uv.values())
        flags=bm.loops.layers.float_color['Source ownership']; fallback=bm.faces.layers.int['reprojection_fallback_material']
        originals=list(bm.faces)
        def signature(face):
            return (face.material_index,tuple((tuple(l.vert.co),tuple(tuple(l[u].uv) for u in all_uv),tuple(l[flags])) for l in face.loops))
        before=[signature(f) for f in originals]
        candidates=[f for f in originals if f.material_index==slot and abs((obj.matrix_world.to_3x3()@f.normal).dot(ray))>0.1]; assert len(candidates)%2==0
        count=0; inverse=obj.matrix_world.inverted(); x0,y0,x1,y1=report['native_bbox']
        for first,second in zip(candidates[::2],candidates[1::2]):
            verts=set(first.verts)|set(second.verts); assert len(verts)==4
            center=sum((obj.matrix_world@v.co for v in verts),Vector())/4
            coords=[l[uv].uv.copy() for face in (first,second) for l in face.loops]
            u0,u1=min(t.x for t in coords),max(t.x for t in coords)
            v0,v1=min(t.y for t in coords),max(t.y for t in coords)
            width=(u1-u0)*(x1-x0); height=(v1-v0)*(y1-y0)
            assert 0<width<8.001 and 0<height<8.001
            # Irregular small inferred clusters continue foliage into the rear
            # volume. Their planes remain edge-on to the native source camera.
            rng=random.Random(71831+count)
            for k in range(4):
                rotation=Quaternion(ray,rng.uniform(-math.pi,math.pi))
                axis=rotation@Vector((1,0,0))
                c=center+Vector((rng.uniform(-7,7),0,0))+up*rng.uniform(-7,7)-ray*rng.uniform(4,55)
                if not (1092<c.x<1460 and -1535<c.y<-1144 and 75<c.z<282): continue
                scale=rng.uniform(.75,1.15)
                ps=[c+axis*s*width*scale/2+ray*t*height*scale/2 for s,t in ((-1,-1),(1,-1),(1,1),(-1,1))]
                vs=[bm.verts.new(inverse@p) for p in ps]; tex=[(u0,v0),(u1,v0),(u1,v1),(u0,v1)]
                for indices in ((0,1,2),(0,2,3)):
                    face=bm.faces.new([vs[i] for i in indices]); face.material_index=slot; face[fallback]=slot
                    for loop,index in zip(face.loops,indices):
                        for layer in all_uv: loop[layer].uv=tex[index]
                        loop[flags]=(0,1,1,1)
                count+=1
        assert [signature(f) for f in originals]==before
        bm.normal_update(); bm.to_mesh(mesh); bm.free(); mesh.update()
        assert appearance_state(obj)['materials']==materials
        assert outside=={o.name:_geometry(o,protect_appearance=True) for o in bpy.data.objects if o.name in outside}
        ps=[obj.matrix_world@v.co for v in mesh.vertices]
        bounds=[[min(p[i] for p in ps),max(p[i] for p in ps)] for i in range(3)]
        assert bounds[1][1]-bounds[1][0]>=bounds[0][1]-bounds[0][0]
        out.mkdir(); bpy.ops.wm.save_as_mainfile(filepath=str(out/'worker.blend'),compress=True)
        shutil.copyfile(source/'native-samples.npz',out/'native-samples.npz')
        report.update(model_sha256=sha(out/'worker.blend'),cluster_base_model_sha256=sha(source/'worker.blend'),
            added_crossing_planes=count,added_crossing_faces=count*2,all_existing_faces_unchanged=True,after_bounds=bounds)
        (out/'construction.json').write_text(json.dumps(report,indent=2)+'\n')
        print(dict(model_sha256=report['model_sha256'],added_crossing_planes=count,bounds=bounds))
    finally: release()


if __name__=='__main__': main()
