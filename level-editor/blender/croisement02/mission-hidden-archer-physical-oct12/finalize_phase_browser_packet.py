"""Freeze root-only actual material capture inputs after exact camera recovery."""
from pathlib import Path
import json,hashlib,shutil,subprocess,sys
ROOT=Path(__file__).resolve().parents[4];BASE=ROOT/'level-editor/work/active/croisement02/mission-hidden-archer-physical-oct12-v1/approved03-04-textures-v1/private-phase-integration-v1';P=BASE/'browser-packet-v1'
def main():
 plan=json.loads((P/'plan.json').read_text());assert len(plan['rows'])==8
 for source,target in [('browser-prepared.mjs','browser.mjs'),('admit-prepared.py','admit.py'),('assemble-prepared.py','assemble.py'),('exclusive-pool-prepared.py','exclusive_pool.py')]:shutil.copy2(BASE/source,P/target)
 pins={}
 def pin(p):
  p=Path(p).absolute();pins[str(p)]=hashlib.sha256(p.read_bytes()).hexdigest()
 for file in ['plan.json','browser.mjs','admit.py','assemble.py','exclusive_pool.py']:pin(P/file)
 for row in plan['rows']:
  assert len(row['views'])==8 and row['views'][0]['index']==0
  for key in ['model','approved_source']:
   r=row[key];assert hashlib.sha256(Path(r['path']).read_bytes()).hexdigest()==r['sha256'];pin(r['path'])
  pin(row['native_source']);pin(row['approved_review']);review=json.loads(Path(row['approved_review']).read_text())
  for image,h in review['images'].items():
   path=Path(image);assert hashlib.sha256(path.read_bytes()).hexdigest()==h;pin(path)
 for file in ['phase-integration-delta.json','mission-contract-delta-v2.json','exports-v1/summary.json']:pin(BASE/file)
 approval=json.loads((BASE/'phase-integration-delta.json').read_text())['approval_authority'];assert hashlib.sha256(Path(approval['path']).read_bytes()).hexdigest()==approval['sha256'];pin(approval['path'])
 for file in ['refinement/render_slots.py','app/src/texture-display.ts','app/tests/cdp.mjs','pnpm-lock.yaml']:pin(ROOT/'level-editor'/file)
 paths={}
 for module in ['three','meshoptimizer']:
  p=ROOT/'level-editor/app/node_modules'/module;paths[str(p)]=str(p.resolve())
  for file in p.resolve().rglob('*'):
   if file.is_file()and file.suffix in ['.js','.mjs','.json','.wasm']:pin(file)
 (P/'module-paths.json').write_text(json.dumps(paths,indent=2)+'\n');pin(P/'module-paths.json');(P/'pins.json').write_text(json.dumps(pins,indent=2)+'\n');subprocess.run([sys.executable,str(P/'admit.py')],check=True,cwd=ROOT)
 (P/'handoff.json').write_text(json.dumps({'status':'ROOT_ONLY_CAPTURE_CPU_VERIFIED_NOT_LAUNCHED','command':['python',str(P/'admit.py'),'--run'],'model_count':8,'views':64,'native_comparisons':8,'source_camera_first':True,'approved_model_geometry_and_source_images_unchanged':True,'shared_render_slots_reserved':4,'browser_lock':True,'pins':len(pins),'canonical_writes':False,'outputs':str(P/'captures'),'scope':'03/04approvedphaseappearance qualification independent of01/02; finalmissionintegration still separate.'},indent=2)+'\n');print(P)
if __name__=='__main__':main()
