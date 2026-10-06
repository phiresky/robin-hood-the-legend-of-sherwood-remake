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
    asset=read(e/'views.json')['asset_id'];number={'croisement01-tree-00':'030','croisement01-tree-01':'029'}[asset]
    assert report['status']=='PASS' and report['receivers']==[f'Unresolved Source Part {number} / Source part {number}']
    assert report['foreign_objects']==['Tree'+asset[-2:]+' inferred off-map crown']
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
    provenance=approval['source_decision']['original_gallery_decision']
    if asset=='croisement01-tree-00':
        provenance=provenance['batch_approval_provenance'];assert 'WOOD ONLY' in provenance['scope'] and sha(Path(provenance['path']))==provenance['sha256']
    else:
        path=Path(provenance['batch_approval_path']);assert sha(path)==provenance['batch_approval_sha256'];decision=read(path);member=next(m for c in decision['cards'] for m in c['members'] if m['asset_id']==asset);assert decision['status']=='approved' and member['scope']=='geometry' and member['model_sha256']==report['model_sha256'];assert 'woody geometry' in member['scope_description'] and 'crown is excluded' in member['scope_description']
    return report

def bake_packet(experiment, manifest_path):
    """Separate the immutable source-ownership mask from the API edit subset.

    The projector requires the complete original ownership mask. Its receiver
    selection remains wood-only; the generated sheet retains original pixels
    everywhere excluded by the stricter API mask. No reviewed file is changed.
    """
    e=Path(experiment).resolve();report=validate(e)
    manifest_path=Path(manifest_path).resolve()
    manifest=json.loads(manifest_path.read_text())
    assert manifest['texture_receiver_object_names']==report['receivers']
    packet=e/(manifest_path.stem+'-wood-bake-packet')
    packet.mkdir(exist_ok=False)
    for source in e.iterdir():
        if source==packet or source.name in ('mask.png',manifest_path.name):continue
        (packet/source.name).symlink_to(source,target_is_directory=source.is_dir())
    manifest['texture_generated_support_mask'] = {
        'path':str(e/'mask.png'),
        'sha256':hashlib.sha256((e/'mask.png').read_bytes()).hexdigest(),
    }
    (packet/manifest_path.name).write_text(json.dumps(manifest,indent=2)+'\n')
    (packet/'mask.png').symlink_to(e/'wood-scope-original/mask.png')
    record={'scope':'WOOD ONLY','api_scope_sha256':hashlib.sha256((e/'wood-scope.json').read_bytes()).hexdigest(),
            'api_mask_sha256':hashlib.sha256((e/'mask.png').read_bytes()).hexdigest(),
            'projection_ownership_mask_sha256':hashlib.sha256((packet/'mask.png').read_bytes()).hexdigest(),
            'receivers':report['receivers'],'protected_foreign_objects':report['foreign_objects'],
            'projection_manifest_sha256':hashlib.sha256((packet/manifest_path.name).read_bytes()).hexdigest(),
            'reason':'Restore full reviewed ownership only for projection validation; wood receiver selection and exact foreign-appearance guards remain mandatory.'}
    (packet/'adapter.json').write_text(json.dumps(record,indent=2)+'\n')
    return packet/manifest_path.name

if __name__=='__main__':
    r=validate(sys.argv[1]);print(json.dumps(dict(status='PASS',receivers=r['receivers'],protected=r['foreign_objects'],wood_editable=sum(v['wood_editable'] for v in r['counts']))))
