"""Freeze seven source-initial mission UI checks; never launch or publish."""
import argparse, json, re, shutil, subprocess
from pathlib import Path
from prepare_native_physical_effect_ui import prepare, sha, dump, R, LIB

def main(out):
 stage=R/'level-editor/work/active/croisement02/tree42-all-mission-contracts-oct12-v2'
 plan=json.loads((stage/'draft-plan.json').read_text())
 prepare(out,receiver_assets=['croisement02-tree-42'])
 routes={'/library/'+row['destination']:row['staged']['path'] for row in plan['files']}
 routes['/library/scenes/croisement02.rhlos-map.json']=str(out/'bounded-receiver-map.json')
 for row in plan['files']:assert sha(Path(row['staged']['path']))==row['staged']['sha256']
 missions=[];pins={}
 def resources(value):
  if isinstance(value,dict):
   if 'path' in value and 'sha256' in value:
    path=Path(routes.get('/library/'+value['path'],str(LIB/value['path'])));assert sha(path)==value['sha256'],str(path);pins[str(path)]=value['sha256']
   for k,v in value.items():
    if k=='model' and isinstance(v,str) and 'model_sha256' in value:
     path=LIB/v;assert sha(path)==value['model_sha256'];pins[str(path)]=value['model_sha256']
    resources(v)
  elif isinstance(value,list):
   for v in value:resources(v)
 for entry in plan['catalog_additions']:
  resources(entry);contract=json.loads(Path(routes['/library/'+entry['contract']['path']]).read_text());resources(contract);missions.append({'entry':entry,'contract':contract})
 document=json.loads((out/'bounded-receiver-map.json').read_text())
 for ref in [*document['assetSources'],*document['sceneAssets']]:
  for key in ['model','descriptor']:
   p=LIB/ref[key];assert sha(p)==ref[key+'_sha256'];pins[str(p)]=ref[key+'_sha256']
  resources(json.loads((LIB/ref['descriptor']).read_text()))
 dump(out/'check-config.json',{'missions':missions,'cropRegion':{'x':615,'y':680,'width':330,'height':290},'scope':'Bounded tree42 receiver and unchanged scene terrain; complete declared native source initial context; startup scripts excluded'})
 shutil.copy2(R/'level-editor/blender/croisement02/tree42_mission_ui_proof.js',out/'check.js')
 run=(out/'run.mjs').read_text();run=re.sub(r'const privateRoutes=new Map\(Object.entries\(.*?\)\);','const privateRoutes=new Map(Object.entries('+json.dumps(routes)+'));',run,count=1)
 run=re.sub(r'/\^b05-.*?\$/.test\(p.stage.name\)',r'/^tree42-[0-6]-comparison$/.test(p.stage.name)',run)
 run=re.sub(r'/\^b05-.*?\$/.test\(row.name\)',r'/^tree42-[0-6]-(original|physical)-tick(0|28)$/.test(row.name)',run)
 run=run.replace('for(let i=0;i<1800;i++)','for(let i=0;i<7200;i++)')
 (out/'run.mjs').write_text(run)
 # Pin the committed adapter files explicitly. Other served source is frozen and listed.
 adapter=subprocess.check_output(['git','diff-tree','--no-commit-id','--name-only','-r','c42f624df'],text=True).splitlines()
 for key in adapter:
  expected=subprocess.check_output(['git','show','8c0d20097:'+key]);assert (out/'snapshot'/key).read_bytes()==expected,key
 dump(out/'adapter-authority.json',{'commit':'8c0d20097','base_adapter_commit':'c42f624df','files':{key:sha(out/'snapshot'/key) for key in adapter},'startup_scripts_executed':False})
 snap=json.loads((out/'source-snapshot.json').read_text())
 pins.update({str(p):sha(p) for p in [LIB/'3d-assets/index.json',LIB/'scenes/croisement02.rhlos-map.json',LIB/'mission-states/index.json',stage/'validation.json',stage/'handoff.json',*map(Path,routes.values())]})
 dump(out/'runtime-inputs.json',{**snap,**pins})
 pins.update({str(p):sha(p) for p in out.iterdir() if p.is_file() and p.name not in ['inputs.json','request.json']})
 dump(out/'inputs.json',pins)
 dump(out/'request.json',{'status':'PREPARED_NOT_LAUNCHED','run':str(out/'run.mjs'),'run_sha256':sha(out/'run.mjs'),'scope':'Seven actual mission switches plus first-mission rebind; source-initial context and 15 clocks, physical56tick cycle. Bounded receiver only, startup scripts not executed.','verified_resource_pins':len(pins),'adapter_commit':'8c0d20097'})
 print(json.dumps({'run':str(out/'run.mjs'),'sha256':sha(out/'run.mjs'),'pins':len(pins)}))
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--out',type=Path,required=True);a=p.parse_args();main(a.out.resolve())
