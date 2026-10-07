"""Collect completed contact evidence without loading or rewriting scene data."""
from pathlib import Path
import json
from PIL import Image,ImageDraw
from mound_contact_budget import digest,check,png_bound,MIB

def main():
 root=Path(__file__).resolve().parents[2]/'work/croisement02-refinement/restart15-hiding-mounds/all-placements-v1'
 total=root/'contacts-v3';review=total/'review-pages';review.mkdir(exist_ok=True);rows=[]
 for i in range(20):
  tag=f'site-{i:02}';completed=[p.parent for p in (total/tag).glob('attempt-*/resource-receipt.json')if json.loads(p.read_text())['status']=='PASS'and len(json.loads((p.parent/'render-receipt.json').read_text())['images'])==3];assert len(completed)==1,f'Expected one completed attempt for {tag}';folder=completed[0];resource=json.loads((folder/'resource-receipt.json').read_text());render=json.loads((folder/'render-receipt.json').read_text())
  assert resource['status']=='PASS'and resource['exit_code']==0 and not resource['source_drift']
  assert all(digest(folder/n)==h for n,h in resource['outputs'].items())
  assert resource['model_writes']==resource['scene_snapshot_writes']==resource['temporary_scene_writes']==0
  rows.append(dict(site=tag,directory=str(folder.relative_to(total)),resource_sha256=digest(folder/'resource-receipt.json'),render_sha256=digest(folder/'render-receipt.json'),sheet_sha256=digest(folder/'contact-three.png'),aliases=render['aliases'],peak_rss_bytes=resource['peak_rss_bytes']))
 pages=[]
 for start in range(0,20,4):
  existing=review/f'page-{start//4}.png'
  if not existing.exists():existing=review/f'page-{start//4}'/'sheet.png'
  if existing.exists():pages.append(dict(path=str(existing.relative_to(total)),sha256=digest(existing)));continue
  page_dir=review/f'page-{start//4}';page_dir.mkdir(exist_ok=True);check(page_dir,total,4*MIB);assert png_bound(960,1280,3)<4*MIB
  canvas=Image.new('RGB',(960,1280),(35,35,35));draw=ImageDraw.Draw(canvas)
  for j,row in enumerate(rows[start:start+4]):
   source=Image.open(total/row['directory']/'contact-three.png').convert('RGB').resize((960,320),Image.Resampling.LANCZOS);canvas.paste(source,(0,j*320));draw.rectangle((0,j*320,205,j*320+19),fill=(20,20,20));draw.text((5,j*320+3),row['site']+' | native / side / low',fill=(255,255,255))
  dest=page_dir/'sheet.png';assert not dest.exists();canvas.save(dest);check(page_dir,total);pages.append(dict(path=str(dest.relative_to(total)),sha256=digest(dest)))
 page_dir=total/'report-budget';page_dir.mkdir(exist_ok=True)
 report=dict(status='COMPLETE_CONTACT_RENDERS_PENDING_VISUAL_REVIEW',model_sha256=digest(root/'model.blend'),sites=rows,pages=pages,scope='Ground/bank/wall-only contact views. Original camera first. No model changes or appearance approval.');check(page_dir,total,128*1024);(total/'report.json').write_text(json.dumps(report,indent=2)+'\n')

if __name__=='__main__':main()
