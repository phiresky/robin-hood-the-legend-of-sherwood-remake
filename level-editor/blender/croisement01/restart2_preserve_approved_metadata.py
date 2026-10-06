"""Carry existing movement/sight records through the approved assets' pivot changes."""
import copy
import json
import math
import sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from asset_index import write_asset_index
from review_evidence import sha
kind=sys.argv[1]
asset,node={'stump64':('croisement01-southwest-broken-stump','060'),'stump67':('croisement01-east-ivy-stump','054'),'stump65':('croisement01-southwest-cut-stump','055'),'tree20':('croisement01-tree-20','082'),'stump68':('croisement01-southeast-small-stump','059')}[kind]
R=ROOT/'level-editor/work/croisement01-refinement/restart2'/(kind+'-integration-v2')
scene=json.loads((ROOT/'level-editor/library/scenes/croisement01.rhlos-map.json').read_text())
p=R/'assets'/asset/'asset.json';d=json.loads(p.read_text())
assert 'gameplay' not in d
original=copy.deepcopy(d)
x,y,z=d['source_origin_scene'];new=[x,-y*math.sin(math.radians(35)),z*math.cos(math.radians(35))]
gameplay=None;records=[]
for n in [node]:
    asset='croisement01-group-'+n
    live=ROOT/'level-editor/library/3d-assets/croisement01'/asset/'asset.json'
    old=json.loads(live.read_text());g=old['gameplay'];placement=next(p for p in scene['placements'] if p['assets']==[asset]);t=placement['transform'];before=[t['dx'],t['dy'],t['dz']]
    required={'version','collision','surfaces','sightOrder','movementBlockers','doors','lifts','interiors','draft'}
    assert required<=set(g)<=required|{'jumpZones','jumpPairs','jumpSegments'}
    assert all(g[k]==[] for k in ['doors','lifts','interiors']) and g['collision']=='parts'
    if gameplay is None:gameplay=copy.deepcopy(g);gameplay['movementBlockers']=[];gameplay['sightOrder']={}
    assert gameplay['draft']==g['draft']
    gameplay['sightOrder'].update(g['sightOrder'])
    def point3(point):
        assert len(point)==3
        result=[point[i]+before[i]-new[i] for i in range(3)]
        assert max(abs(point[i]+before[i]-result[i]-new[i]) for i in range(3))<1e-9
        return result
    def edge3(edge):
        assert set(edge)=={'zone','a','b'}
        return dict(edge,a=point3(edge['a']),b=point3(edge['b']))
    for key in ['jumpZones','jumpPairs','jumpSegments']:
        if key not in g:continue
        translated=[]
        for original_jump in g[key]:
            jump=copy.deepcopy(original_jump)
            if key=='jumpZones':
                assert set(jump)=={'id','node','anchor','polygon','helperNeeded'}
                jump['anchor']=point3(jump['anchor']);jump['polygon']=[point3(v) for v in jump['polygon']]
            elif key=='jumpPairs':
                assert set(jump)=={'id','node','long','edges'}
                jump['edges']=[edge3(e) for e in jump['edges']]
            else:
                assert set(jump)=={'id','node','long','join','edge'}
                jump['join']=point3(jump['join']);jump['edge']=edge3(jump['edge'])
            translated.append(jump)
        gameplay[key]=translated
        records.append(dict(source_descriptor=str(live),source_descriptor_sha256=sha(live),jump_field=key,count=len(translated),world_coordinate_max_error_bound=1e-9,ids_links_and_flags_unchanged=True))

    for surface in gameplay['surfaces']:
        old_surface=next(s for s in g['surfaces'] if s['id']==surface['id'])
        assert not surface['holes']
        surface['polygon']=[[x+before[0]-new[0],y+before[1]-new[1]] for x,y in surface['polygon']]
        surface['height']=[h+before[2]-new[2] for h in surface['height']]
        projection=surface.get('projectionMaterials')
        if projection:
            assert projection['regions']==[]
            for key in ['planePoints','footprint']:
                projection[key]=[[v[i]+before[i]-new[i] for i in range(3)] for v in projection[key]]
                assert max(abs(a[i]+before[i]-b[i]-new[i]) for a,b in zip(old_surface['projectionMaterials'][key],projection[key]) for i in range(3))<1e-9
            projection['priorityHeight']+=before[2]-new[2]
            assert abs(old_surface['projectionMaterials']['priorityHeight']+before[2]-projection['priorityHeight']-new[2])<1e-9
        error=max(abs(a[i]+before[i]-b[i]-new[i]) for a,b in zip(old_surface['polygon'],surface['polygon']) for i in range(2))
        error=max(error,max(abs(a+before[2]-b-new[2]) for a,b in zip(old_surface['height'],surface['height'])))
        assert error<1e-9
        records.append(dict(source_descriptor=str(live),source_descriptor_sha256=sha(live),surface=surface['id'],world_coordinate_max_error=error,projection_material_semantics_unchanged=True))
    for blocker in g['movementBlockers']:
        assert not blocker['holes']
        translated=copy.deepcopy(blocker)
        translated['polygon']=[[px+before[0]-new[0],py+before[1]-new[1]] for px,py in blocker['polygon']]
        translated['height']=[v+before[2]-new[2] for v in blocker['height']]
        old_world=[[px+before[0],py+before[1],h+before[2]] for (px,py),h in zip(blocker['polygon'],blocker['height'])]
        new_world=[[px+new[0],py+new[1],h+new[2]] for (px,py),h in zip(translated['polygon'],translated['height'])]
        err=max(abs(a-b) for row1,row2 in zip(old_world,new_world) for a,b in zip(row1,row2));assert err<1e-9
        gameplay['movementBlockers'].append(translated)
        records.append(dict(source_descriptor=str(live),source_descriptor_sha256=sha(live),blocker=blocker['id'],world_coordinate_max_error=err,old_world=old_world,new_world=new_world))
    part=old['parts'][0];new_part=next(row for row in d['parts'] if row['node']==part['node'])
    for point,other in zip(part['obstacle_local_game']['points'],new_part['obstacle_local_game']['points']):
        for key,index in [('x',0),('y',1),('z_bottom',2),('z_top',2)]:assert abs(point[key]+before[index]-other[key]-new[index])<1e-9
backup=R/'original-export-asset.json';assert not backup.exists();backup.write_text(json.dumps(original,indent=2)+'\n')
d['gameplay']=gameplay;p.write_text(json.dumps(d,indent=2)+'\n');write_asset_index(R/'assets')
(R/'metadata-preservation.json').write_text(json.dumps(dict(status='PASS',scope='Exact existing metadata, pivot rebased; no new parity claim',records=records,sight_order=gameplay['sightOrder'],original_descriptor_sha256=sha(backup),descriptor_sha256=sha(p)),indent=2)+'\n')
print('PASS: existing blockers/sight order and world obstacle volumes preserved')
