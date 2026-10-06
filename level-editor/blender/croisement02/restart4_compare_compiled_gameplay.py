"""Compare compiled world behavior, resolving placement-dependent indices."""
import copy,json,pathlib,sys,math
folder=pathlib.Path(sys.argv[1])
def normalized(raw):
 d=copy.deepcopy(raw);d.pop('warnings')
 for obstacle in d['sight_obstacles']:
  if 'material_indices' in obstacle:obstacle['material_indices']=[d['material_sectors'][i] for i in obstacle['material_indices']]
 d['sight_material_indices']=[d['material_sectors'][i] for i in d['sight_material_indices']]
 transitions={}
 for t in d['movement_transitions']:
  t['id']=t['id'].split('/')[-1]
  for c in t['motion_changes']:transitions[(c['layer'],c['sector'],c['changing_obstacle'])]=t['id']
 for li,layer in enumerate(d['motion_data']['layers']):
  for si,sector in enumerate(layer):
   for o in sector.get('obstacles',[]):
    state=o['state_id'];labels=[]
    for bit in range(state.bit_length()):
     if state&(1<<bit):labels.append([transitions[(li,sum(len(x) for x in d['motion_data']['layers'][:li])+si,bit//2)],bit%2])
    o['state_id']=labels
   sector['obstacles']=sorted(sector.get('obstacles',[]),key=lambda x:json.dumps(x['state_id']))
 for t in d['movement_transitions']:
  for c in t['motion_changes']:c['changing_obstacle']=transitions[(c['layer'],c['sector'],c['changing_obstacle'])]
  for key in ['initial_sight','applied_sight']:
   if key in t:t[key]=[d['sight_obstacles'][i] for i in t[key]]
 d['movement_transitions'].sort(key=lambda x:x['id']);d['sound_sources'].sort(key=lambda x:x['id'])
 return d
maxdiff=0;count=0
# Precise polygon coordinates may intentionally serialize as decimal strings.
def compare(a,b,p=''):
 global maxdiff,count
 if isinstance(a,(int,float)) and not isinstance(a,bool) or isinstance(a,str) and p.find('/precise_polygon/')>=0:
  delta=abs(float(a)-float(b));maxdiff=max(maxdiff,delta);count+=1
  assert delta<1e-8,(p,a,b,delta);return
 assert type(a)==type(b),(p,type(a),type(b))
 if isinstance(a,dict):
  assert a.keys()==b.keys(),p
  for k in a:compare(a[k],b[k],p+'/'+k)
 elif isinstance(a,list):
  assert len(a)==len(b),(p,len(a),len(b))
  for i,(x,y) in enumerate(zip(a,b)):compare(x,y,p+'/'+str(i))
 else:assert a==b,(p,a,b)
a=normalized(json.load(open(folder/'before.json')));b=normalized(json.load(open(folder/'after.json')));compare(a,b)
report={'pass':True,'numeric_values_compared':count,'max_world_or_numeric_drift':maxdiff,'normalization':['Placement prefixes removed from stable transition IDs','Changing-obstacle state bits resolved to stable transition ID and initial/applied phase','Sight material indices resolved to exact material sectors','Sight transition indices resolved to exact world obstacles','Sound records sorted by stable sound ID'],'limitations':'Retains existing best-effort draft warnings; does not claim new gameplay authoring or complete native parity.'}
(folder/'semantic-comparison.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
