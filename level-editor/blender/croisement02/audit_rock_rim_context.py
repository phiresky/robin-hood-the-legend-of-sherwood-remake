"""Locate visible unknown rock rims and inspect source owners before reshaping solids."""
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from mathutils.bvhtree import BVHTree
from PIL import Image
sys.path.insert(0,str(Path(__file__).parent))
from catalog import OUT
from tree_geometry import RAY
from log_trap_state_candidate import point,sha
from native_log_foreground_reference import crop_frame


def main():
    base=Path(sys.argv[sys.argv.index('--candidate')+1]).resolve() if '--candidate' in sys.argv else OUT/'rock-trap-state-candidate-v8';joint=Path(sys.argv[sys.argv.index('--joint')+1]).resolve() if '--joint' in sys.argv else OUT/'rock-state-shrub-joint-v2';binding=json.loads((joint/'manifest.json').read_text());assert sha(base/'worker.blend')==binding['rock_model_sha256']
    root=OUT/'state-target-evidence/rock-trap';manifest=json.loads((root/'manifest.json').read_text());box=manifest['bbox'];left,top,right,bottom=box;w=right-left;h=bottom-top;scale=max(w,h)*1.2
    native=np.array(Image.open(root/'tick--01.png'))[:,:,3]>0;pixels=np.array(Image.open(joint/'joint-source-actual.png'));gray=(pixels[:,:,:3].max(axis=2)-pixels[:,:,:3].min(axis=2)<=3)&(pixels[:,:,:3].mean(axis=2)>35)
    yy,xx=np.mgrid[:512,:512];sx=left+w/2+(xx+.5-256)*scale/512;sy=top+h/2+(yy+.5-256)*scale/512;ix=np.floor(sx-left).astype(int);iy=np.floor(sy-top).astype(int);valid=(ix>=0)&(ix<w)&(iy>=0)&(iy<h);known=np.zeros((512,512),bool);known[valid]=native[iy[valid],ix[valid]]
    bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'));bpy.context.view_layer.update();rocks=[o for o in bpy.data.objects if o.get('state_endpoint')=='covered'];surveys=json.loads((base/'manifest.json').read_text())['geometry'];centers={f'covered inferred complete boulder {r["index"]:02d}':r['source_center'][1]for r in surveys if r['state']=='covered'}
    bvhs=[]
    for obj in rocks:bvhs.append((obj,BVHTree.FromPolygons([obj.matrix_world@v.co for v in obj.data.vertices],[tuple(p.vertices)for p in obj.data.polygons])))
    bank_vertices=[];bank_faces=[]
    for obj in bpy.data.objects:
        if obj.type!='MESH' or obj.get('state_endpoint'):continue
        offset=len(bank_vertices);bank_vertices.extend(obj.matrix_world@v.co for v in obj.data.vertices);bank_faces.extend(tuple(offset+i for i in face.vertices)for face in obj.data.polygons)
    bank_bvh=BVHTree.FromPolygons(bank_vertices,bank_faces)
    selected=np.zeros((512,512),bool);domain=np.zeros((h,w),bool);bank_occluded=0
    for y,x in zip(*np.where(gray&~known&valid)):
        ray=point(float(sx[y,x]),float(sy[y,x]),0)+RAY*5000;hits=[]
        for obj,bvh in bvhs:
            if sy[y,x]>centers[obj.name]:continue
            hit=bvh.ray_cast(ray,-RAY)
            if hit[0]is not None:hits.append(hit[3])
        if hits:
            bank_hit=bank_bvh.ray_cast(ray,-RAY)
            if bank_hit[0]is not None and bank_hit[3]<min(hits):bank_occluded+=1;continue
            selected[y,x]=True;domain[iy[y,x],ix[y,x]]=True
    mask_rows=[]
    for mask in json.loads((OUT/'baseline/masks/manifest.json').read_text())['masks']:
        mx,my=mask['box_top_left'];mw,mh=mask['box_size']
        if mx>=right or mx+mw<=left or my>=bottom or my+mh<=top:continue
        path=OUT/'baseline/masks'/mask['png'];crop=np.array(Image.open(path).convert('L').crop((left-mx,top-my,right-mx,bottom-my)))>0;count=int((crop&domain).sum())
        if count:mask_rows.append(dict(global_mask=mask['index'],layer=mask['layer'],layer_local=mask['layer_index'],mask_type=mask['mask_type'],unknown_rim_source_pixels=count,mask_sha256=sha(path)))
    animations=[]
    for animation in json.loads((OUT/'animation-references/manifest.json').read_text())['animations']:
        hits=[]
        for frame in animation['frames']:hits.append(int(((np.array(crop_frame(frame,box))[:,:,3]>0)&domain).sum()))
        if any(hits):animations.append(dict(animation=animation['index'],profile=animation['profile'],polyline=animation['display_polyline'],phase0_pixels=hits[0],minimum_phase_pixels=min(hits),maximum_phase_pixels=max(hits),phase0_sha256=sha(Path(animation['frames'][0]['image'])),all_phase_sha256=[sha(Path(f['image']))for f in animation['frames']]))
    dest=Path(sys.argv[sys.argv.index('--output')+1]).resolve() if '--output' in sys.argv else root/'unknown-rim-context-v2';dest.mkdir(exist_ok=False);Image.fromarray(domain.astype(np.uint8)*255).save(dest/'rim-native-domain.png');display=pixels.copy();display[selected,:3]=(255,30,160);Image.fromarray(display).save(dest/'rim-actual-locations.png')
    baseline=Image.open(OUT/'baseline/covered.png').convert('RGBA').crop(box);baseline.save(dest/'static-baseline.png');composite=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA').crop(box);composite.save(dest/'static-plus-native-phase0.png')
    report=dict(status='context audit only; no further rock shape change justified yet',model_sha256=binding['rock_model_sha256'],joint_manifest_sha256=sha(joint/'manifest.json'),source_manifest_sha256=sha(root/'manifest.json'),bank_first_hit_excluded_render_pixels=bank_occluded,selected_render_pixels=int(selected.sum()),unique_native_pixels=int(domain.sum()),mask_overlaps=mask_rows,animation_rgba_overlaps=animations,method='Near-neutral visible pixels outside native rock RGBA; each must intersect a complete covered rock above its source center before any exact bank surface. Source pixel samples are then intersected with native occupancy and visual-animation RGBA separately.',limitations=['Neutral rendered pixels are a conservative visual diagnostic, not semantic material segmentation.','Mask overlap alone does not prove visible leaf ownership or draw order.','Current joint includes shrub62 only; other exact-context crowns must be considered before shrinking full rocks.'])
    (dest/'manifest.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))


if __name__=='__main__':main()
