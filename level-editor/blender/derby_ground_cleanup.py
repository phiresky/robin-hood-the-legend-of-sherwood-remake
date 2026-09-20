"""Remove two audited, now-modeled well details from Derby's ground atlas.

This intentionally does not regenerate the complete terrain texture. Source
ownership is restricted to traced lifting-frame strokes and the reviewed bucket
footprint. Clean nearby ground supplies deterministic donor pixels; shadows and
unrelated ground props outside those masks are unchanged.
"""
import hashlib
import json
from pathlib import Path

import bpy
import numpy as np

TAG='derby-well-ground-ownership-v1'
FRAME_PATHS=[[(800,1734),(801,1716),(798,1710),(802,1708),(805,1695),(811,1687)],
             [(823,1734),(823,1701),(820,1703),(817,1692),(811,1687)],
             [(798,1710),(793,1704)],[(823,1701),(828,1692)],
             [(811,1691),(811,1697)],[(811,1697),(812,1731)],
             [(810,1728),(815,1724),(822,1723)],
             [(792,1699),(795,1704)],[(829,1688),(834,1692)],
             [(823,1704),(828,1700)]]


def ownership_masks(width, height):
    """Return bounded alpha masks and audited donor offsets in source pixels."""
    if (width,height)!=(1920,2752):
        raise ValueError('Derby cleanup traces require a 1920 x 2752 atlas')
    result=[]
    x0,y0,x1,y1=787,1681,839,1740
    yy,xx=np.mgrid[y0:y1,x0:x1].astype(np.float32)
    xx+=.5;yy+=.5
    mask=np.zeros_like(xx)
    for path_index,path in enumerate(FRAME_PATHS):
        # Include antialiasing and the few-pixel artwork/geometry mismatch;
        # narrower centerline-only masks leave disconnected iron fragments.
        radius=4.5 if path_index<4 or path_index>=7 else (4.2 if path_index==4 else 3.4)
        for (ax,ay),(bx,by) in zip(path,path[1:]):
            dx,dy=bx-ax,by-ay
            along=np.clip(((xx-ax)*dx+(yy-ay)*dy)/(dx*dx+dy*dy),0,1)
            distance=np.hypot(xx-(ax+along*dx),yy-(ay+along*dy))
            mask=np.maximum(mask,np.clip((radius-distance)/.8,0,1))
    # The modeled hanging bucket was painted behind the stone coping.
    bucket=np.sqrt(((xx-807)/6)**2+((yy-1722)/8)**2)
    mask=np.maximum(mask,np.clip((1-bucket)*4,0,1))
    # The donor lies completely outside the frame's full source extent, so
    # cloning cannot transfer a left-hand iron stroke into a right-hand repair.
    result.append(('lower-well-lifting-frame', (x0,y0,x1,y1),mask,(-60,0)))
    x0,y0,x1,y1=1411,1486,1426,1507
    yy,xx=np.mgrid[y0:y1,x0:x1]
    edge=np.minimum.reduce([xx-x0+1,x1-xx,yy-y0+1,y1-yy]).astype(np.float32)
    result.append(('covered-well-water-bucket',(x0,y0,x1,y1),np.minimum(edge/1.5,1),(-24,0)))
    return result


