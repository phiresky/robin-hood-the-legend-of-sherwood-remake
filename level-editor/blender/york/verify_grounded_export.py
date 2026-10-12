"""Verify grounded asset anchors, world bounds, shared resources and gallery links."""
import hashlib
import json
import struct
from html.parser import HTMLParser
from pathlib import Path

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/active/york'


def main():
    library=OUT/'stage/map-assets';root=library/'3d-assets'
    index=json.loads((root/'index.json').read_text())['assets']
    geometry=json.loads((OUT/'review/geometry.json').read_text())
    grounding=json.loads((OUT/'grounding/report.json').read_text())
    manifest=json.loads((OUT/'review/manifest.json').read_text())
    catalog=json.loads((ROOT/'level-editor/refinement/catalogs/york.json').read_text())
    assert len(index)==len(geometry)==len(catalog['groups'])+1
    assert {entry['id'] for entry in index}==set(geometry)
    assert manifest['scene']['sha256']==grounding['output_sha256']
    resources=set();maximum=0;near_zero_bases=[]
    for entry in index:
        path=root/entry['descriptor']
        assert hashlib.sha256(path.read_bytes()).hexdigest()==entry['descriptor_sha256']
        descriptor=json.loads(path.read_text());model=root/entry['model'];data=model.read_bytes()
        assert data[:4]==b'glTF'
        length=struct.unpack_from('<I',data,12)[0];gltf=json.loads(data[20:20+length])
        for resource in descriptor.get('resources',[]):
            path=library/resource['path'];assert path.is_file()
            if path not in resources:
                assert hashlib.sha256(path.read_bytes()).hexdigest()==resource['sha256'];resources.add(path)
        for resource in gltf.get('images',[])+gltf.get('buffers',[]):
            uri=resource.get('uri')
            if uri and not uri.startswith('data:'):assert (model.parent/uri).is_file()
        identity=entry['id'];anchor=descriptor.get('source_origin_scene',[0,0,0])
        if identity in grounding['anchors']:assert anchor==grounding['anchors'][identity]
        accessors=[gltf['accessors'][p['attributes']['POSITION']] for m in gltf.get('meshes',[]) for p in m['primitives']]
        points=[p for part in geometry[identity]['parts'] for p in part['positions']]
        for axis in range(3):
            for key,operation in [('min',min),('max',max)]:
                expected=operation(p[axis] for p in points)
                actual=operation(a[key][axis] for a in accessors)+anchor[axis]
                error=abs(actual-expected);maximum=max(maximum,error)
                assert error<.003,(identity,axis,error)
        if identity in grounding['anchors']:
            low=min(a['min'][2] for a in accessors)
            # Exposed global-ground vertices retain their input quantization.
            assert low>=(-.1 if anchor[2]==0 else -.003),(identity,low)
            if low<-.003:near_zero_bases.append({'asset':identity,'z':low})
    class Links(HTMLParser):
        def __init__(self):super().__init__();self.count=0
        def handle_starttag(self,tag,attrs):
            for key,value in attrs:
                if key in ('src','href') and value and not value.startswith('#'):
                    assert (OUT/'review'/value).is_file(),value
                    self.count+=1
    parser=Links();parser.feed((OUT/'review/index.html').read_text())
    assert (OUT/'stage/york.rhlos-map.json').read_bytes()==(library/'scenes/york.rhlos-map.json').read_bytes()
    result={'status':'PASS','assets':len(index),'grounded_asset_origins_verified':len(grounding['anchors']),
        'max_world_bounds_error':maximum,'gallery_links_verified':parser.count,'shared_resource_hashes_verified':len(resources),
        'retained_global_ground_quantization':near_zero_bases}
    (OUT/'grounding/export-verification.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result))


if __name__=='__main__':main()
