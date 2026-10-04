"""Enumerate foreground, background and gameplay effects of every native patch."""
import argparse,json,hashlib
from pathlib import Path
from catalog import OUT

def main():
 parser=argparse.ArgumentParser();parser.add_argument('output',type=Path);args=parser.parse_args()
 source=OUT/'source-states/layers.json';layers=json.loads(source.read_text());records=[]
 for kind,rows in [('native',layers['patches']),('mission',layers['mission_patches'])]:
  for patch in rows:
   s=patch['state'];states=patch.get('states',{})
   def frames(name):return [dict(f,sha256=hashlib.sha256((source.parent/f['image']).read_bytes()).hexdigest()) for f in states.get(name,{}).get('frames',[])]
   transition=frames('transition');integrated=bool(s['integrate_in_background'])
   if kind=='mission' and integrated and s['transition_animation_valid'] and not transition:raise ValueError('Missing terminal background frame')
   records.append(dict(id=patch['id'],kind=kind,mission=patch.get('mission'),runtime_patch_index=patch.get('runtime_patch_index',int(patch['id'].split('-')[-1])),
    initial=dict(foreground_enabled=s['start_animation_valid'],foreground=frames('initial'),background='unchanged',active_obstacles=s['old_sight_obstacles'],active_masks=s['old_masks']),
    transition=dict(foreground_enabled=s['transition_animation_valid'],foreground=transition,frame_order='forward on apply; reverse on reversible unapply',gameplay_swap='after transition completes'),
    applied=dict(foreground_enabled=s['end_animation_valid'],foreground=frames('final'),background_terminal_transition=transition[-1] if integrated and transition else None,background_operation='composite terminal transition frame' if integrated else 'unchanged',active_obstacles=s['new_sight_obstacles'],active_masks=s['new_masks']),
    unapply=dict(allowed_through_apply=not s['definitive'],background_operation='restore captured background' if integrated else 'unchanged',foreground_endpoint='initial if valid, otherwise disabled'),
    forced_reset=dict(background_operation='restore captured background if previously applied',foreground_endpoint='initial if valid, otherwise disabled',restore_initial_active=s['active'],restore_old_gameplay=True),
    preservation=dict(state=s,script_binding='native patch index and mission scope unchanged'),status='contract preserved; receiver-specific visual integration pending'))
 args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(dict(source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),patches=records,counts=dict(total=len(records),mission=sum(r['kind']=='mission' for r in records),background_changes=sum(r['applied']['background_operation']!='unchanged' for r in records),missing_final_sprite=sum(not r['applied']['foreground_enabled'] for r in records))),indent=2)+'\n')
 print(args.output)
if __name__=='__main__':main()
