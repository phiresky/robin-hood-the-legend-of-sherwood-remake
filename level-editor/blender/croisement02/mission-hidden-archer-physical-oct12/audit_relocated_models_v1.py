"""Reopen retained archer scenes without rendering or changing their bytes."""
import bpy,json,hashlib,sys
from pathlib import Path
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from operational_records import PACKET,sha
packet=json.loads(PACKET.read_text());rows=[]
for row in packet['models']:
 p=Path(row['path']);assert sha(p)==row['sha256'];bpy.ops.wm.open_mainfile(filepath=str(p))
 images=[];missing=[]
 for image in bpy.data.images:
  packed=bool(image.packed_file or image.packed_files)
  external=image.source in ['FILE','MOVIE','SEQUENCE','TILED'] and not packed
  resolved=Path(bpy.path.abspath(image.filepath)) if image.filepath else None
  record=dict(name=image.name,source=image.source,packed=packed,filepath=image.filepath,external_required=external,exists=bool(resolved and resolved.is_file()))
  if external and not record['exists']:missing.append(record)
  images.append(record)
 libraries=[dict(path=l.filepath,exists=Path(bpy.path.abspath(l.filepath)).is_file()) for l in bpy.data.libraries]
 assert sha(p)==row['sha256']
 rows.append(dict(**row,images=images,libraries=libraries,missing=missing))
result=dict(models=rows,models_checked=len(rows),missing_external=sum(len(r['missing'])+sum(not l['exists'] for l in r['libraries']) for r in rows),model_bytes_unchanged=True)
(PACKET.parent/'model-dependencies.json').write_text(json.dumps(result,indent=2)+'\n')
print('ARCHER_RELOCATION_MODEL_AUDIT',result['models_checked'],result['missing_external'])
assert result['missing_external']==0
