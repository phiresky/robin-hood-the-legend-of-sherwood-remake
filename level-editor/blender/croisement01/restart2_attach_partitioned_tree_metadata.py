"""Rebase exact partitioned tree gameplay into its approved standalone pivot."""
import copy,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from review_evidence import sha
from asset_index import write_asset_index
R=ROOT/'level-editor/work/croisement01-refinement/restart2'
kind=sys.argv[1];assert kind in {'tree21','tree22','tree71'}
group='062' if kind=='tree71' else '061'
asset='croisement01-tree-'+kind[4:];stage=R/(kind+'-integration-v2');p=stage/'assets'/asset/'asset.json'
source=R/('group'+group+'-partition-v1')/(kind+'-metadata.json');partition=json.loads(source.read_text());d=json.loads(p.read_text());assert 'gameplay' not in d
old=copy.deepcopy(d);g=copy.deepcopy(partition['gameplay']);x,y,z=partition['source_origin_scene'];before=[x,-y*math.sin(math.radians(35)),z*math.cos(math.radians(35))];x,y,z=d['source_origin_scene'];after=[x,-y*math.sin(math.radians(35)),z*math.cos(math.radians(35))]
scene=json.loads((ROOT/'level-editor/library/scenes/croisement01.rhlos-map.json').read_text());placement=next(p for p in scene['placements'] if p['assets']==['croisement01-group-'+group])
transform=placement['transform'];assert set(transform)=={'dx','dy','dz','rot_deg'} and transform['rot_deg']==0
actual_before=[transform[k] for k in ('dx','dy','dz')]
assert max(abs(a-b) for a,b in zip(before,actual_before))<1e-9
before=actual_before
assert set(g)=={'version','collision','surfaces','projectionReceivers','sightOrder','movementBlockers','doors','lifts','interiors','draft'}
assert all(g[k]==[] for k in ['surfaces','projectionReceivers','doors','lifts','interiors']) and g['collision']=='parts'
errors=[]
for row in g['movementBlockers']:
    assert not row['holes'];original=copy.deepcopy(row)
    row['polygon']=[[p[i]+before[i]-after[i] for i in range(2)] for p in row['polygon']]
    row['height']=[h+before[2]-after[2] for h in row['height']]
    error=max([abs(a[i]+before[i]-b[i]-after[i]) for a,b in zip(original['polygon'],row['polygon']) for i in range(2)]+[abs(a+before[2]-b-after[2]) for a,b in zip(original['height'],row['height'])]);assert error<1e-9;errors.append(error)
    assert {k:v for k,v in row.items() if k not in {'polygon','height'}}=={k:v for k,v in original.items() if k not in {'polygon','height'}}
for part in partition['parts']:
    target=next(p for p in d['parts'] if p['node']==part['node']);source_obstacle=part['obstacle_local_game'];target_obstacle=target['obstacle_local_game']
    assert {k:v for k,v in source_obstacle.items() if k!='points'}=={k:v for k,v in target_obstacle.items() if k!='points'}
    assert len(source_obstacle['points'])==len(target_obstacle['points'])
    for a,b in zip(source_obstacle['points'],target_obstacle['points']):
        for key,i in [('x',0),('y',1),('z_bottom',2),('z_top',2)]:assert abs(a[key]+before[i]-b[key]-after[i])<1e-9
backup=stage/'original-export-asset.json';assert not backup.exists();backup.write_text(json.dumps(old,indent=2)+'\n');d['gameplay']=g;p.write_text(json.dumps(d,indent=2)+'\n');write_asset_index(stage/'assets')
(stage/'metadata-preservation.json').write_text(json.dumps(dict(status='PASS',scope='Exact partitioned baseline metadata, rebased to reviewed standalone pivot',partition_sha256=sha(source),source_group_partition_proof_sha256=sha(R/('group'+group+'-partition-v1/proof.json')),movement_world_coordinate_errors=errors,sight_order_unchanged=True,all_noncoordinate_gameplay_fields_unchanged=True,original_native_obstacle_world_volume_unchanged=True,descriptor_sha256=sha(p)),indent=2)+'\n')
print('PASS',asset)
