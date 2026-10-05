"""Compare opaque shrub samples against frozen neighboring bank and rock solids."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release

def samples(obj):
    mesh=obj.data;mesh.calc_loop_triangles();uv=mesh.uv_layers['Foliage UV'];result=[];images={}
    world=np.array([tuple(obj.matrix_world@v.co) for v in mesh.vertices])
    for tri in mesh.loop_triangles:
        m=mesh.materials[tri.material_index];im=next(n.image for n in m.node_tree.nodes if n.type=='TEX_IMAGE' and n.image)
        w,h=im.size
        if im.name not in images:images[im.name]=np.array(im.pixels[:],dtype=np.float32).reshape(h,w,4)[:,:,3]
        alpha=images[im.name];coords=np.array([uv.data[i].uv for i in tri.loops])*[w,h]
        x0,y0=np.maximum(0,np.floor(coords.min(0)).astype(int));x1,y1=np.minimum([w,h],np.ceil(coords.max(0)).astype(int))
        if x1<=x0 or y1<=y0:continue
        yy,xx=np.mgrid[y0:y1:2,x0:x1:2];q=np.column_stack((xx.ravel()+.5,yy.ravel()+.5));a,b,c=coords;basis=np.column_stack((b-a,c-a))
        if abs(np.linalg.det(basis))<1e-9:continue
        bc=(q-a)@np.linalg.inv(basis).T;inside=(bc[:,0]>=0)&(bc[:,1]>=0)&(bc.sum(1)<=1)&(alpha[yy.ravel(),xx.ravel()]>.5)
        bary=np.column_stack((1-bc[inside].sum(1),bc[inside]));result.extend(bary@world[list(tri.vertices)])
    return np.array(result)

def main(version=1):
    dest=OUT/f'restart2-fence/shrub57-sign-bend-v{version}/ground-parity-proof';dest.mkdir(exist_ok=False)
    source=OUT/'understory-round-9/assets/croisement02-shrub-57/model.blend';candidate=dest.parent/'model.blend'
    positions=[]
    for path in [source,candidate]:
        bpy.ops.wm.open_mainfile(filepath=str(path));positions.append(samples(bpy.data.objects['West Rock Foliage 57']))
    assert positions[0].shape==positions[1].shape
    manifest=OUT/'restart2-fence/sign-neighbors-v4/manifest.json';inputs=json.loads(manifest.read_text())['inputs'];results=[]
    for key in ['north-woodland-bank','west-rock-outcrop','southwest-rock-outcrop']:
        row=inputs[key];path=Path(row['worker'])/'model.blend';assert sha(path)==row['model_sha256']
        bpy.ops.wm.open_mainfile(filepath=str(path));verts=[];tris=[]
        for name in row['objects']:
            obj=bpy.data.objects[name];mesh=obj.data;mesh.calc_loop_triangles();offset=len(verts)
            verts.extend([tuple(obj.matrix_world@v.co) for v in mesh.vertices]);tris.extend([tuple(offset+i for i in t.vertices) for t in mesh.loop_triangles])
        tree=BVHTree.FromPolygons(verts,tris,all_triangles=True);values=[]
        for points in positions:
            signed=[]
            for point in points:
                location,normal,index,distance=tree.find_nearest(Vector(point))
                assert location is not None
                signed.append(distance if (Vector(point)-location).dot(normal)>=0 else -distance)
            values.append(np.array(signed))
        old,new=values
        bounds=np.array(verts);lo,hi=bounds.min(0),bounds.max(0)
        direction=Vector((.723,.417,.552)).normalized()
        inside_arrays=[]
        for points in positions:
            flags=[]
            for point in points:
                if np.any(point<lo) or np.any(point>hi):flags.append(False);continue
                origin=Vector(point);count=0
                for _ in range(256):
                    hit,normal,index,distance=tree.ray_cast(origin,direction,10000)
                    if hit is None:break
                    count+=1;origin=hit+direction*.0005
                else:raise ValueError('Bank parity exceeded256 crossings')
                flags.append(count%2==1)
            inside_arrays.append(np.array(flags))
        oldinside,newinside=inside_arrays
        results.append(dict(old_inside_by_parity=int(oldinside.sum()),new_inside_by_parity=int(newinside.sum()),newly_inside_by_parity=int((newinside&~oldinside).sum()),asset=key,model_sha256=row['model_sha256'],samples=len(old),old_negative_beyond_halfunit=int(np.count_nonzero(old<-.5)),new_negative_beyond_halfunit=int(np.count_nonzero(new<-.5)),newly_negative_beyond_halfunit=int(np.count_nonzero((new<-.5)&(old>=-.5))),old_minimum_signed_distance=float(old.min()),new_minimum_signed_distance=float(new.min())))
    write_json(dest/'report.json',dict(status='Oriented surface and independent odd-even ray parity comparison',model_sha256=sha(candidate),source_model_sha256=sha(source),neighbor_manifest_sha256=sha(manifest),results=results,limitations=['Odd-even parity assumes closed consistently overlapping bank parts; it is a sampled point-in-volume audit, not an exhaustive triangle intersection certificate.','Opaque samples use existing two-texel spacing; all paired sides included.','Negative values may include preexisting intentional foliage/rock overlap; the newly-negative count isolates changed contact.']))
    print(dest)
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
