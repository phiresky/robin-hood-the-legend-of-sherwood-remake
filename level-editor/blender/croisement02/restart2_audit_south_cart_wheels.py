"""Reopen the private wheel debris and audit source intersections and finite solids."""
import json,sys
from pathlib import Path
import bpy,bmesh
import numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
sys.path.insert(0,str(Path(__file__).parent))
from catalog import OUT
from tree_geometry import RAY
from log_trap_state_candidate import point,sha

def main():
    base=OUT/'restart2-state/south-cart-wheel-pair-v2';manifest=json.loads((base/'manifest.json').read_text());model=base/'worker.blend';assert sha(model)==manifest['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(model));objects=[o for o in bpy.context.scene.objects if o.type=='MESH'];vertices=[];faces=[];records=[]
    for obj in objects:
        bm=bmesh.new();bm.from_mesh(obj.data);assert all(e.is_manifold for e in bm.edges);v=bm.calc_volume(signed=True);assert v>0;bm.free();offset=len(vertices);vertices.extend(obj.matrix_world@p.co for p in obj.data.vertices);faces.extend(tuple(offset+i for i in f.vertices)for f in obj.data.polygons);records.append(dict(object=obj.name,closed=True,volume=v,minimum_z=min(p.co.z for p in obj.data.vertices)))
    bvh=BVHTree.FromPolygons(vertices,faces);a=np.array(Image.open(base/'wheel-pair-source.png').convert('RGBA'));frame=manifest['source_frame'];left,top=[manifest['native_position'][i]+frame['offset'][i]for i in range(2)];ys,xs=np.where(a[:,:,3]>0);hit=np.zeros(a.shape[:2],dtype=bool)
    for x,y in zip(xs,ys):hit[y,x]=bvh.ray_cast(point(left+x+.5,top+y+.5,0)+RAY*3000,-RAY,6000)[0]is not None
    diag=a.copy();diag[(a[:,:,3]>0)&~hit]=[255,0,255,255];Image.fromarray(diag).save(base/'source-missing-magenta.png');report=dict(status='PASS reopened closed positive solids; source region coverage diagnostic only',model_sha256=sha(model),source_domain_sha256=sha(base/'wheel-pair-source.png'),objects=records,native_region_pixels=len(xs),geometry_hits=int(hit.sum()),missing=int(len(xs)-hit.sum()),limitations=['The broad source region includes ambiguous lower rim and ground-shadow pixels; missing samples are not automatically missing wheel body.','This audit does not establish current ground receiver contact or temporal identity.'])
    ground=OUT/'restart2-ground38/cumulative848-v1/model.blend';assert sha(ground)=='402c1f72d8b19f9d8754abd84856187d71948a417d815e6879abde573b90f8f3'
    with bpy.data.libraries.load(str(ground),link=False)as(src,dst):dst.objects=list(src.objects)
    for obj in dst.objects:
        if obj is not None and obj.name not in bpy.context.scene.objects:bpy.context.scene.collection.objects.link(obj)
    bpy.context.view_layer.update();gv=[];gf=[]
    for obj in dst.objects:
        if obj is None or obj.type!='MESH':continue
        start=len(gv);gv.extend(obj.matrix_world@v.co for v in obj.data.vertices);gf.extend(tuple(start+i for i in f.vertices)for f in obj.data.polygons)
    gbvh=BVHTree.FromPolygons(gv,gf);contacts=[]
    for obj in objects:
        points=[obj.matrix_world@v.co for v in obj.data.vertices if abs(v.co.z-min(p.co.z for p in obj.data.vertices))<.0001];gaps=[]
        for p in points:
            loc=gbvh.ray_cast(Vector((p.x,p.y,1000)),Vector((0,0,-1)),2000)[0];assert loc is not None;gaps.append(p.z-loc.z)
        assert min(gaps)>=-.001;contacts.append(dict(object=obj.name,samples=len(gaps),minimum_gap=min(gaps),maximum_gap=max(gaps)))
    report['receiver_contact']=dict(ground_sha256=sha(ground),records=contacts,scope='Exact receiver vertex/rim support; hubs rest on wheel tops at Z=4. No dynamic stability claim.')
    (base/'reopened-audit.json').write_text(json.dumps(report,indent=2)+'\n');print(report)
if __name__=='__main__':main()
