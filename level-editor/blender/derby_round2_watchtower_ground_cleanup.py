"""Remove the reviewed roof machinery ghost from the terrain material only."""
import hashlib
import io
import json
from pathlib import Path

import bpy
import numpy as np
from PIL import Image

TAG = 'derby-watchtower-ground-native233-v1'


def _sha(data):
    return hashlib.sha256(data).hexdigest()


def cleanup(output_dir, mask_inventory):
    from refinement_workspace import _geometry
    output=Path(output_dir).resolve()
    if output.exists():
        raise FileExistsError(output)
    objects=list(bpy.data.objects)
    before_geometry={o.name:_geometry(o) for o in objects}
    working=bpy.data.collections['Derby Working']
    grounds=[o for o in working.all_objects if o.type=='MESH' and not o.hide_render
             and o.get('source_node')=='ground']
    if len(grounds)!=1:
        raise ValueError('Expected one visible ground')
    ground=grounds[0]
    outside_materials={o.name:[m.name if m else None for m in o.data.materials]
                       for o in objects if o.type=='MESH' and o!=ground}
    uv_before={layer.name:np.array([tuple(item.uv) for item in layer.data],dtype=np.float32).tobytes()
               for layer in ground.data.uv_layers}
    if len(ground.data.materials)!=1:
        raise ValueError('Expected one ground material')
    material=ground.data.materials[0]
    images=[node for node in material.node_tree.nodes if node.type=='TEX_IMAGE' and node.image]
    if len(images)!=1:
        raise ValueError('Expected one ground atlas')
    original=images[0].image
    if not original.packed_file:
        raise ValueError('Ground atlas must be packed for a byte-exact backup')
    packed=bytes(original.packed_file.data)
    before=np.array(Image.open(io.BytesIO(packed)).convert('RGBA'))
    if before.shape!=(2752,1920,4):
        raise ValueError('Expected native Derby atlas dimensions')
    inventory_path=Path(mask_inventory).resolve()
    inventory=json.loads(inventory_path.read_text())
    record=next(r for r in inventory['masks'] if r['index']==233)
    if record['box_top_left']!=[1650,839] or record['box_size']!=[127,138] or record['layer']!=6:
        raise ValueError('Reviewed machinery mask metadata changed')
    mask_path=inventory_path.parent/record['png']
    native=np.array(Image.open(mask_path).convert('L'))>127
    # Two pixels cover the artwork antialias fringe. Restrict all edits to this
    # explicit dilation; harmonic filling never writes surrounding donor pixels.
    pad=4
    core=np.pad(native,pad)
    active=np.zeros_like(core)
    for dy in range(-2,3):
        for dx in range(-2,3):
            if dx*dx+dy*dy<=4:
                active|=np.roll(np.roll(core,dy,0),dx,1)
    x0,y0=record['box_top_left'][0]-pad,record['box_top_left'][1]-pad
    h,w=active.shape
    crop=before[y0:y0+h,x0:x0+w].copy()
    colors=crop[:,:,:3].astype(np.float64)
    colors[active]=np.mean(colors[~active],axis=0)
    # Discrete harmonic continuation uses only the unchanged neighborhood.
    # This is conservative missing-ground interpolation, not recovered artwork.
    residual=0.0
    for iteration in range(4000):
        average=(np.roll(colors,1,0)+np.roll(colors,-1,0)+
                 np.roll(colors,1,1)+np.roll(colors,-1,1))*.25
        residual=float(np.max(np.abs(colors[active]-average[active])))
        colors[active]=average[active]
        if residual<.002:
            break
    after=before.copy()
    target=after[y0:y0+h,x0:x0+w]
    target[active,:3]=np.clip(np.rint(colors[active]),0,255).astype(np.uint8)
    union=np.zeros(before.shape[:2],dtype=bool);union[y0:y0+h,x0:x0+w]=active
    assert np.array_equal(before[~union],after[~union])
    assert np.array_equal(before[:,:,3],after[:,:,3])
    output.mkdir(parents=True)
    (output/'original-atlas.png').write_bytes(packed)
    Image.fromarray(after).save(output/'ground-clean.png')
    Image.fromarray(union.astype(np.uint8)*255).save(output/'ownership-mask.png')
    box=(1630,825,1795,995)
    Image.fromarray(before).crop(box).resize((660,680),Image.Resampling.NEAREST).save(output/'before-closeup.png')
    Image.fromarray(after).crop(box).resize((660,680),Image.Resampling.NEAREST).save(output/'after-closeup.png')
    Image.fromarray(union.astype(np.uint8)*255).crop(box).resize((660,680),Image.Resampling.NEAREST).save(output/'mask-closeup.png')
    decoded=np.array(Image.open(output/'ground-clean.png').convert('RGBA'))
    assert np.array_equal(before[~union],decoded[~union])
    image=bpy.data.images.load(str(output/'ground-clean.png'),check_existing=False)
    image.name='Derby ground / watchtower machinery removed'
    image.colorspace_settings.name=original.colorspace_settings.name
    image.pack()
    replacement=material.copy();replacement.name='Derby ground / wells and watchtower cleanup'
    next(n for n in replacement.node_tree.nodes if n.type=='TEX_IMAGE' and n.image).image=image
    ground.data.materials[0]=replacement
    for o in objects:
        assert _geometry(o)==before_geometry[o.name],o.name
        if o.type=='MESH' and o!=ground:
            assert [m.name if m else None for m in o.data.materials]==outside_materials[o.name]
    assert uv_before=={layer.name:np.array([tuple(item.uv) for item in layer.data],dtype=np.float32).tobytes()
                       for layer in ground.data.uv_layers}
    report={'status':'PASS','recipe':TAG,'mask_index':233,'mask_layer':6,
            'mask_sha256':_sha(mask_path.read_bytes()),'inventory_sha256':_sha(inventory_path.read_bytes()),
            'original_atlas_sha256':_sha(packed),'output_atlas_sha256':_sha((output/'ground-clean.png').read_bytes()),
            'native_mask_pixels':int(native.sum()),'dilated_mask_pixels':int(active.sum()),
            'changed_pixels':int(np.any(before!=after,axis=2).sum()),
            'outside_mask_pixels_identical':True,'png_roundtrip_outside_identical':True,
            'alpha_identical':True,'all_object_geometry_identical':True,'objects_checked':len(objects),
            'terrain_uv_identical':True,'outside_material_assignments_identical':True,
            'iterations':iteration+1,'solver_residual_byte_units':residual,
            'limitations':['Only reviewed mask233 plus its two-pixel antialias fringe is repaired.',
                           'Missing ground is deterministic harmonic interpolation, not recovered original pixels.',
                           'Existing unrelated ground ghosts, building fill and baked shadows remain unchanged.']}
    (output/'cleanup.json').write_text(json.dumps(report,indent=2)+'\n')
    ground['watchtower_ground_cleanup']=TAG
    ground['watchtower_ground_cleanup_report']=str(output/'cleanup.json')
    bpy.ops.wm.save_as_mainfile(filepath=str(output/'worker.blend'))
    return report
