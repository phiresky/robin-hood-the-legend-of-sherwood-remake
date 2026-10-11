"""Report every same-compiler canonical/staged difference without filtering fields."""
import argparse,collections,gzip,hashlib,json
from pathlib import Path
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement/restart2/compiler-rebase-diagnosis-v2'
def main():
 ap=argparse.ArgumentParser();ap.add_argument('--candidate',type=Path,default=B/'candidate.json');args=ap.parse_args();a=json.loads((B/'canonical.json').read_text());b=json.loads(args.candidate.read_text());diff=[]
 def compare(x,y,path=''):
  if isinstance(x,dict)and isinstance(y,dict):
   for k in sorted(x.keys()|y.keys()):
    p=path+'/'+k
    if k not in x or k not in y:diff.append({'path':p,'before':x.get(k),'after':y.get(k),'missing':'before'if k not in x else'after'})
    else:compare(x[k],y[k],p)
  elif isinstance(x,list)and isinstance(y,list):
   if len(x)!=len(y):diff.append({'path':path+'/length','before':len(x),'after':len(y)})
   for i in range(min(len(x),len(y))):compare(x[i],y[i],path+'/'+str(i))
  elif x!=y:diff.append({'path':path,'before':x,'after':y})
 compare(a,b);raw=json.dumps(diff,separators=(',',':')).encode();(B/'canonical-candidate-all-differences.json.gz').write_bytes(gzip.compress(raw,mtime=0));counts=collections.Counter('/'.join(d['path'].split('/')[:3])for d in diff)
 report={'status':'DIFFERENCES_REPORTED_NOT_IGNORED','canonical_sha256':hashlib.sha256((B/'canonical.json').read_bytes()).hexdigest(),'candidate_sha256':hashlib.sha256(args.candidate.read_bytes()).hexdigest(),'total_differences':len(diff),'by_top_level_field':dict(counts),'doors_exact':a['geometry']['doors']==b['geometry']['doors'],'passage_states_canonical':[d.get('passage_states')for d in a['geometry']['doors']],'passage_states_candidate':[d.get('passage_states')for d in b['geometry']['doors']],'complete_diff':'canonical-candidate-all-differences.json.gz','examples':diff[:30],'limitations':['Canonical draft duplicates were deduplicated only for the exact two approved descriptor repairs in diagnostic compile.','This report does not certify every conversion delta. Full candidate equality against the current-compiler reviewed stage is a separate mandatory guard.']}
 (B/'canonical-candidate-comparison.json').write_text(json.dumps(report,indent=2)+'\n');print({k:v for k,v in report.items()if k not in ['examples']})
if __name__=='__main__':main()
