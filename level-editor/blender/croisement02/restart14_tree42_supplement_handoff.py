"""Freeze bidirectional neighbor evidence without changing the motion trial."""
import json,hashlib,shutil
from collections import Counter
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.ndimage import distance_transform_edt
from restart14_tree42_v5_supplement import BASE,TRIAL,DEST,guard,sha

def main():
 old=json.loads((DEST/'reverse-parent/report.json').read_text());new=json.loads((DEST/'reverse-neighbors/report.json').read_text());forward=json.loads((DEST/'neighbors/report.json').read_text());source=json.loads((BASE/'source-reconciliation-v1/report.json').read_text())['groups'][1];assert old['baseline_counts']==new['baseline_counts'];rows=[]
 for a,b in zip(old['phases'],new['phases']):
  key=lambda q:(*q['pixel'],q['neighbor']);prior={key(q):q for q in a['new_crown_in_front']};current={key(q):q for q in b['new_crown_in_front']};f=source['frames'][b['phase']];x,y,w,h=f['bbox'];im=Image.new('RGBA',(342,288));im.paste(Image.open(f['path']),(x-616,y-688));alpha=np.array(im)[:,:,3]>=128;dist=distance_transform_edt(~alpha);added=[]
  for k in sorted(current.keys()-prior.keys()):
   q=dict(current[k]);sx,sy=q['pixel'];q['own_source_alpha_distance']=float(dist[sy-688,sx-616]);q['own_source_exact_alpha']=bool(alpha[sy-688,sx-616]);added.append(q)
  rows.append({'phase':b['phase'],'v4_covers_baseline_neighbors':len(prior),'v5_covers_baseline_neighbors':len(current),'introduced':added,'resolved':[prior[k]for k in sorted(prior.keys()-current.keys())]})
 report={'status':'MEASURED_FOR_ROOT_REVIEW_NOT_ACCEPTANCE','baseline_neighbor_counts':new['baseline_counts'],'phases':rows,'introduced_total_phase_events':sum(len(r['introduced'])for r in rows),'resolved_total_phase_events':sum(len(r['resolved'])for r in rows),'forward_valid_tree_hits_blocked_by_neighbors':[sum(q['tree42_hit']for q in p['foreign_first_hits'])for p in forward['phases']],'scope':['Baseline targets are physical nearest fence/shrub hits in full342x288 native crown rectangle, not a claim every target is observed neighbor artwork.','Native source-alpha distance labels checker sampling support; it is not permission to move neighbors or regenerate textures.','Neighbor source geometry, placements, model/resource hashes fixed to prior validated audit; no neighboring models saved.','V4/v5 reversal audit uses actual saved shape-key poses and independent alpha-aware per-asset first hits.']}
 guard(262144);path=DEST/'neighbor-comparison.json';assert not path.exists();path.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'introduced':report['introduced_total_phase_events'],'resolved':report['resolved_total_phase_events'],'rows':[(r['phase'],r['v4_covers_baseline_neighbors'],r['v5_covers_baseline_neighbors'],len(r['introduced']),len(r['resolved']))for r in rows]},indent=2))
if __name__=='__main__':main()