def cleanup(output_dir):
    output=Path(output_dir).resolve()
    working=bpy.data.collections['Derby Working']
    grounds=[o for o in working.all_objects if o.type=='MESH' and o.get('source_node')=='ground' and not o.hide_render]
    if len(grounds)!=1:raise ValueError('Expected one visible Derby terrain mesh')
    ground=grounds[0]
    if ground.get('ground_ownership_cleanup')==TAG:
        return {'status':'existing','report':ground['ground_ownership_report']}
    # Refuse to erase a painted object before its replacement exists.
    if not any(o.get('well_refinement')=='lower-bailey-open-well-v1' and not o.hide_render for o in working.all_objects):
        raise ValueError('Refine the lower well before removing its ground ghost')
    if not any(o.get('source_node')=='building-111' and 'Water bucket' in o.name and not o.hide_render for o in working.all_objects):
        raise ValueError('Refine the covered well bucket before removing its ground ghost')
    if len(ground.data.materials)!=1:raise ValueError('Expected one terrain material')
    original_material=ground.data.materials[0]
    nodes=[n for n in original_material.node_tree.nodes if n.type=='TEX_IMAGE' and n.image]
    if len(nodes)!=1:raise ValueError('Expected one terrain source atlas')
    original=nodes[0].image
    width,height=original.size
    flat=np.empty(width*height*4,dtype=np.float32);original.pixels.foreach_get(flat)
    before=flat.reshape(height,width,4)[::-1].copy();after=before.copy()
    mask_union=np.zeros((height,width),dtype=bool);mask_preview=np.zeros((height,width),dtype=np.float32)
    regions=[]
    for label,(x0,y0,x1,y1),alpha,(dx,dy) in ownership_masks(width,height):
        donor=before[y0+dy:y1+dy,x0+dx:x1+dx]
        if donor.shape!=after[y0:y1,x0:x1].shape:raise ValueError('Donor lies outside source image')
        target=after[y0:y1,x0:x1];active=alpha>0
        blended=target[:,:,:3]*(1-alpha[:,:,None])+donor[:,:,:3]*alpha[:,:,None]
        target[:,:,:3][active]=blended[active]
        mask_union[y0:y1,x0:x1]|=active
        mask_preview[y0:y1,x0:x1]=np.maximum(mask_preview[y0:y1,x0:x1],alpha)
        regions.append({'owner':label,'bounds':[x0,y0,x1,y1],'donor_offset':[dx,dy],'mask_pixels':int(active.sum())})
    outside_equal=np.array_equal(before[~mask_union],after[~mask_union])
    if not outside_equal:raise AssertionError('Cleanup changed pixels outside ownership masks')
    if output.exists():raise FileExistsError('Use a fresh ground cleanup output directory')
    output.mkdir(parents=True)
    if original.packed_file:
        backup=bytes(original.packed_file.data)
        (output/'original-atlas.packed').write_bytes(backup)
    else:
        backup=before.tobytes();(output/'original-atlas-float32.rgba').write_bytes(backup)
    image=bpy.data.images.new('Derby ground / audited well cleanup',width=width,height=height,alpha=True)
    image.colorspace_settings.name=original.colorspace_settings.name
    image.pixels.foreach_set(after[::-1].copy().reshape(-1));image.filepath_raw=str(output/'ground-clean.png');image.file_format='PNG';image.save();image.pack()
    # Verify the exported texture too: PNG encoding must not quietly alter
    # unrelated terrain pixels even though the in-memory operation was exact.
    roundtrip=bpy.data.images.load(str(output/'ground-clean.png'),check_existing=False)
    decoded=np.empty_like(flat);roundtrip.pixels.foreach_get(decoded)
    decoded=decoded.reshape(height,width,4)[::-1]
    roundtrip_equal=np.array_equal(before[~mask_union],decoded[~mask_union])
    bpy.data.images.remove(roundtrip)
    if not roundtrip_equal:raise AssertionError('Saved PNG changed pixels outside ownership masks')
    preview=bpy.data.images.new('Derby ground cleanup ownership mask',width=width,height=height,alpha=True)
    rgba=np.ones((height,width,4),dtype=np.float32);rgba[:,:,:3]=mask_preview[:,:,None]
    preview.pixels.foreach_set(rgba[::-1].copy().reshape(-1));preview.filepath_raw=str(output/'ownership-mask.png');preview.file_format='PNG';preview.save()
    material=original_material.copy();material.name='Derby ground / audited well cleanup'
    next(n for n in material.node_tree.nodes if n.type=='TEX_IMAGE' and n.image).image=image
    ground.data.materials[0]=material
    report={'status':'cleaned','recipe':TAG,'original_image':original.name,'original_sha256':hashlib.sha256(backup).hexdigest(),'outside_mask_identical':outside_equal,'png_roundtrip_outside_identical':roundtrip_equal,
            'mask_pixels':int(mask_union.sum()),'changed_pixels':int(np.any(before!=after,axis=2).sum()),'regions':regions,
            'limitations':['Only the two audited well ghosts are repaired','Occluded ground is copied from nearby ground; its exact original content is unavailable','Existing baked ground shadows are retained']}
    (output/'cleanup.json').write_text(json.dumps(report,indent=2)+'\n')
    ground['ground_ownership_cleanup']=TAG;ground['ground_ownership_report']=str(output/'cleanup.json');ground['ground_ownership_original_image']=original.name
    return report
