"""Freeze fresh player bundles without replacing any candidate or older evidence."""
from pathlib import Path
import hashlib,json,shutil,subprocess,argparse
R=Path.cwd();B=R/'level-editor/work/croisement02-refinement';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
parser=argparse.ArgumentParser();parser.add_argument('--player-commit',required=True);args=parser.parse_args();player=R/'level-editor/app/src/state-appearance-player.ts';expected=subprocess.check_output(['git','show',args.player_commit+':level-editor/app/src/state-appearance-player.ts']);assert hashlib.sha256(expected).hexdigest()==sha(player),'Player changed after reviewed fix'
def pins_in(value):
 if isinstance(value,dict):
  if isinstance(value.get('path'),str)and isinstance(value.get('sha256'),str):yield value
  for v in value.values():yield from pins_in(v)
 elif isinstance(value,list):
  for v in value:yield from pins_in(v)
for oldname,newname in [('canopy04-aligned-junction-supplement-oct12-v1','canopy04-aligned-junction-fractionalfix-oct12-v1'),('canopy05-complete-calibrated-playerfix-oct12-v1','canopy05-complete-calibrated-fractionalfix-oct12-v1'),('canopy07-complete-calibrated-playerfix-oct12-v1','canopy07-complete-calibrated-fractionalfix-oct12-v1')]:
 old=B/oldname;out=B/newname;out.mkdir(exist_ok=False);request=json.loads((old/'request.json').read_text())
 for pin in pins_in(request):assert sha(Path(pin['path']))==pin['sha256'],pin['path']
 shutil.copy2(old/'request.json',out/'request.json')
 for name in ['run.mjs','build.mjs','proof.ts']:(out/name).write_text((old/name).read_text().replace(oldname,newname))
 subprocess.run(['node',str(out/'build.mjs')],check=True);subprocess.run(['node','--check',str(out/'run.mjs')],check=True)
 pins=json.loads((out/'source-pins.json').read_text());snapshot={}
 for path,digest in list(pins.items()):
  source=Path(path);assert sha(source)==digest;relative=source.relative_to(R)if source.is_absolute()else source;dest=out/'runtime-snapshot'/relative;dest.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(source,dest);snapshot[str(dest)]=digest
 for pin in pins_in(request):pins[pin['path']]=pin['sha256']
 for name in ['request.json','proof.ts','proof.js','index.html','run.mjs','build.mjs']:pins[str(out/name)]=sha(out/name)
 pins.update(snapshot);pins[str(old/'request.json')]=sha(old/'request.json');pins[str(player)]=sha(player)
 (out/'source-pins.json').write_text(json.dumps(pins,indent=2)+'\n')
 handoff=json.loads((old/'handoff.json').read_text());handoff['status']='PREPARED_NEW_PLAYER_BUNDLE_NOT_LAUNCHED';handoff['runner']={'path':str(out/'run.mjs'),'sha256':sha(out/'run.mjs')};handoff.pop('run',None);handoff['pins']={'path':str(out/'source-pins.json'),'sha256':sha(out/'source-pins.json')};handoff.pop('pins_sha256',None);handoff['previous_request']={'path':str(old/'request.json'),'sha256':sha(old/'request.json')};handoff['player_fix']={'commit':args.player_commit,'path':str(player),'sha256':sha(player)};handoff['candidate_identity']='Request bytes identical to older packet; all model/source/view pins verified. New compiled bundle and copied runtime inputs, not relabeled old proof.';(out/'handoff.json').write_text(json.dumps(handoff,indent=2)+'\n');print(newname,sha(out/'handoff.json'),len(pins),flush=True)
