"""Stage every approved Sherwood group with completed, review-pending textures."""
import argparse,copy,hashlib,json,math,re,sys
from pathlib import Path
import bpy
import numpy as np
HERE=Path(__file__).resolve().parent;EDITOR=HERE.parents[1]
sys.path[:0]=[str(HERE),str(EDITOR/'refinement'),str(EDITOR/'refinement/blender')]
from stage_editor_migration import sha,write,finalize_glb
from export_editor import export_editor
from editor_catalog import asset_type
from asset_index import write_asset_index
from verify_editor_handoff import points,deviation
from lossy_assets import read_glb,accessor_array,mesh_instances
import render_slots


def main(candidate,output):
    candidate=Path(candidate).resolve();output=Path(output).resolve();live=EDITOR/'library'
    fill=json.loads((candidate/'fill.json').read_text());verified=json.loads((candidate/'saved-worker-verification.json').read_text())
    assert verified['status']=='PASS_SAVED_TEXTURE_CANDIDATE' and verified['worker_sha256']==sha(candidate/'candidate.blend')==fill['worker_sha256']
    plan_path=EDITOR/'work/needs-review/sherwood-refinement/grouping-review/plan.json';plan=json.loads(plan_path.read_text())
    catalog_path=plan_path.with_name('catalog.json');assert sha(catalog_path)==plan['catalog_sha256']
    catalog=json.loads(catalog_path.read_text());assert len(catalog['groups'])==80
    assert len(fill['assets'])==86
    for row in fill['assets']:
        r=json.loads((candidate/'renders'/row['asset']/'review.json').read_text())
        assert r['worker_sha256']==fill['worker_sha256'] and r['residual_unknown_pixels']==0
    output.mkdir(parents=True,exist_ok=False)
    old=json.loads((live/'scenes/sherwood.rhlos-map.json').read_text())
    before={'scenes/sherwood.rhlos-map.json':sha(live/'scenes/sherwood.rhlos-map.json')}
    descriptors={}
    for r in old['assetSources']+old['sceneAssets']:
        descriptors[r['id']]=json.loads((live/r['descriptor']).read_text())
        for key in ('model','descriptor'):before[r[key]]=sha(live/r[key])
    level_path=EDITOR.parent/'datadirs/fullgame_gog_hackable/Data/Levels/Sherwood.rhp.json'
    level=json.loads(level_path.read_text());sin,cos=math.sin(math.radians(35)),math.cos(math.radians(35))
    render_slots.acquire();bpy.ops.wm.open_mainfile(filepath=str(candidate/'candidate.blend'))
    bpy.context.window.scene=bpy.data.scenes['Sherwood Editor Migration']
    groups={};working=bpy.data.collections['Sherwood Working']
    for obj in working.objects:
        if obj.type!='MESH':continue
        assert plan['assignments'][obj.name]==obj['asset_group']
        groups.setdefault(obj['asset_group'],[]).append(obj)
    assert set(groups)=={g['id'] for g in plan['groups']}|{'sherwood-terrain'}
    # Publication IDs are portable slugs; retain exact authored selectors separately.
    selectors={};export_catalog=copy.deepcopy(catalog)
    for group in export_catalog['groups']:
        group['parts']=[p for p in group['parts'] if p.get('obstacle')!=48]
        for part in group['parts']:
            if 'components' not in part:continue
            normalized=[]
            for name in part['components']:
                token=re.sub('[^a-z0-9]+','-',name.lower()).strip('-')+'-'+hashlib.sha256(name.encode()).hexdigest()[:8]
                selectors[token]=name;normalized.append(token)
                bpy.data.objects[name]['projection_component']=token
            part['components']=normalized
    export_catalog.get('canonical_owners',{}).pop('building-048',None)
    write(output/'export-catalog.json',export_catalog);write(output/'component-selectors.json',selectors)
    document=copy.deepcopy(old);document['assetSources']=[];document['sceneAssets']=[];document['placements']=[]
    reports=[];root=output/'map-assets/3d-assets'
    for identity,objects in sorted(groups.items()):
        model=root/'sherwood'/identity/'model.glb'
        world=points(objects);low=world.min(0);high=world.max(0)
        pivot=([0,0,0] if identity=='sherwood-terrain' else
               descriptors.get(identity,{}).get('source_origin_scene',[(low[0]+high[0])/2,(low[1]+high[1])/2,0]))
        report=export_editor('Sherwood',model,asset_id=identity,standalone_pivot=pivot,
            catalog=export_catalog,level=level)
        descriptor=report['asset'];descriptor['source_origin_scene']=pivot
        if identity=='sherwood-central-oak':
            descriptor['parts'].append({'node':'building-048','name':'Central oak upper trunk','source_obstacle':48})
        for part in descriptor['parts']:
            if part.get('source_components'):
                part['authored_source_components']=[selectors[t] for t in part['source_components']]
            elif not part.get('scenery'):
                obstacle=copy.deepcopy(level['sight_obstacles'][part['source_obstacle']])
                for point in obstacle['points']:
                    point['x']-=pivot[0];point['y']+=pivot[1]*sin
                    point['z_bottom']-=pivot[2]*cos;point['z_top']-=pivot[2]*cos
                part['obstacle_local_game']=obstacle
        descriptor['asset_type']=asset_type(descriptor['name'])
        descriptor['model_review']={'status':'approved','grouping_catalog_sha256':sha(catalog_path)}
        descriptor['texture_candidate']={'status':'published-at-user-request-review-pending','publication_authorization':'skip the gallery and publish the assets to main library','worker_sha256':fill['worker_sha256'],'source_worker_sha256':fill['source_worker_sha256'],'fill_report_sha256':sha(candidate/'fill.json')}
        finalize_glb(model,descriptor);write(model.with_name('asset.json'),descriptor)
        gltf,binary,_=read_glb(model);instances=mesh_instances(gltf);exported=[];triangles=0
        for index,mesh in enumerate(gltf['meshes']):
            matrix=instances[index]
            for primitive in mesh['primitives']:
                vertices=accessor_array(gltf,binary,primitive['attributes']['POSITION'],True)
                exported.append(vertices@matrix[:3,:3].T+matrix[:3,3]+pivot)
                triangles+=len(accessor_array(gltf,binary,primitive['indices']))//3
        actual=np.concatenate(exported);drift=max(deviation(world,actual),deviation(actual,world));count=0
        for obj in objects:obj.data.calc_loop_triangles();count+=len(obj.data.loop_triangles)
        assert drift<.001 and count==triangles,(identity,drift,count,triangles)
        ref={'id':identity,'resources':[],'model':f'3d-assets/sherwood/{identity}/model.glb','model_sha256':sha(model),'descriptor':f'3d-assets/sherwood/{identity}/asset.json','descriptor_sha256':sha(model.with_name('asset.json'))}
        if identity=='sherwood-terrain':document['sceneAssets'].append({**ref,'role':'ground'})
        else:
            document['assetSources'].append(ref)
            document['placements'].append({'id':identity,'assets':[identity],'transform':{'dx':pivot[0],'dy':-pivot[1]*sin,'dz':pivot[2]*cos,'rot_deg':0}})
        reports.append({'asset':identity,'triangles':triangles,'maximum_world_vertex_drift':drift,'meshes':len(objects)})
        print(json.dumps(reports[-1]),flush=True)
    write(output/'sherwood.rhlos-map.json',document);write_asset_index(root)
    protected=[candidate/'candidate.blend',candidate/'fill.json',candidate/'saved-worker-verification.json',catalog_path,plan_path,level_path]
    write(output/'handoff.json',{'status':'PASS','assets':reports,'before':before,'protected_files':{str(p):sha(p) for p in protected},'candidate_worker_sha256':fill['worker_sha256'],'publication_authorization':'skip the gallery and publish the assets to main library','texture_review':'pending'})
    render_slots.release()
    print('FULL PUBLICATION STAGED',flush=True)

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--candidate',required=True);p.add_argument('--output',required=True)
    a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);main(a.candidate,a.output)
