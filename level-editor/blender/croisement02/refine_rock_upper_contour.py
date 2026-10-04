"""Constrain exposed upper rock hulls without slicing their hidden lower bodies."""
import json, math, sys
from pathlib import Path
import bpy, bmesh
import numpy as np
from mathutils import Vector
from PIL import Image
ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT / 'level-editor/refinement'), str(Path(__file__).parent)]
from catalog import OUT
from tree_geometry import SIN, COS, RAY
from log_trap_state_candidate import point, sha
from render_slots import acquire, release


def hull(points):
    points = sorted(set(points))
    def cross(o, a, b):
        return (a[0]-o[0])*(b[1]-o[1])-(a[1]-o[1])*(b[0]-o[0])
    low, high = [], []
    for output, seq in [(low, points), (high, points[::-1])]:
        for p in seq:
            while len(output)>1 and cross(output[-2], output[-1], p)<=0: output.pop()
            output.append(p)
    return low[:-1]+high[:-1]


def inside(p, polygon):
    return all((b[0]-a[0])*(p[1]-a[1])-(b[1]-a[1])*(p[0]-a[0]) >= -1e-7
               for a,b in zip(polygon,polygon[1:]+polygon[:1]))


def main():
    base = OUT / 'rock-trap-state-candidate-v8'
    column_mode = '--column-contour' in sys.argv
    dest = OUT / ('rock-trap-state-candidate-v10' if column_mode else 'rock-trap-state-candidate-v9')
    dest.mkdir(exist_ok=False)
    binding = json.loads((base/'manifest.json').read_text())
    assert binding['model_sha256'] == sha(base/'worker.blend')
    source = OUT/'state-target-evidence/rock-trap'
    native = np.array(Image.open(source/'tick--01.png'))[:,:,3]>0
    box = json.loads((source/'manifest.json').read_text())['bbox']
    left,top,right,bottom = box
    surveys = json.loads((source/'covered-ellipsoid-fit.json').read_text())['survey']
    yy,xx = np.where(native)
    distance = np.array([((xx-cx)/rx)**2+((yy-cy)/math.sqrt((rx*SIN)**2+(rz*COS)**2))**2 for cx,cy,rx,rz in surveys])
    owners = distance.argmin(axis=0)
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'))
        scene=bpy.context.scene
        untouched={o.name:[tuple(v.co) for v in o.data.vertices] for o in scene.objects if o.type=='MESH' and o.get('state_endpoint')!='covered'}
        records=[]
        top_columns={int(x):float(yy[xx==x].min()) for x in np.unique(xx)}
        for index,(cx,cy,rx,rz) in enumerate(surveys):
            obj=bpy.data.objects[f'covered inferred complete boulder {index:02d}']
            if column_mode:
                bm=bmesh.new();bm.from_mesh(obj.data);bmesh.ops.subdivide_edges(bm,edges=list(bm.edges),cuts=1,use_grid_fill=True);bm.to_mesh(obj.data);bm.free();obj.data.update()
            points=[]
            for x,y,owner in zip(xx,yy,owners):
                if owner==index and y<=cy:
                    points.extend((float(x)+dx,float(y)+dy)for dx in (-.25,.25)for dy in (-.25,.25))
            # Preserve the equatorial width and the entire hidden lower half.
            points.extend([(cx-rx*1.1,cy),(cx+rx*1.1,cy),(cx,cy+1)])
            polygon=hull(points);changed=[]
            for vertex in obj.data.vertices:
                world=obj.matrix_world@vertex.co
                projected=np.array([world.x-left,-world.y*SIN-world.z*COS-top])
                if projected[1]>=cy:continue
                if column_mode:
                    col=max(min(top_columns),min(max(top_columns),projected[0]-.5));a=int(math.floor(col));b=int(math.ceil(col));a=min(top_columns,key=lambda x:abs(x-a));b=min(top_columns,key=lambda x:abs(x-b));edge=top_columns[a] if a==b else top_columns[a]+(top_columns[b]-top_columns[a])*(col-a)/(b-a)
                    destination=projected.copy();destination[1]=max(destination[1],edge+.15)
                    if abs(destination[1]-projected[1])<1e-8:continue
                elif inside(projected,polygon):continue
                start=np.array([cx,cy]);delta=projected-start;lo,hi=0.,1.
                for _ in range(24):
                    mid=(lo+hi)/2
                    if inside(start+delta*mid,polygon):lo=mid
                    else:hi=mid
                if not column_mode:destination=start+delta*lo
                dx,dy=destination-projected
                assert math.hypot(dx,dy)<10, (index,vertex.index,dx,dy)
                before=vertex.co.copy();vertex.co+=Vector((dx,-dy*SIN,-dy*COS))
                assert abs((vertex.co-before).dot(RAY))<1e-5
                changed.append(dict(vertex=vertex.index,source_before=projected.tolist(),source_after=destination.tolist()))
            obj.data.update()
            for face in obj.data.polygons:
                for loop in face.loop_indices:
                    p=obj.matrix_world@obj.data.vertices[obj.data.loops[loop].vertex_index].co
                    obj.data.uv_layers.active.data[loop].uv=((p.x-left)/(right-left),1-(-p.y*SIN-p.z*COS-top)/(bottom-top))
            records.append(dict(object=obj.name,upper_source_hull=polygon,changed_vertices=changed))
        for name,coords in untouched.items():assert coords==[tuple(v.co)for v in bpy.data.objects[name].data.vertices]
        bpy.ops.wm.save_as_mainfile(filepath=str(dest/'worker.blend'))
        report=dict(binding)
        report.update(status='private upper-contour candidate; contact and joint review pending',model_sha256=sha(dest/'worker.blend'),base_model_sha256=binding['model_sha256'],upper_contour_correction=records)
        (dest/'manifest.json').write_text(json.dumps(report,indent=2)+'\n')
        target=point((left+right)/2,(top+bottom)/2,0)
        scene.camera.location=target+RAY*3000;scene.camera.rotation_euler=(target-scene.camera.location).to_track_quat('-Z','Y').to_euler()
        scene.cycles.samples=16
        scene.render.filepath=str(dest/'covered-source-actual-bank1.png');bpy.ops.render.render(write_still=True)
        print('Upper contour candidate', [(r['object'],len(r['changed_vertices']))for r in records],flush=True)
    finally:release()


if __name__=='__main__':main()
