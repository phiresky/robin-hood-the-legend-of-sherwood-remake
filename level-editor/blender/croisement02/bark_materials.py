"""Same-tree bark completion, keeping ownership and observed RGB unchanged."""
import json
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from scipy.ndimage import distance_transform_edt
from catalog import OUT
from evidence_io import sha


def fill(workspace,objects,mask):
    rows=json.loads((OUT/'baseline/masks/manifest.json').read_text())['masks']
    row=next(r for r in rows if r['index']==mask)
    x,y=row['box_top_left'];w,h=row['box_size']
    alpha=np.asarray(Image.open(OUT/'baseline/masks'/row['png']).convert('L'))>0
    rgb=np.asarray(Image.open(workspace/'reference/source.png').convert('RGB').crop((x,y,x+w,y+h)))
    distance=distance_transform_edt(alpha)
    # Prefer the broad lower trunk; avoid foliage in the upper wood outline.
    yy,xx=np.indices(alpha.shape)
    score=distance*(.5+yy/max(1,h))
    cy,cx=np.unravel_index(np.argmax(score),score.shape)
    radius=max(1,min(5,int(distance[cy,cx])//2))
    box=(max(0,cx-radius),max(0,cy-radius),min(w,cx+radius+1),min(h,cy+radius+1))
    donor_path=workspace/'inspection/bark-donor.png';donor_path.parent.mkdir(exist_ok=True)
    Image.fromarray(rgb).crop(box).save(donor_path)
    donor=bpy.data.images.load(str(donor_path),check_existing=False)
    donor_rgb=np.asarray(donor.pixels[:],dtype=np.float32).reshape(donor.size[1],donor.size[0],4)[:,:,:3]
    from source_projection_bake import bake
    config=json.loads((workspace/'workspace.json').read_text())
    wood=[o for o in objects if o.get('projection_component')!='crown']
    filled=[0]
    def sampler(obj,normal,positions,accepted,colors):
        unknown=~accepted
        ix=np.floor(positions[:,0]).astype(int)%donor_rgb.shape[1]
        iy=np.floor(positions[:,2]).astype(int)%donor_rgb.shape[0]
        colors[unknown,:3]=donor_rgb[iy[unknown],ix[unknown]]
        filled[0]+=int(unknown.sum())
        return unknown
    report=bake(config['map_name'],config['source_path'],workspace/'inspection/bark-ownership.json',
                receiver_nodes=sorted({o['source_node'] for o in wood}),receiver_object_names=[o.name for o in wood],
                collection_name=config['collection_name'],projection_label='exterior',preserve_authored=False,
                source_mask_manifest=config['source_mask_manifest'],hidden_sampler=sampler,
                provenance_directory=workspace/'inspection/bark-provenance')
    for obj in wood:
        for mat in obj.data.materials:
            if mat and mat.get('source_ownership_bake'):
                mat['source_ownership_alpha']='Opaque coverage; source/inferred ownership is in the explicit provenance NPZ'
                mat['inferred_bark_donor_sha256']=sha(donor_path)
                mat['inferred_bark_method']='Unknown-only same-tree donor, protected source RGB unchanged'
    result=dict(native_mask=mask,donor_source_box=[int(x+box[0]),int(y+box[1]),int(x+box[2]),int(y+box[3])],donor_sha256=sha(donor_path),
                inferred_samples=filled[0],known_rgb_unchanged=True,method='Explicit unknown-only bake sampler; same-tree donor, no AI generation',ownership_report=str(workspace/'inspection/bark-ownership.json'))
    (workspace/'inspection/bark-fill.json').write_text(json.dumps(result,indent=2)+'\n')
    return result
