"""Validate the derived wood-only edit subset and immutable context contract."""
import hashlib,json,sys
from pathlib import Path
import numpy as np
from PIL import Image

def validate(experiment):
    e=Path(experiment);old=e/'wood-scope-original'
    read=lambda p:json.loads(p.read_text())
    sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
    report=read(e/'wood-scope.json');prepared=read(e/'preparation.json')
    assert prepared['wood_scope_sha256']==sha(e/'wood-scope.json')
    assert report['status']=='PASS' and report['receivers']==['Unresolved Source Part 030 / Source part 030']
    assert report['foreign_objects']==['Tree00 inferred off-map crown']
    for name,value in report['original_files'].items():assert sha(old/name)==value
    for name,value in report['scoped_files'].items():assert sha(e/name)==value==prepared['files'][name]
    before=read(old/'views.json');after=read(e/'views.json')
    assert after.pop('texture_receiver_object_names')==report['receivers']
    assert before==after
    parent=read(old/'preparation.json')
    assert {k:v for k,v in parent.items() if k!='files'}=={k:v for k,v in prepared.items() if k not in ['files','wood_scope_sha256']}
    assert set(parent['files'])==set(prepared['files'])
    for name,value in parent['files'].items():
        if name not in report['scoped_files']:assert value==prepared['files'][name]==sha(e/name)
    for name in ['mask.png']+[v['mask'] for v in before['views']]:
        a=np.array(Image.open(old/name).convert('RGBA'));b=np.array(Image.open(e/name).convert('RGBA'))
        assert np.array_equal(a[:,:,:3],b[:,:,:3])
        assert np.all((b[:,:,3]==0)<=(a[:,:,3]==0))
        assert set(np.unique(b[:,:,3]))<={0,255}
    assert sha(e/'approved-model.blend')==report['model_sha256']
    assert sha(e/'input.png')==report['input_sha256'] and sha(e/'solid.png')==report['solid_sha256']
    approval=read(e/'approval.json');assert approval['saved_model_sha256']==report['model_sha256']
    provenance=approval['source_decision']['original_gallery_decision']['batch_approval_provenance']
    assert 'WOOD ONLY' in provenance['scope'] and sha(Path(provenance['path']))==provenance['sha256']
    return report

if __name__=='__main__':
    r=validate(sys.argv[1]);print(json.dumps(dict(status='PASS',receivers=r['receivers'],protected=r['foreign_objects'],wood_editable=sum(v['wood_editable'] for v in r['counts']))))
