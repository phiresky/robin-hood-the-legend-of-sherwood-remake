"""Verify and transactionally install the full user-authorized Sherwood publication."""
import argparse,hashlib,json,math,os,shutil,sys
from pathlib import Path
EDITOR=Path(__file__).resolve().parents[2]
sys.path[:0]=[str(EDITOR/'refinement'),str(EDITOR/'refinement/blender')]
from asset_index import generate_asset_index,write_asset_index
from scene_manifest import scene_metadata
from promote_staged_publication import library_lock
from lossy_assets import read_glb


def read(p):return json.loads(Path(p).read_text())
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def write(p,d):Path(p).write_text(json.dumps(d,indent=2)+'\n')


def verify(stage,check_live=True):
    stage=Path(stage).resolve();live=EDITOR/'library';handoff=read(stage/'handoff.json')
    assert handoff['status']=='PASS' and handoff['texture_review']=='pending'
    if check_live:
        for path,digest in handoff['before'].items():assert sha(live/path)==digest,('Changed live input',path)
    for path,digest in handoff['protected_files'].items():assert sha(path)==digest,('Changed source',path)
    document=read(stage/'sherwood.rhlos-map.json');catalog=read(EDITOR/'work/needs-review/sherwood-refinement/grouping-review/catalog.json')
    expected={g['id'] for g in catalog['groups']};assert len(expected)==80
    assert {r['id'] for r in document['assetSources']}==expected
    assert [r['id'] for r in document['sceneAssets']]==['sherwood-terrain']
    assert len(document['placements'])==80
    placements={p['id']:p for p in document['placements']};level=read(EDITOR.parent/'datadirs/fullgame_gog_hackable/Data/Levels/Sherwood.rhp.json')
    native=set();parts=set();components=set();artifacts={};sin,cos=math.sin(math.radians(35)),math.cos(math.radians(35))
    for ref in document['assetSources']+document['sceneAssets']:
        directory=stage/'map-assets';d=read(directory/ref['descriptor']);assert d['id']==ref['id']
        assert 'group-' not in d['id'];assert d['texture_candidate']['worker_sha256']==handoff['candidate_worker_sha256']
        for key in ('model','descriptor'):
            path=directory/ref[key];assert sha(path)==ref[key+'_sha256'];artifacts[str(path.relative_to(stage))]=sha(path)
        gltf,_,_=read_glb(directory/ref['model'])
        assert all('baseColorTexture' in m.get('pbrMetallicRoughness',{}) for m in gltf['materials']),d['id']
        assert not any('uri' in x for x in gltf.get('buffers',[])+gltf.get('images',[]))
        nodes={n.get('name') for n in gltf['nodes']};assert {p['node'] for p in d['parts']}<=nodes
        pivot=d['source_origin_scene']
        if d['id']!='sherwood-terrain':
            p=placements[d['id']];assert p['assets']==[d['id']]
            assert p['transform']=={'dx':pivot[0],'dy':-pivot[1]*sin,'dz':pivot[2]*cos,'rot_deg':0}
        for c in d['components']:
            assert c['name'] not in components;components.add(c['name'])
        for part in d['parts']:
            assert part['node'] not in parts;parts.add(part['node'])
            if part.get('scenery'):continue
            source=part['source_obstacle'];native.add(source)
            actual=part['obstacle_local_game'];original=level['sight_obstacles'][source]
            assert {k:v for k,v in actual.items() if k!='points'}=={k:v for k,v in original.items() if k!='points'}
            if part.get('source_components'):
                assert len(actual['points'])>=3 and part.get('authored_source_components')
            else:
                assert len(actual['points'])==len(original['points'])
                for a,b in zip(actual['points'],original['points']):
                    for key,offset in [('x',pivot[0]),('y',-pivot[1]*sin),('z_bottom',pivot[2]*cos),('z_top',pivot[2]*cos)]:assert abs(a[key]+offset-b[key])<1e-8
    plan=read(EDITOR/'work/needs-review/sherwood-refinement/grouping-review/plan.json')
    assert components==set(plan['assignments'])
    expected_native={p['obstacle'] for g in catalog['groups'] for p in g['parts'] if 'obstacle' in p}
    assert native==expected_native
    assert len(generate_asset_index(stage/'map-assets/3d-assets')['assets'])==81
    scene_metadata(stage/'map-assets',document)
    artifacts['sherwood.rhlos-map.json']=sha(stage/'sherwood.rhlos-map.json')
    result={'status':'PASS','assets':81,'placements':80,'parts':len(parts),'mesh_components':len(components),'native_sources':len(native),'artifacts':artifacts,'checks':['exact approved mesh ownership','exported world positions and triangle counts','native obstacle flags and unsplit world footprints','explicit split component footprints','runtime scene metadata','all embedded textures and reference hashes']}
    write(stage/'verification.json',result);print(json.dumps({k:v for k,v in result.items() if k!='artifacts'}),flush=True)
    return result


def install(stage):
    stage=Path(stage).resolve();live=EDITOR/'library'
    with library_lock(live):
        checked=verify(stage);old=read(live/'scenes/sherwood.rhlos-map.json')
        old_ids={r['id'] for r in old['assetSources']+old['sceneAssets']}
        new=read(stage/'sherwood.rhlos-map.json');new_ids={r['id'] for r in new['assetSources']+new['sceneAssets']}
        # Replacing the whole directory also removes assets absent from the
        # Sherwood map itself. Other maps may still place those older exports.
        existing_ids={read(p)['id'] for p in (live/'3d-assets/sherwood').glob('*/asset.json')}
        for path in (live/'scenes').glob('*.rhlos-map.json'):
            if path.name=='sherwood.rhlos-map.json':continue
            assert not {r['id'] for r in read(path).get('assetSources',[])+read(path).get('sceneAssets',[])} & (old_ids|new_ids|existing_ids),('Shared scene asset requires migration before directory replacement',path)
        backup=stage/'installation-backup';backup.mkdir(exist_ok=False)
        map_path=live/'scenes/sherwood.rhlos-map.json';index=live/'3d-assets/index.json';target=live/'3d-assets/sherwood'
        shutil.copy2(map_path,backup/map_path.name);shutil.copy2(index,backup/'index.json')
        pending=live/'3d-assets/.sherwood-full-publication';assert not pending.exists()
        shutil.copytree(stage/'map-assets/3d-assets/sherwood',pending)
        os.replace(target,backup/'sherwood')
        try:
            os.replace(pending,target)
            temp=map_path.with_suffix('.publish.tmp');shutil.copy2(stage/map_path.name,temp);os.replace(temp,map_path)
            write_asset_index(live/'3d-assets')
            for ref in new['assetSources']+new['sceneAssets']:
                for key in ('model','descriptor'):assert sha(live/ref[key])==ref[key+'_sha256']
            assert not (old_ids-new_ids)&{p.name for p in target.iterdir()}
            scene_metadata(live,new)
        except BaseException:
            if target.exists():shutil.rmtree(target)
            os.replace(backup/'sherwood',target);shutil.copy2(backup/map_path.name,map_path);shutil.copy2(backup/'index.json',index)
            raise
        result={'status':'APPLIED','installed_assets':sorted(new_ids),'retired_asset_ids':sorted(old_ids-new_ids),'map_sha256':sha(map_path),'backup':str(backup),'texture_review':'pending','authorization':'skip the gallery and publish the assets to main library','verification':'PASS'}
        write(stage/'installation.json',result);print(json.dumps(result),flush=True)

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('stage');p.add_argument('--install',action='store_true');a=p.parse_args()
    (install if a.install else verify)(a.stage)
