"""Deterministic source-ray RGBA and lower-fringe checks for private shrub bends."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import SIN,COS,RAY
from render_slots import acquire,release
from restart2_sign_fragment_bounds import Surface
from opacity_bounds import measure

def sample(path,box):
    bpy.ops.wm.open_mainfile(filepath=str(path));obj=bpy.data.objects['West Rock Foliage 57']
    surface=Surface([('shrub57',obj)]);rgba={}
    for m in obj.data.materials:
        im=next(n.image for n in m.node_tree.nodes if n.type=='TEX_IMAGE' and n.image)
        w,h=im.size;rgba[im.name]=np.array(im.pixels[:],dtype=np.float32).reshape(h,w,4)
    out=np.zeros((box[3]-box[1],box[2]-box[0],4),dtype=np.float32);roles=np.zeros(out.shape[:2],dtype=np.uint8)
    for y in range(box[1],box[3]):
        for x in range(box[0],box[2]):
            origin=Vector((x+.5,-(y+.5)/SIN,0))+RAY*5000
            hit=next(surface.intersections(origin),None)
            if hit is None:continue
            point,_,rec=hit;a,b,c=rec['points'];bc=np.linalg.lstsq(np.column_stack((b-a,c-a)),np.array(point)-a,rcond=None)[0]
            uv=np.array([1-bc.sum(),*bc])@rec['uv'];im=rgba[rec['image']];h,w=im.shape[:2];ix,iy=np.floor(uv*[w,h]).astype(int)
            out[y-box[1],x-box[0]]=im[iy%h,ix%w];roles[y-box[1],x-box[0]]=1 if rec['role']=='observed' else 2
    return out,roles,measure(obj)

def main(versions=(1,3), output_name="shrub57-bend-source-audit-v1"):
    dest=OUT/'restart2-fence'/output_name;dest.mkdir(exist_ok=False)
    source=OUT/'understory-round-9/assets/croisement02-shrub-57/model.blend';box=[-42,200,116,390]
    before,roles,oldbounds=sample(source,box)
    results=[]
    for version in versions:
        candidate=OUT/f'restart2-fence/shrub57-sign-bend-v{version}/model.blend'
        after,newroles,bounds=sample(candidate,box)
        visible=(before[:,:,3]>.5)|(after[:,:,3]>.5);different=np.any(abs(before-after)>1e-5,axis=2)&visible
        observed=roles==1
        row=dict(version=version,model_sha256=sha(candidate),source_model_sha256=sha(source),source_crop=box,visible_union=int(visible.sum()),alpha_coverage_changed=int(np.count_nonzero((before[:,:,3]>.5)!=(after[:,:,3]>.5))),first_hit_rgba_changed=int(different.sum()),observed_first_hit_rgba_changed=int((different&observed).sum()),old_opacity_bounds=oldbounds,new_opacity_bounds=bounds)
        results.append(row)
        image=np.zeros((*visible.shape,3),dtype=np.uint8);image[visible]=80;image[different]=[255,30,100];Image.fromarray(image).resize((632,760),Image.Resampling.NEAREST).save(dest/f'v{version}-difference.png')
    write_json(dest/'report.json',dict(status='Deterministic alpha-aware center-ray first-hit comparison',results=results,limits=['Ignores material lighting; compares packed native/donor RGBA sampled with saved UV.','Floating-point center rays can differ on exact texel boundaries; mismatches remain reported.','Ground measure is alpha-sampled foliage minimum, not bank collision proof.']))
    print(dest)
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
