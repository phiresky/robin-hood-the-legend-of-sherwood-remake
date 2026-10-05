"""Bind accepted fence95 boundary roles as an explicit post-freeze source delta."""
import json,sys
from pathlib import Path
import numpy as np
from PIL import Image
sys.path[:0]=[str(Path(__file__).parent),str(Path(__file__).resolve().parents[3]/'level-editor/refinement/blender')]
from catalog import OUT,reviewed_catalog
from evidence_io import sha,write_json


def main():
    base=OUT/'restart2-vegetation/source-role-delta-v2';out=OUT/'restart2-vegetation/source-role-delta-v3'
    if out.exists():raise FileExistsError(out)
    roles=OUT/'missing-fence-candidates/boundary-roles95-v1';root=OUT/'missing-fence-candidates/post-cap95-v1/root-review.json';coverage=roles/'receiver-coverage/scoped-report-v2.json'
    accepted=json.loads(root.read_text());assert accepted['source_roles']==dict(cap95=7,existing95=1,foliage75=28,ground=15)
    assert accepted['model_sha256']=='dde0fc7944ea5bb4ce3613edc401f506c3ee189210f13427207adaf5c81cb414'
    catalog=json.loads((base/'catalog.json').read_text());assert catalog==json.loads(reviewed_catalog().read_text()) and len(catalog['groups'])==126
    manifest=json.loads((base/'source-masks.json').read_text());inventory=json.loads((base/'mask-inventory.json').read_text());indices={r['index']:r for r in inventory['masks']};assert 6009 not in indices
    def read(path):return np.asarray(Image.open(path).convert('L'))>0
    cap=read(roles/'cap95.png');old=read(roles/'existing95.png');leaf=read(roles/'foliage75.png');ground=read(roles/'ground.png')
    assert [int(a.sum())for a in (cap,old,leaf,ground)]==[7,1,28,15]
    assert not np.any(cap&old);wood=cap|old
    assert not np.any(wood&read(indices[431]['png']))
    assert not np.any(wood&read(indices[487]['png'])) and not np.any(wood&read(indices[6008]['png']))
    assert np.all(read(indices[487]['png'])[leaf]) and np.all(read(indices[6008]['png'])[ground])
    out.mkdir();mask=out/'inferred-wood95-8.png';Image.fromarray(wood.astype('uint8')*255).save(mask)
    inventory['masks'].append(dict(index=6009,layer=0,png=str(mask),box_top_left=[0,0],box_size=[1792,1152],provenance='Separately inferred fence95 wood boundary: seven cap pixels plus one existing wood pixel. Root accepted exact role partition; source coverage remains partially sampled at one cap boundary.'))
    changed=0
    for row in manifest['projections']['exterior']['assignments']:
        if row.get('source_node')=='scenery-upright-fence-095':
            assert row['mask_indices']==[431];row['mask_indices']=[431,6009];changed+=1
        if row.get('source_node')=='ground':row['exclude_mask_indices']=sorted(set(row.get('exclude_mask_indices',[])+[6009]))
    assert changed==1
    manifest['mask_inventory']=str(out/'mask-inventory.json')
    for name,obj in [('catalog.json',catalog),('mask-inventory.json',inventory),('source-masks.json',manifest)]:write_json(out/name,obj)
    paths=[root,coverage,roles/'cap95.png',roles/'existing95.png',roles/'foliage75.png',roles/'ground.png']
    write_json(out/'delta.json',dict(status='Authoritative accepted inferred source-role delta; not a coverage or user-approval claim',base_delta=str(base/'delta.json'),base_delta_sha256=sha(base/'delta.json'),source_manifest_sha256=sha(out/'source-masks.json'),unchanged_groups=126,new_domain=6009,pixels=8,cap_pixels=7,existing_wood_pixels=1,already_owned_leaf_pixels=28,already_owned_ground_pixels=15,ground_permission_unchanged=772189,evidence={str(p):sha(p)for p in paths},receiver_model_sha256=accepted['model_sha256'],limitations=['Frozen scene/source bundles remain immutable.','Cap coverage is six full pixels plus one partial alpha32/255, explicitly accepted; existing wood pixel is covered.','Current tree38/shrub75 joint readiness is separate.']))
    print(out)

if __name__=='__main__':main()
