from pathlib import Path
import hashlib,json,struct
r=Path('level-editor/work/croisement02-refinement/restart2-state');out=r/'remaining-local-origins-v1';sha=lambda b:hashlib.sha256(b).hexdigest();manifests={n:json.loads((r/n/'manifest.json').read_text())for n in ['approved-physical-endpoint-exports-v1','approved-physical-endpoint-exports-v2']};records=[]
for name,m in manifests.items():
 for row in m['records']:
  if name.endswith('v1')and row['id']!='croisement02-net-piege01-occupied-final-0':continue
  row=dict(row);row['source_export']=str(r/name/row['glb']);records.append(row)
def family(row):
 s=row['id']
 return 'north-cart'if'north-cart'in s else'south-cart'if'south-cart'in s else'net-piege01'if'piege01'in s else'net-piege03'if'piege03'in s else'south-field-fence'
anchors={}
for name in set(map(family,records)):
 parts=[p for row in records if family(row)==name for p in row['parts']];lo=[min(p['bounds'][0][i]for p in parts)for i in range(3)];hi=[max(p['bounds'][1][i]for p in parts)for i in range(3)];anchors[name]=[(lo[0]+hi[0])/2,(lo[2]+hi[2])/2,-(lo[1]+hi[1])/2]
for row in records:
 b=Path(row['source_export']).read_bytes();assert sha(b)==row['glb_sha256'];n,kind=struct.unpack_from('<II',b,12);assert kind==0x4e4f534a;g=json.loads(b[20:20+n]);scene=g['scenes'][g.get('scene',0)];anchor=anchors[family(row)];node=len(g['nodes']);g['nodes'].append({'name':'Reusable family origin','translation':[-v for v in anchor],'children':scene['nodes']});scene['nodes']=[node];j=json.dumps(g,separators=(',',':')).encode();j+=b' '*((-len(j))%4);tail=b[20+n:];data=struct.pack('<III',0x46546c67,2,20+len(j)+len(tail))+struct.pack('<II',len(j),kind)+j+tail;p=out/row['glb'];p.write_bytes(data);row.update(family=family(row),position=anchor,source_glb_sha256=row['glb_sha256'],glb_sha256=sha(data),binary_chunks_sha256=sha(tail),binary_chunks_unchanged=data[20+len(j):]==tail)
(out/'manifest.json').write_text(json.dumps({'scope':'Private reusable family-origin conversion; explicit binding translation restores reviewed scene coordinates.','origin_convention':'Common union-bounds center for each family, glTF Y-up. South fence anchor from both surviving runs; future intact model must use same anchor.','records':records,'anchors':anchors},indent=2)+'\n');print(len(records),'byte-exact binary payloads rebased')
