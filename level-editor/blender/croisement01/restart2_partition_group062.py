"""Privately split approved tree71 from a baseline group without changing retained surfaces."""
import copy,hashlib,json,struct
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
R=ROOT/'level-editor/work/croisement01-refinement/restart2'
LIB=ROOT/'level-editor/library'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main():
    source=LIB/'3d-assets/croisement01/croisement01-group-062'
    descriptor=json.loads((source/'asset.json').read_text());data=(source/'model.glb').read_bytes()
    magic,version,total=struct.unpack_from('<III',data);assert magic==0x46546C67 and version==2 and total==len(data)
    length,kind=struct.unpack_from('<II',data,12);assert kind==0x4E4F534A
    original=json.loads(data[20:20+length]);gltf=copy.deepcopy(original)
    removed={'building-062','building-063'};retained={'building-064','building-065','building-076'}
    assert {p['node'] for p in descriptor['parts']}==removed|retained
    indices={i for i,n in enumerate(gltf['nodes']) if n.get('name') in removed};assert len(indices)==2
    assert not any(gltf.get(k) for k in ['animations','skins'])
    mapping={old:new for new,old in enumerate(i for i in range(len(gltf['nodes'])) if i not in indices)}
    nodes=[]
    for i,node in enumerate(gltf['nodes']):
        if i in indices:continue
        if 'children' in node:node['children']=[mapping[c] for c in node['children'] if c not in indices]
        nodes.append(node)
    gltf['nodes']=nodes
    for scene in gltf['scenes']:scene['nodes']=[mapping[n] for n in scene['nodes'] if n not in indices]
    assert {k:v for k,v in gltf.items() if k not in ['nodes','scenes']}=={k:v for k,v in original.items() if k not in ['nodes','scenes']}
    for name in retained:assert next(n for n in gltf['nodes'] if n.get('name')==name)==next(n for n in original['nodes'] if n.get('name')==name)
    metadata=descriptor['gameplay'];assert set(metadata)=={'version','collision','surfaces','projectionReceivers','sightOrder','movementBlockers','doors','lifts','interiors','draft'}
    assert metadata['collision']=='parts'
    buckets={name:copy.deepcopy(metadata) for name in ['retained','tree71']}
    domains={'retained':retained,'tree71':removed}
    for name,domain in domains.items():
        group=buckets[name]
        for key in ['surfaces','projectionReceivers','movementBlockers','doors','lifts','interiors']:
            assert all(row.get('node') in removed|retained for row in metadata[key])
            group[key]=[copy.deepcopy(row) for row in metadata[key] if row['node'] in domain]
        group['sightOrder']={k:v for k,v in metadata['sightOrder'].items() if k in domain}
    for key in ['surfaces','projectionReceivers','movementBlockers','doors','lifts','interiors']:
        all_rows=[row for group in buckets.values() for row in group[key]]
        assert sorted(all_rows,key=lambda row:row['id'])==sorted(metadata[key],key=lambda row:row['id'])
    out=R/'group062-partition-v1';out.mkdir(exist_ok=False);asset=out/'croisement01-group-062';asset.mkdir()
    residual=copy.deepcopy(descriptor);residual['parts']=[p for p in descriptor['parts'] if p['node'] in retained];residual['gameplay']=buckets['retained']
    encoded=json.dumps(gltf,separators=(',',':')).encode();encoded+=b' '*((-len(encoded))%4);tail=data[20+length:]
    result=struct.pack('<III',magic,version,20+len(encoded)+len(tail))+struct.pack('<II',len(encoded),kind)+encoded+tail
    (asset/'model.glb').write_bytes(result);write(asset/'asset.json',residual)
    assert (asset/'model.glb').read_bytes()[20+len(encoded):]==tail
    for name in ['tree71']:
        write(out/(name+'-metadata.json'),dict(source_asset=descriptor['id'],source_origin_scene=descriptor['source_origin_scene'],parts=[p for p in descriptor['parts'] if p['node'] in domains[name]],gameplay=buckets[name]))
    resources={r['path']:r['sha256'] for r in descriptor['resources']}
    for path,digest in resources.items():assert sha(LIB/path)==digest
    write(out/'proof.json',dict(status='private partition only; exact tree71 export and combined editor verification required before publication',source_descriptor_sha256=sha(source/'asset.json'),source_model_sha256=sha(source/'model.glb'),retained_model_sha256=sha(asset/'model.glb'),retained_descriptor_sha256=sha(asset/'asset.json'),retained_parts=sorted(retained),removed_tree_parts=sorted(removed),retained_mesh_nodes_exact=True,all_mesh_accessors_materials_buffers_images_exact=True,unchanged_trailing_chunks_sha256=hashlib.sha256(tail).hexdigest(),shared_resources=resources,pivot_unchanged=True,gameplay_partition_lossless=True,source_scene_sha256=sha(LIB/'scenes/croisement01.rhlos-map.json'),publication_performed=False))
    print(out)
if __name__=='__main__':main()
