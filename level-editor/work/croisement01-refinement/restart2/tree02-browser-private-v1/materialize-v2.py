from pathlib import Path
import os,json,hashlib
R=Path('level-editor/work/croisement01-refinement/restart2')
source=R/'tree02-publication-plan-v1/private-library'
target=R/'tree02-publication-plan-v1/browser-private-library-v2'
packet=R/'tree02-browser-private-v2'
assert not target.exists() and not packet.exists(), 'Versioned output already exists'
target.mkdir();packet.mkdir()
files=[]; skipped=[]
def walk(src,dst):
    for item in sorted(src.iterdir()):
        rel=item.relative_to(source)
        if item.name.startswith('.') or 'backup' in item.name.lower():
            skipped.append(str(rel));continue
        out=dst/item.name
        if rel==Path('game-data'):
            out.symlink_to(item.resolve(),target_is_directory=True);continue
        if item.is_dir():
            out.mkdir();walk(item,out)
        elif item.is_file():
            os.link(item.resolve(),out)
            a,b=item.stat(),out.stat()
            assert (a.st_dev,a.st_ino,a.st_size)==(b.st_dev,b.st_ino,b.st_size)
            files.append({'path':str(rel),'bytes':b.st_size,'device':b.st_dev,'inode':b.st_ino})
        else: raise RuntimeError(f'Unsupported resource {item}')
walk(source,target)
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
req=json.loads((R/'tree02-publication-plan-v1/browser-request.json').read_text())
assert sha(target/'scenes/croisement01.rhlos-map.json')==req['stage_map']['sha256']
assert sha(target/'3d-assets/index.json')==req['private_index']['sha256']
req['private_library']=str(target)
req['private_index']['path']=str(target/'3d-assets/index.json')
req['materialization']='Real directories and identical-inode hardlinked files; no chmod because hardlinks share canonical inode permissions. Browser library must be served read-only.'
(packet/'browser-request.json').write_text(json.dumps(req,indent=2)+'\n')
cfg=json.loads((R/'tree02-browser-private-v1/check-config.json').read_text());cfg['private_library']=str(target)
(packet/'check-config.json').write_text(json.dumps(cfg,indent=2)+'\n')
proof={'status':'PASS_IDENTICAL_INODE_RESOURCE_BYTES','source':str(source),'target':str(target),'files':files,'skipped':skipped,'file_count':len(files),'bytes':sum(x['bytes'] for x in files),'map_sha256':req['stage_map']['sha256'],'index_sha256':req['private_index']['sha256']}
(packet/'materialization-proof.json').write_text(json.dumps(proof,indent=2)+'\n')
handoff=json.loads((R/'tree02-browser-private-v1/ui-check-handoff.json').read_text())
handoff['private_library']=str(target);handoff['browser_request']=str(packet/'browser-request.json')
handoff['files'].pop(str(R/'tree02-browser-private-v1/check-config.json'))
for name in ['browser-request.json','check-config.json','materialization-proof.json']:
    handoff['files'][str(packet/name)]=sha(packet/name)
handoff['materialization']=req['materialization']
(packet/'ui-check-handoff.json').write_text(json.dumps(handoff,indent=2)+'\n')
print(json.dumps({'files':len(files),'bytes':proof['bytes'],'handoff':str(packet/'ui-check-handoff.json'),'handoff_sha256':sha(packet/'ui-check-handoff.json'),'pins':handoff['files']},indent=2))
