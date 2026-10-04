"""Bind the isolated native phase candidate to its evidence and ownership limits."""
import hashlib, json, struct
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.ndimage import distance_transform_edt
from catalog import OUT, tree_workspace


def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()
def main():
    root = OUT/'tree41-phase-appearance-proof-v2'
    inputs = json.loads((root/'inputs.json').read_text())
    base = np.array(Image.open(inputs['approved_atlas']))[:,:,3] > 127
    temporal = np.array(Image.open(root/'temporal-domain.png')) > 0
    new = temporal & ~base
    own_distance = distance_transform_edt(~base)
    neighbors = []
    for mask in [31,32,33,34,35,36,37]:
        worker = tree_workspace(mask)
        image = worker/'inspection/irregular-crown-edge/complete-source.png'
        packet = worker/'inspection/irregular-crown-edge/partition.json'
        assert json.loads(packet.read_text())['native_bbox'] == inputs['atlas_bbox']
        alpha = np.array(Image.open(image))[:,:,3] > 127
        distance = distance_transform_edt(~alpha)
        neighbors.append(dict(mask=mask,model=str(worker/'model.blend'),model_sha256=sha(worker/'model.blend'),source_mask=str(image),source_mask_sha256=sha(image),new_pixel_overlap=int((new&alpha).sum()),existing_phase0_overlap=int((base&alpha).sum()),new_pixels_closer_to_neighbor=int((new&(distance<own_distance)).sum()),new_pixels_equidistant=int((new&(distance==own_distance)).sum())))
    assert all(n['new_pixel_overlap']==0 for n in neighbors)
    glb = root/'phase-appearance-tree41.glb'
    raw = glb.read_bytes();length=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+length])
    front_keys=[]
    for node in doc['nodes']:
        if '/ Crown' in node['name']:
            primitive=doc['meshes'][node['mesh']]['primitives'][0]
            front_keys.append(json.dumps({k:v for k,v in primitive.items() if k!='material'},sort_keys=True))
    assert len(set(front_keys)) == 1, 'All phase fronts must share geometry accessors'
    evidence = {}
    for name in ['inputs.json','proof.json','optimization-verification.json','browser-verification.json','actual-phase-comparison.png','browser-phase-0.png','browser-phase-7.png','browser-phase-11.png','temporal-domain.png']:
        evidence[name] = sha(root/name)
    candidate = dict(id='croisement02-tree-41-native-phase-animation',status='pending-coordinator-plan-and-user-state-approval',base_model=dict(path=inputs['model'],sha256=inputs['model_sha256']),candidate_glb=dict(path=str(glb),sha256=sha(glb)),native_timing=doc['extras']['nativeTiming'],phases=inputs['phases'],evidence_sha256=evidence,ownership=dict(new_pixels=int(new.sum()),neighbors=neighbors,reservation='Reserve these currently unoccupied native source coordinates for tree41 only in this isolated candidate. Future same-sequence candidates must subtract this reservation or jointly replace its ownership; no neighbor asset is changed.',native_identity_limit='Native frames describe the combined crown sequence, not per-tree identities. Current approved masks overlap. Nearest-mask distance alone cannot establish physical tree identity.',preservation='All approved phase0 pixels, neighboring masks, stationary wood and full-depth geometry remain unchanged.'),memory=dict(phase_front_geometry_shared=True,unique_front_geometry_bindings=len(set(front_keys)),deduplication=json.loads((root/'optimization-verification.json').read_text())),review_scope='Own native RGB overlays over static painted source, 14 discrete timed phases, source-supported temporal extension on existing geometry. No global editor or engine integration.',approval=dict(geometry_base='preserved existing approval; this document creates no approval',animation='pending',temporal_ownership='requires joint reservation review before neighboring animation integration'))
    (root/'animation-review-candidate.json').write_text(json.dumps(candidate,indent=2)+'\n')
    print(json.dumps(dict(status='READY_FOR_COORDINATOR_REVIEW',new_pixels=int(new.sum()),current_neighbor_overlap=0,unique_front_geometry_bindings=len(set(front_keys))),indent=2))

if __name__ == '__main__': main()
