"""Pin shared external glTF buffers before the private sign context proof."""
from pathlib import Path
import json
from restart10_prepare_physical_signs import glb_json,sha,OUT


def main():
    root=OUT/'restart10-physical-signs/input-v2'
    manifest=json.loads((root/'manifest.json').read_text());rows=[]
    for row in manifest['context']:
        p=Path(row['file']);assert sha(p)==row['sha256'];g=glb_json(p);resources=[]
        for value in g.get('buffers',[])+g.get('images',[]):
            uri=value.get('uri')
            if not uri or uri.startswith('data:'):continue
            f=(p.parent/uri).resolve();digest=sha(f)
            assert f.stem==digest,(f,digest)
            resources.append(dict(uri=uri,file=str(f),sha256=digest))
        rows.append(dict(id=row['id'],resources=resources))
    data=dict(status='Hash-verified external glTF resources; no source mutation',assets=rows)
    path=root/'resources-v1.json'
    if path.exists():assert json.loads(path.read_text())==data
    else:path.write_text(json.dumps(data,indent=2)+'\n')

if __name__=='__main__':main()
