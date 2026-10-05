"""Preserve both stump gameplay references and all linked jump records across a pivot change."""
import copy,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from review_evidence import sha
from asset_index import write_asset_index
R=ROOT/'level-editor/work/croisement01-refinement/restart2/stump66-integration-v2';LIB=ROOT/'level-editor/library';asset='croisement01-south-cut-stump'
def main():
    path=R/'assets'/asset/'asset.json';d=json.loads(path.read_text());assert 'gameplay' not in d
    original=copy.deepcopy(d);x,y,z=d['source_origin_scene'];after=[x,-y*math.sin(math.radians(35)),z*math.cos(math.radians(35))]
    scene=json.loads((LIB/'scenes/croisement01.rhlos-map.json').read_text());combined=None;records=[]
    list_fields=['surfaces','movementBlockers','doors','lifts','interiors','jumpZones','jumpPairs','jumpSegments']
    required={'version','collision','surfaces','sightOrder','movementBlockers','doors','lifts','interiors','draft'}
    for node in [56,67]:
        old_id=f'croisement01-group-{node:03}';p=LIB/'3d-assets/croisement01'/old_id/'asset.json';old=json.loads(p.read_text());g=copy.deepcopy(old['gameplay'])
        assert required<=set(g)<=required|{'jumpZones','jumpPairs','jumpSegments'}
        placement=next(p for p in scene['placements'] if p['assets']==[old_id]);t=placement['transform'];assert t['rot_deg']==0;before=[t['dx'],t['dy'],t['dz']];errors=[]
        def point(v):
            assert len(v)==3;result=[v[i]+before[i]-after[i] for i in range(3)];errors.extend(abs(v[i]+before[i]-result[i]-after[i]) for i in range(3));return result
        def polygon_height(row):
            assert row['holes']==[]
            row['polygon']=[[v[i]+before[i]-after[i] for i in range(2)] for v in row['polygon']]
            row['height']=[h+before[2]-after[2] for h in row['height']]
        def edge(e):
            assert set(e)=={'zone','a','b'};e['a']=point(e['a']);e['b']=point(e['b'])
        for key in ['surfaces','movementBlockers']:
            for row,source in zip(g[key],old['gameplay'][key]):
                polygon_height(row)
                errors.extend(abs(a[i]+before[i]-b[i]-after[i]) for a,b in zip(source['polygon'],row['polygon']) for i in range(2));errors.extend(abs(a+before[2]-b-after[2]) for a,b in zip(source['height'],row['height']))
                projection=row.get('projectionMaterials')
                if projection:
                    assert projection['regions']==[]
                    for field in ['planePoints','footprint']:projection[field]=[point(v) for v in projection[field]]
                    projection['priorityHeight']+=before[2]-after[2]
                    errors.append(abs(source['projectionMaterials']['priorityHeight']+before[2]-projection['priorityHeight']-after[2]))
        for row in g.get('jumpZones',[]):
            assert set(row)=={'id','node','anchor','polygon','helperNeeded'};row['anchor']=point(row['anchor']);row['polygon']=[point(v) for v in row['polygon']]
        for row in g.get('jumpPairs',[]):
            assert set(row)=={'id','node','long','edges'}
            for e in row['edges']:edge(e)
        for row in g.get('jumpSegments',[]):
            assert set(row)=={'id','node','long','join','edge'};row['join']=point(row['join']);edge(row['edge'])
        assert all(g[k]==[] for k in ['doors','lifts','interiors']) and g['collision']=='parts'
        if combined is None:
            combined={k:copy.deepcopy(g[k]) for k in ['version','collision','draft']};combined['sightOrder']={}
        assert all(combined[k]==g[k] for k in ['version','collision','draft'])
        assert not set(combined['sightOrder'])&set(g['sightOrder']);combined['sightOrder'].update(g['sightOrder'])
        for key in list_fields:
            if key not in g:continue
            combined.setdefault(key,[]).extend(g[key]);assert len({v['id'] for v in combined[key]})==len(combined[key])
        for part in old['parts']:
            target=next(p for p in d['parts'] if p['node']==part['node']);a=part['obstacle_local_game'];b=target['obstacle_local_game'];assert {k:v for k,v in a.items() if k!='points'}=={k:v for k,v in b.items() if k!='points'}
            assert len(a['points'])==len(b['points'])
            for av,bv in zip(a['points'],b['points']):
                for k,i in [('x',0),('y',1),('z_bottom',2),('z_top',2)]:errors.append(abs(av[k]+before[i]-bv[k]-after[i]))
        assert max(errors,default=0)<1e-9
        records.append(dict(source_asset=old_id,source_descriptor_sha256=sha(p),source_placement=placement,world_coordinate_max_error=max(errors,default=0),preserved_counts={k:len(g[k]) for k in list_fields if k in g},ids_zone_links_node_ownership_and_flags_preserved=True))
    backup=R/'original-export-asset.json';assert not backup.exists();backup.write_text(json.dumps(original,indent=2)+'\n');d['gameplay']=combined;path.write_text(json.dumps(d,indent=2)+'\n');write_asset_index(R/'assets')
    (R/'metadata-preservation.json').write_text(json.dumps(dict(status='PASS',scope='Exact baseline gameplay56/67 merged with world coordinates preserved; no new parity claim',source_scene_sha256=sha(LIB/'scenes/croisement01.rhlos-map.json'),records=records,descriptor_sha256=sha(path)),indent=2)+'\n');print('PASS both stump references and jump links preserved')
if __name__=='__main__':main()
