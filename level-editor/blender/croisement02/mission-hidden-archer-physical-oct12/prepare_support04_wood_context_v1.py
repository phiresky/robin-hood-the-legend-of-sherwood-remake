"""Create lightweight source-buffer views of the two measured support receivers."""
import json,os,struct,sys
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
import qualify as q

def main():
    dest=q.OUT/'support04-wood-context-v1';assert not dest.exists();dest.mkdir()
    qualification=json.loads((q.OUT/'qualification.json').read_text());records=[]
    for candidate in qualification['rows'][3]['bounds_candidates']:
        if candidate['id'] not in {'croisement02-tree-24','croisement02-tree-25'}:continue
        path=q.LIB/candidate['model'];assert q.pin(path)['sha256']==candidate['model_sha256']
        with path.open('rb')as f:
            header=f.read(20);assert header[:4]==b'glTF';size=struct.unpack_from('<I',header,12)[0];doc=json.loads(f.read(size));offset=28+size
        kept=[]
        for ni,node in enumerate(doc['nodes']):
            if 'mesh'not in node:continue
            if ' wood 'in node.get('name',''):kept.append(dict(index=ni,name=node['name']))
            else:del node['mesh']
        assert kept
        for bi,buffer in enumerate(doc['buffers']):
            if 'uri'in buffer:buffer['uri']=os.path.relpath((path.parent/buffer['uri']).resolve(),dest)
            else:
                buffer['uri']=os.path.relpath(path,dest);buffer['byteLength']=path.stat().st_size
                for view in doc['bufferViews']:
                    if view.get('buffer',0)==bi:view['byteOffset']=view.get('byteOffset',0)+offset
        for im in doc.get('images',[]):
            if 'uri'in im:im['uri']=os.path.relpath((path.parent/im['uri']).resolve(),dest)
        output=dest/(candidate['id']+'.gltf');output.write_text(json.dumps(doc,separators=(',',':'))+'\n')
        records.append(dict(source=q.pin(path),view=q.pin(output),placement=candidate['placement'],kept=kept))
    assert len(records)==2
    (dest/'report.json').write_text(json.dumps(dict(status='WOOD_ONLY_JUNCTION_CONTEXT',records=records,limits=['Exact wood topology, transforms, UVs and source buffers retained.','Other receivers and foliage omitted only for junction visibility; full-scene contact audit remains separate.']),indent=2)+'\n')
if __name__=='__main__':main()
