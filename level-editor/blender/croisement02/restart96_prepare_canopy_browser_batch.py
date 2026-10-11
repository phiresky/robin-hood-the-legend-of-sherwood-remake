"""Prepare a pinned private browser batch from audited crown motion exports."""
import argparse,json,hashlib,subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();pin=lambda p:dict(path=str(p),sha256=sha(p))
def main():
 p=argparse.ArgumentParser();p.add_argument('--exports',type=Path,nargs='+',required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(exist_ok=False);rows=[]
 for ep in a.exports:
  d=json.loads(ep.read_text());audit=ep.parent.with_name(ep.parent.name.replace('tapered-motion','triangle-audit'))/'report.json';assert json.loads(audit.read_text())['issues']==0
  for asset in d['assets']:
   aid=asset['asset_id'];number=int(aid[-2:]);folders=['restart2-wood/combined-crown-v3','restart2-wood/combined-crown-v2','forest-v4-round-3','forest-v4-round-2','forest-v4-round-1'];views=next((BASE/f'{f}/assets/{aid}/modified/views.json' for f in folders if (BASE/f'{f}/assets/{aid}/modified/views.json').is_file()),None);assert views,aid
   matrix=json.loads(views.read_text())['views'][0]['camera_matrix_world'];assert abs(matrix[0][2])<1e-4 and matrix[1][2]<-.8 and matrix[2][2]>.57,aid
   rows.append(dict(asset_id=aid,primitive_count=len(asset['primitives']),model=pin(Path(asset['model']['path'])),placement=asset['placement']['transform'],views=pin(views),export=pin(ep),triangle_audit=pin(audit)))
 assert len({r['asset_id'] for r in rows})==len(rows)
 (a.output/'request.json').write_text(json.dumps(dict(status='PRIVATE_UNAPPROVED_CROWN_MOTION',rows=rows),indent=2)+'\n');template=BASE/'restart91-ten-canopy-browser-v1'
 for name in ['build.mjs','run.mjs','proof.ts']:(a.output/name).write_text((template/name).read_text().replace(template.name,a.output.name))
 subprocess.run(['node',str(a.output/'build.mjs')],cwd=ROOT,check=True);subprocess.run(['node','--check',str(a.output/'run.mjs')],cwd=ROOT,check=True);pins=json.loads((a.output/'source-pins.json').read_text())
 for name in ['request.json','proof.ts','proof.js','index.html','run.mjs','build.mjs']:pins[str(a.output/name)]=sha(a.output/name)
 for r in rows:
  for key in ['model','views','export','triangle_audit']:pins[r[key]['path']]=r[key]['sha256']
 (a.output/'source-pins.json').write_text(json.dumps(pins,indent=2)+'\n');print(len(rows),pin(a.output/'run.mjs'))
if __name__=='__main__':main()
