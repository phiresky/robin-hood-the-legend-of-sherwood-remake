"""Keep patch-controlled obstacles distinct from visible base-map scenery."""
import json
from collections import Counter
from catalog import OUT


def main():
    source=OUT/'source-states/layers.json';data=json.loads(source.read_text())
    inventory=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    masks={(r['layer'],r['layer_index']):r['index'] for r in inventory['masks']}
    records=[]
    for patch in data['patches']:
        state=patch['state'];old=state['old_sight_obstacles'];new=state['new_sight_obstacles']
        records.append(dict(id=patch['id'],old_obstacles=old,new_obstacles=new,
            initial_active=old,applied_active=new,initial_inactive=new,applied_inactive=old,
            old_masks=[masks[(m['layer'],m['index'])] for m in state['old_masks']],
            new_masks=[masks[(m['layer'],m['index'])] for m in state['new_masks']],
            graphic=patch['graphic'],definitive=state['definitive'],
            geometry_role='state-controlled obstacle metadata; visible geometry requires separate sprite evidence' if old or new else 'state/pathfinding metadata'))
    mission=[]
    for patch in data['mission_patches']:
        frames={name:[dict(image=f['image'],bbox=f['bbox'],delay=f.get('delay')) for f in value['frames']] for name,value in patch['states'].items()}
        for sequence in frames.values():
            for frame in sequence:
                if not (source.parent/frame['image']).is_file():raise ValueError('Missing mission frame')
        mission.append(dict(id=patch['id'],mission=patch['mission'],runtime_patch_index=patch['runtime_patch_index'],name=patch['name'],states=frames))
    dest=OUT/'state-review';dest.mkdir(exist_ok=True)
    (dest/'inventory.json').write_text(json.dumps(dict(native_patches=records,mission_patches=mission,
        semantics='Old masks/obstacles are active before application, new masks/obstacles after application. Definitive patches do not toggle back through Apply. Native obstacle metadata is not independently visible artwork.',
        status='state relationships audited; mission visuals and integrated state assembly pending',
        mission_profiles=dict(Counter(p['name'] for p in mission))),indent=2)+'\n')
    print('Audited',len(records),'native patches and',len(mission),'mission patches')

if __name__=='__main__':main()
