"""Verify standalone/map coverage and exported ownership/provenance metadata."""
import json
from pathlib import Path
import struct
import sys


def gltf(path):
    with Path(path).open('rb') as handle:
        magic,version,_=struct.unpack('<III',handle.read(12))
        if magic!=0x46546c67 or version!=2:
            raise ValueError('Expected glTF binary v2')
        length,kind=struct.unpack('<II',handle.read(8))
        if kind!=0x4e4f534a:
            raise ValueError('Expected JSON chunk')
        return json.loads(handle.read(length))


def verify(directory,catalog_path):
    directory=Path(directory)
    catalog=json.loads(Path(catalog_path).read_text())
    stage=json.loads((directory/'stage.json').read_text())
    index=json.loads((directory/'assets/index.json').read_text())
    expected={g['id']:g for g in catalog['groups']}
    if {a['id'] for a in index['assets']}!=set(expected):
        raise ValueError('Standalone group catalog differs from authored ownership')
    nodes=set();components=0;component_metadata=0;masked_generated=0
    for asset in index['assets']:
        descriptor=json.loads((directory/'assets'/asset['descriptor']).read_text())
        owned={f"building-{part['obstacle']:03d}" for part in expected[asset['id']]['parts']}
        actual={c['source_node'] for c in descriptor['components']}
        if owned!=actual or nodes & actual:
            raise ValueError('Standalone canonical ownership differs: '+asset['id'])
        nodes |= actual
        model=gltf(directory/'assets'/asset['model'])
        exported=[n.get('extras',{}) for n in model['nodes']]
        for component in descriptor['components']:
            if component.get('projection_component'):
                component_metadata+=1
                if not any(e.get('projection_component')==component['projection_component'] for e in exported):
                    raise ValueError('Standalone lost component ownership metadata')
        components+=len(descriptor['components'])
    model=gltf(directory/'derby.scene.glb')
    generated={}
    for material in model.get('materials',[]):
        extra=material.get('extras',{})
        if extra.get('generated_source_sha256'):
            generated[extra['generated_source_sha256']]=generated.get(extra['generated_source_sha256'],0)+1
            masked_generated+=bool(extra.get('generated_source_mask_evidence_sha256'))
    for sha,names in stage['generated_materials'].items():
        if generated.get(sha)!=len(names):
            raise ValueError('Map lost selected generated materials: '+sha)
    if len(nodes)!=270:
        raise ValueError('Wrong canonical part count')
    if component_metadata and not any(n.get('extras',{}).get('projection_component') for n in model['nodes']):
        raise ValueError('Map lost component selectors')
    report={'status':'PASS','groups':len(expected),'parts':len(nodes),'components':components,
            'component_metadata':component_metadata,'generated_materials':generated,
            'masked_generated_materials':masked_generated}
    (directory/'asset-verification.json').write_text(json.dumps(report,indent=2)+'\n')
    return report


if __name__=='__main__':
    print(json.dumps(verify(*sys.argv[1:])),flush=True)
