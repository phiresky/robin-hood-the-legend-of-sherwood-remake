"""Audit full source-art foliage coverage/RGB after replacing canopy topology."""
import hashlib
import json
import math
from pathlib import Path
import sys
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release


def main():
    e=ROOT/'level-editor/work/croisement03-refinement/restart2/texture-batch-v7/croisement03-tree-25/experiment'
    out=e/'cluster-geometry-v3'; report=json.loads((out/'construction.json').read_text())
    assert not (out/'native-appearance.json').exists()
    reference=np.load(out/'native-samples.npz'); before=reference['rgba']; observed_before=reference['observed']
    x0,y0,x1,y1=report['native_bbox']; height,width,_=before.shape
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(out/'worker.blend'))
        obj=next(o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement03-tree-25')
        mesh=obj.data; mesh.calc_loop_triangles(); vertices=[obj.matrix_world@v.co for v in mesh.vertices]; tris=list(mesh.loop_triangles)
        bvh=BVHTree.FromPolygons(vertices,[list(t.vertices) for t in tris],all_triangles=True)
        atlases={}
        for slot,material in enumerate(mesh.materials):
            if not material or not material.get('foliage_physical_opacity'): continue
            node=next(n for n in material.node_tree.nodes if n.type=='TEX_IMAGE' and n.image)
            values=np.empty(len(node.image.pixels),np.float32); node.image.pixels.foreach_get(values)
            atlases[slot]=(values.reshape(node.image.size[1],node.image.size[0],4),mesh.uv_layers[node.inputs['Vector'].links[0].from_node.uv_map],material.get('foliage_card_sides')=='paired-one-sided')
        sin,cos=math.sin(math.radians(35)),math.cos(math.radians(35)); ray=Vector((0,-cos,sin))
        after=np.zeros_like(before); observed_after=np.zeros_like(observed_before); limits=0
        ownership=mesh.color_attributes['Source ownership']
        for yy in range(height):
            for xx in range(width):
                origin=Vector((x0+xx+.5,-(y0+yy+.5)/sin,0))+ray*10000
                for step in range(256):
                    p,n,tid,d=bvh.ray_cast(origin,-ray)
                    if p is None: break
                    tri=tris[tid]; face=mesh.polygons[tri.polygon_index]; slot=face.material_index
                    if slot not in atlases: break
                    rgba,uv,sided=atlases[slot]
                    if not (sided and n.dot(-ray)>=0):
                        mapped=barycentric_transform(p,*[vertices[i] for i in tri.vertices],*[Vector((*uv.data[i].uv,0)) for i in tri.loops])
                        tx=min(rgba.shape[1]-1,int((mapped.x%1)*rgba.shape[1])); ty=min(rgba.shape[0]-1,int((mapped.y%1)*rgba.shape[0]))
                        if rgba[ty,tx,3]>=.5:
                            after[yy,xx]=rgba[ty,tx]; after[yy,xx,3]=1
                            observed_after[yy,xx]=min(ownership.data[i].color[0] for i in tri.loops)>.5
                            break
                    origin=p-ray*.002
                else: limits+=1
            if yy%75==0: print('Appearance audit row',yy,flush=True)
        old_mask=before[...,3]>.5; new_mask=after[...,3]>.5
        missing=old_mask&~new_mask; extra=new_mask&~old_mask; common=old_mask&new_mask
        rgb=np.abs(before[...,:3]-after[...,:3]); rgb_changed=common&np.any(rgb>1/255+1e-6,axis=2)
        ownership_changed=common&(observed_before!=observed_after)
        def picture(rgba):
            rgb=np.full((height,width,3),32,np.uint8); mask=rgba[...,3]>.5
            rgb[mask]=np.rint(np.clip(rgba[mask,:3],0,1)*255).astype(np.uint8)
            return Image.fromarray(rgb)
        diff=np.full((height,width,3),32,np.uint8); diff[missing]=(255,60,60); diff[extra]=(60,220,255); diff[rgb_changed]=(255,220,60)
        sheet=Image.new('RGB',(width*3,height)); sheet.paste(picture(before),(0,0)); sheet.paste(picture(after),(width,0)); sheet.paste(Image.fromarray(diff),(width*2,0)); sheet.save(out/'full-native-appearance.png')
        data=dict(status='PASS source-art pixel-centre coverage and RGB' if not (missing.any() or extra.any() or rgb_changed.any() or ownership_changed.any() or limits) else 'HOLD differences require classification',
            model_sha256=report['model_sha256'],baseline_sha256=report['source_model_sha256'],native_bbox=report['native_bbox'],sampled_source_pixels=width*height,
            baseline_foliage_pixels=int(old_mask.sum()),candidate_foliage_pixels=int(new_mask.sum()),missing_pixels=int(missing.sum()),extra_pixels=int(extra.sum()),
            rgb_changes_over_one_8bit_step=int(rgb_changed.sum()),maximum_common_rgb_delta=float(rgb[common].max()),observed_ownership_changes=int(ownership_changed.sum()),ray_depth_limits=limits,
            ownership_difference_source_pixels=[dict(pixel=[int(x+x0),int(y+y0)],before=bool(observed_before[y,x]),after=bool(observed_after[y,x])) for y,x in zip(*np.where(ownership_changed))],
            difference_source_pixels={'missing':[[int(x+x0),int(y+y0)] for y,x in zip(*np.where(missing))],'extra':[[int(x+x0),int(y+y0)] for y,x in zip(*np.where(extra))]},
            appearance_sheet_sha256=hashlib.sha256((out/'full-native-appearance.png').read_bytes()).hexdigest(),
            limitations=['One pixel-centre ray per original source-art pixel with nearest alpha and authored sidedness; saved render antialiasing differences are separate.',
                        'Pixel ownership means observed versus inferred foliage and preservation of wood holes; old face IDs deliberately replaced.'])
        (out/'native-appearance.json').write_text(json.dumps(data,indent=2)+'\n'); print(data)
    finally: release()


if __name__=='__main__': main()
