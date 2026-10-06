"""Read-only attribution of near-neutral native-view pixels to baked UV provenance."""
import json,math,sys
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector,geometry
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2'
B=R/'approved-tree00-wood-fill-v1/croisement01-tree-00/baked-v3-luminance'
W=B/'actual-review-v1'
def main():
    acquire();before=sha(B/'worker.blend');bpy.ops.wm.open_mainfile(filepath=str(B/'worker.blend'))
    obj=next(o for o in bpy.data.objects if o.type=='MESH' and o.get('source_node')=='building-030')
    mesh=obj.data;mesh.calc_loop_triangles();triangles=list(mesh.loop_triangles)
    vertices=[obj.matrix_world@v.co for v in mesh.vertices]
    bvh=BVHTree.FromPolygons(vertices,[tuple(t.vertices) for t in triangles],all_triangles=True)
    validation=json.loads((B/'validation.json').read_text());record=validation['layers'][0]['objects'][0]['texel_provenance']
    assert sha(Path(record['path']))==record['sha256'];ownership=np.load(record['path'])['ownership']
    a=np.array(Image.open(W/'inspection/native-source/actual.png').convert('RGBA'))
    select=(np.ptp(a[:,:,:3].astype(int),axis=2)<6)&(a[:,:,3]>250)&(a[:,:,0]>20)
    left,top,right,bottom=json.loads((W/'inspection/actual-materials/evidence.json').read_text())['source_crop']
    sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35));toward=Vector((0,-cosine,sine));direction=-toward
    result=[]
    for row,col in zip(*np.where(select)):
        x=left+col+.5;y=top+row+.5;origin=Vector((x,-y/sine,0))+toward*5000
        hit,normal,idx,distance=bvh.ray_cast(origin,direction,20000)
        entry=dict(x=int(col),y=int(row),render_rgba=a[row,col].tolist())
        if idx is None:entry['status']='center ray outside wood'
        else:
            tri=triangles[idx];mat=mesh.materials[mesh.polygons[tri.polygon_index].material_index]
            texture=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image)
            uvnode=texture.inputs['Vector'].links[0].from_node;layer=mesh.uv_layers[uvnode.uv_map]
            coords=[Vector((*layer.data[i].uv,0)) for i in tri.loops]
            uv=geometry.barycentric_transform(hit,*[vertices[i] for i in tri.vertices],*coords)
            width,height=texture.image.size;px=uv.x*width-.5;py=uv.y*height-.5;ix,iy=math.floor(px),math.floor(py)
            entry.update(face=tri.polygon_index,uv=[uv.x,uv.y],material=mat.name,samples=[])
            assert ownership.shape==(height,width)
            for oy in [0,1]:
                for ox in [0,1]:
                    xx=max(0,min(width-1,ix+ox));yy=max(0,min(height-1,iy+oy))
                    entry['samples'].append(dict(atlas=[xx,yy],ownership=int(ownership[yy,xx]),weight=(1-abs(px-(ix+ox)))*(1-abs(py-(iy+oy))),rgba_linear=list(texture.image.pixels[(yy*width+xx)*4:(yy*width+xx)*4+4])))
        result.append(entry)
    assert sha(B/'worker.blend')==before
    out=R/'tree00-neutral-probe-v1.json';assert not out.exists();out.write_text(json.dumps(dict(model_sha256=before,scope='Read-only native center-ray lookup; no model, material or image mutation',ownership_semantics=record['semantics'],pixels=result),indent=2)+'\n');print(out)
if __name__=='__main__':main()
