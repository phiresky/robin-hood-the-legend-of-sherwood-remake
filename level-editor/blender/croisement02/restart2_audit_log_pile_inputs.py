"""Verify private endpoint masks protect every source-known and background pixel."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image
from catalog import OUT

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    root=OUT/'restart2-state/private-log-pile-inputs-v1';rows=[]
    for base in sorted(root.iterdir()):
        if not base.is_dir():continue
        proof=json.loads((base/'derivation.json').read_text());prepared=json.loads((base/'private-inputs/private-inputs.json').read_text());frames=json.loads((base/'modified/views.json').read_text());assert sha(Path(proof['source_model']))==proof['source_model_sha256'];assert sha(base/'model.blend')==proof['prepared_model_sha256'];assert prepared['generation_authorized']is False;views=[]
        for v in frames['views']:
            i=v['index'];known=np.array(Image.open(base/f'modified/views/view-{i}-known.png').convert('RGBA'))[:,:,0]>127;solid=np.array(Image.open(base/f'modified/views/view-{i}-solid.png').convert('RGBA'))[:,:,3]>0;mask=np.array(Image.open(base/f'private-inputs/views/view-{i}-mask.png').convert('RGBA'))[:,:,3];editable=mask==0;assert np.array_equal(editable,solid&~known);assert not np.any(editable&known);assert not np.any(editable&~solid);assert not solid[0].any()and not solid[-1].any()and not solid[:,0].any()and not solid[:,-1].any();views.append(dict(index=i,known_pixels=int(known.sum()),editable_pixels=int(editable.sum()),source_and_background_protected=True,complete_framing=True))
        assert sum(v['editable_pixels']for v in views)==prepared['editable_pixels'];record=dict(asset_id=base.name,source_model_sha256=proof['source_model_sha256'],prepared_model_sha256=proof['prepared_model_sha256'],derivation_sha256=sha(base/'derivation.json'),inputs_sha256=sha(base/'private-inputs/private-inputs.json'),canvas=[1536,768],views=views,status='PASS exact private mask and unchanged geometry guards; exact user approval recorded separately; official bridge required before synthesis',generation_authorized=False);(base/'private-input-audit.json').write_text(json.dumps(record,indent=2)+'\n');rows.append(record)
    assert len(rows)==2
    (root/'manifest.json').write_text(json.dumps(dict(status='Two corrected log endpoint packets prepared; official user approval bridge remains separate',packets=rows),indent=2)+'\n');print([(r['asset_id'],sum(v['editable_pixels']for v in r['views']))for r in rows])
if __name__=='__main__':main()
