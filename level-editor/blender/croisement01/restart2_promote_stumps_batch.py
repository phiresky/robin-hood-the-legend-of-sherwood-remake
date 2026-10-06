"""Promote the verified three-stump splice with rollback evidence."""
import copy,json,os,shutil,sys,tempfile
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from review_evidence import sha
from asset_index import validate_asset_index
R=ROOT/'level-editor/work/croisement01-refinement/restart2'
stage=R/'stumps64-67-69-integration-batch-v1';library=ROOT/'level-editor/library'
def read(p):return json.loads(p.read_text())
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main():
    revision=sys.argv[1] if len(sys.argv)>1 else 'full-editor-private-v2'
    assert revision.startswith('full-editor-private-v') and Path(revision).name==revision
    full_path=stage/revision/'result.json';full=read(full_path)
    assert full['status']=='PASS' and full['mapGroups']==58 and full['mapParts']==94
    config=read(stage/revision/'config.json')
    assert full['filesHashVerified']==len(config['files'])
    assert set(full['insertedAssets'])=={a.get('inserted_id',a['id']) for a in config['expected']['assets'] if a.get('editor_usage')!='map-background'}
    assert len(full['selectionChecks'])==config['expected']['groups']
    checker_path=stage/revision/'checker-provenance.json'
    for path,digest in read(checker_path).items():assert sha(ROOT/path)==digest,path
    for path,digest in config['protected_live_files'].items():assert sha(Path(path))==digest,path
    proof=read(stage/'scene-splice-proof.json')
    assert sha(stage/'croisement01.rhlos-map.json')==proof['staged_scene_sha256']
    assert next(f['sha256'] for f in config['files'] if f['path']=='scenes/croisement01.rhlos-map.json')==proof['staged_scene_sha256']
    cases={'stump64':'croisement01-southwest-broken-stump','stump69':'croisement01-central-ivy-stump','stump67':'croisement01-east-ivy-stump'}
    assert set(cases.values())<=set(full['insertedAssets'])
    for kind,asset in cases.items():
        source=stage/'assets'/asset;e=proof['assets'][asset]
        assert sha(source/'model.glb')==e['model_sha256'] and sha(source/'asset.json')==e['descriptor_sha256']
        review=read(R/(kind+'-integration-v2')/'export-visual-root-review.json')
        assert review['status']=='scoped export appearance PASS'
        base=R/(kind+'-integration-v2')
        assert review['files']['assets/'+asset+'/model.glb']==e['model_sha256']
        for name,digest in review['files'].items():assert sha(base/name)==digest
        assert sha(base/'metadata-preservation.json')==e['metadata_sha256']
        assert sha(base/'export-proof.json')==e['export_sha256']
        assert sha(R/('approved-'+kind+'-wood-fill-v1')/asset/'user-texture-decision.json')==e['approval_sha256']
    target_base=library/'3d-assets/croisement01'
    scene=library/'scenes/croisement01.rhlos-map.json';index=library/'3d-assets/index.json'
    assert sha(scene)==proof['live_scene_sha256'];before=read(index);index_hash=sha(index)
    scoped=set(cases.values());entries=read(stage/'assets/index.json')['assets'];assert {e['id'] for e in entries}==scoped
    for asset in cases.values():assert not (target_base/asset).exists()
    updated=copy.deepcopy(before);replacements={}
    for row in entries:
        entry=copy.deepcopy(row)
        for key in ['model','descriptor']:entry[key]='croisement01/'+entry[key]
        replacements[entry['id']]=entry
    updated['assets'] += [replacements[a] for a in cases.values()]
    assert [e for e in updated['assets'] if e['id'] not in scoped]==[e for e in before['assets'] if e['id'] not in scoped]
    if '--check' in sys.argv:
        print(json.dumps(dict(status='PASS',mode='read-only promotion preflight',scene_sha256=sha(scene),index_sha256=index_hash,assets=sorted(scoped))))
        return
    backup=stage/'publication-backup-v1';backup.mkdir(exist_ok=False)
    shutil.copy2(scene,backup/scene.name);shutil.copy2(index,backup/'asset-index.json')
    protected={str(p.relative_to(library)):sha(p) for p in (target_base).rglob('*') if p.is_file()}
    write(backup/'receipt.json',dict(scene_sha256=sha(scene),index_sha256=index_hash,protected_existing_assets=protected,rollback='Restore this scene after hash checks; coordinate removal of only the new scoped assets and index rollback to preserve later unrelated entries.'))
    assert sha(scene)==proof['live_scene_sha256'] and sha(index)==index_hash
    installed={}
    for asset in cases.values():
        source=stage/'assets'/asset;target=target_base/asset
        target.mkdir()
        for file in source.iterdir():
            assert file.is_file()
            fd,name=tempfile.mkstemp(prefix='.combined-',dir=target);os.close(fd);tmp=Path(name)
            shutil.copy2(file,tmp);assert sha(tmp)==sha(file);os.replace(tmp,target/file.name)
            installed[str((target/file.name).relative_to(library))]=sha(target/file.name)
    validate_asset_index(library/'3d-assets',updated)
    for path,digest in protected.items():assert sha(library/path)==digest
    assert sha(scene)==proof['live_scene_sha256'] and sha(index)==index_hash
    temp=index.with_name('.index-croisement01-combined.json');write(temp,updated);os.replace(temp,index)
    temp=scene.with_name('.croisement01-combined.json');shutil.copy2(stage/scene.name,temp);os.replace(temp,scene)
    assert sha(scene)==proof['staged_scene_sha256']
    write(stage/'publication.json',dict(status='installed; bounded live proof pending',scene_sha256=sha(scene),index_sha256=sha(index),published_files=installed,private_full_editor=dict(path=str(full_path),sha256=sha(full_path),config_sha256=sha(stage/revision/'config.json'),checker_provenance_sha256=sha(checker_path)),backup=str(backup),unchanged_other_placements=proof['unchanged_other_placements'],unrelated_palette_entries_preserved=len(before['assets']),protected_existing_asset_files=len(protected),scope=proof['scopes']))
    print(stage/'publication.json')
if __name__=='__main__':main()
