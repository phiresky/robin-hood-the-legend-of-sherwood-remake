"""Prepare a pinned Night gate UI check without launching or publishing."""
import argparse,json,re,shutil,sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'croisement02'))
from prepare_native_physical_effect_ui import prepare,sha,dump,R,LIB

def main(out):
 stage=R/'level-editor/work/active/york/restart2/restart119-night-gate-qualification-v1'
 handoff=json.loads((stage/'handoff.json').read_text());entry=handoff['entry'];private=Path(handoff['library'])
 prepare(out,receiver_assets=['croisement02-tree-42'])
 routes={'/library/'+str(p.relative_to(private)):str(p) for p in private.rglob('*') if p.is_file()}
 canonical=json.loads((LIB/'scenes/york.rhlos-map.json').read_text());selected={'york-castle-west-gatehouse'}
 bounded={**canonical,'assetSources':[r for r in canonical['assetSources'] if r['id'] in selected],'placements':[r for r in canonical['placements'] if set(r['assets'])<=selected]}
 assert {a for r in bounded['placements'] for a in r['assets']}==selected
 dump(out/'bounded-receiver-map.json',bounded);routes['/library/scenes/york.rhlos-map.json']=str(out/'bounded-receiver-map.json')
 # Use the qualified private catalog as the sole state choice for this proof.
 shutil.copy2(private/'mission-states/index.json',out/'candidate-catalog.json')
 oldconfig=R/'level-editor/work/active/york/restart2/restart60-gate-ui-request-v1/config.json'
 views=json.loads(oldconfig.read_text())['views'];dump(out/'check-config.json',{'entry':entry,'views':views})
 shutil.copy2(R/'level-editor/blender/york/night_gate_ui_proof.js',out/'check.js')
 editor=(out/'editor.tsx').read_text().replace('croisement02','york');(out/'editor.tsx').write_text(editor)
 run=(out/'run.mjs').read_text();run=re.sub(r'const privateRoutes=new Map\(Object.entries\(.*?\)\);','const privateRoutes=new Map(Object.entries('+json.dumps(routes)+'));',run,count=1)
 run=re.sub(r'/\^b05-.*?\$/.test\(p.stage.name\)',r'/^york-gate-ui-[a-z0-9-]+$/.test(p.stage.name)',run)
 (out/'run.mjs').write_text(run)
 pins={str(p):sha(p) for p in [stage/'handoff.json',oldconfig,LIB/'scenes/york.rhlos-map.json',LIB/'mission-states/index.json',LIB/'3d-assets/index.json',*map(Path,routes.values())]}
 def resources(value):
  if isinstance(value,dict):
   if 'path' in value and 'sha256' in value:
    p=Path(value['path']);p=p if p.is_absolute() else Path(routes.get('/library/'+str(p),str(LIB/p)));assert sha(p)==value['sha256'],str(p);pins[str(p)]=value['sha256']
   for k,v in value.items():
    if k=='model' and isinstance(v,str) and 'model_sha256' in value:
     p=Path(routes.get('/library/'+v,str(LIB/v)));assert sha(p)==value['model_sha256'];pins[str(p)]=value['model_sha256']
    resources(v)
  elif isinstance(value,list):
   for v in value:resources(v)
 resources(handoff['pins']);resources(entry);resources(json.loads(Path(handoff['contract']['path']).read_text()))
 for ref in [*bounded['assetSources'],*bounded['sceneAssets']]:
  for k in ['model','descriptor']:
   p=LIB/ref[k];assert sha(p)==ref[k+'_sha256'];pins[str(p)]=ref[k+'_sha256']
  resources(json.loads((LIB/ref['descriptor']).read_text()))
 dump(out/'receiver-scope.json',{'scope':'Bounded gatehouse9parts plus scene terrain; controlled Night patch000 only; no startup script execution','asset_ids':list(selected),'scene_assets':[s['id']for s in bounded['sceneAssets']],'canonical_sha256':sha(LIB/'scenes/york.rhlos-map.json')})
 snap=json.loads((out/'source-snapshot.json').read_text());dump(out/'runtime-inputs.json',{**snap,**pins})
 pins.update({str(p):sha(p) for p in out.iterdir() if p.is_file() and p.name not in ['inputs.json','request.json']});dump(out/'inputs.json',pins)
 dump(out/'request.json',{'status':'PREPARED_NOT_LAUNCHED','run':str(out/'run.mjs'),'sha256':sha(out/'run.mjs'),'source_contract':handoff['contract'],'scope':'Night patch00045poses, terminal89, source-camera first, native/physical mode comparison, reset and retirement/rebind. Exact approved models; startup9/7/8 excluded.'});print(sha(out/'run.mjs'))
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--out',type=Path,required=True);a=p.parse_args();main(a.out.resolve())
