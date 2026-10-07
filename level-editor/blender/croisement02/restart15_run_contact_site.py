"""Run one authorized contact site with measured memory and persistent-output limits."""
from pathlib import Path
import sys,json,os,time,subprocess,resource
import psutil
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from mound_contact_budget import digest,allocated,check,MIB,GIB

def main():
 tag=sys.argv[1];worker=P.parents[1]/'work/croisement02-refinement/restart15-hiding-mounds/all-placements-v1';permission=Path(sys.argv[3])if len(sys.argv)>3 else worker/'contacts-v3-output-authorization.json';grant=json.loads(permission.read_text());probe=json.loads((worker/'startup-cap-comparison-v1/receipt.json').read_text());by_cap={r['fsize_cap']:r['exit_code']for r in probe['records']};assert by_cap=={True:-25,False:0},'Successful bounded startup comparison required';assert tag in grant['allowed_sites'];assert grant['limits']==dict(site_mib=8,total_mib=160,file_mib=4,free_floor_gib=10,address_space_gib=12,rss_gib=8);total=worker/'contacts-v3';total.mkdir(exist_ok=True);site_root=total/tag;site_root.mkdir(exist_ok=True);attempt=sys.argv[2]if len(sys.argv)>2 else'attempt-02';assert attempt.startswith('attempt-')and attempt[8:].isdigit();out=site_root/attempt
 if(out/'resource-receipt.json').exists():
  prior=json.loads((out/'resource-receipt.json').read_text());assert prior['status']=='PASS'and all(digest(out/n)==h for n,h in prior['outputs'].items());assert all(digest(Path(n))==h for n,h in prior['input_hashes'].items());print('Verified completed site',tag);return
 if out.exists():raise RuntimeError('Partial site exists: preserve it and obtain a fresh explicit attempt path before retry')
 out.mkdir();check(site_root,total);recipe=P/'restart15_contact_site.py';before={str(p):digest(p)for p in [recipe,P/'mound_contact_budget.py',permission,worker/'model.blend',worker/'validation.json']};started=time.monotonic();peak=0;reason=None
 def limits():
  resource.setrlimit(resource.RLIMIT_AS,(12*GIB,12*GIB));os.setsid()
 env=os.environ.copy();env.update(OMP_NUM_THREADS='2',OPENBLAS_NUM_THREADS='2',MESA_SHADER_CACHE_DISABLE='true',CUDA_CACHE_DISABLE='1')
 command=['/usr/bin/blender','--background','--threads','2','--python-exit-code','1','--python',str(recipe),'--',tag,str(out),str(permission)]
 with(out/'process.log').open('wb')as log:
  child=subprocess.Popen(command,stdout=log,stderr=subprocess.STDOUT,env=env,preexec_fn=limits);process=psutil.Process(child.pid)
  while child.poll()is None:
   try:
    peak=max(peak,process.memory_info().rss);check(site_root,total)
    if peak>8*GIB:raise RuntimeError('8GiB RSS watchdog reached')
    if time.monotonic()-started>1800:raise TimeoutError('30minute site deadline reached')
   except (RuntimeError,TimeoutError)as error:
    reason=str(error);child.terminate()
    try:child.wait(timeout=10)
    except subprocess.TimeoutExpired:child.kill();child.wait()
    break
   except psutil.NoSuchProcess:break
   time.sleep(.2)
  code=child.wait()
 peak=max(peak,int(resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss)*1024);drift=[p for p,h in before.items()if digest(Path(p))!=h];receipt=out/'render-receipt.json';passed=code==0 and not reason and not drift and receipt.exists()and peak<=8*GIB
 outputs={p.name:digest(p)for p in out.iterdir()if p.is_file()};record=dict(status='PASS'if passed else'FAILED',site=tag,exit_code=code,stop_reason=reason,source_drift=drift,peak_rss_bytes=peak,file_limit_enforcement='Explicit root-authorized raster bound plus persistent-output watchdog; no global FSIZE',address_space_limit_bytes=12*GIB,rss_watchdog_bytes=8*GIB,elapsed_seconds=time.monotonic()-started,allocated_output_bytes_before_receipt=allocated(out),authorization_sha256=digest(permission),input_hashes=before,outputs=outputs,model_writes=0,scene_snapshot_writes=0,temporary_scene_writes=0,scope='Resource and render completion only; visual review required before another site.');check(site_root,total,128*1024);(out/'resource-receipt.json').write_text(json.dumps(record,indent=2)+'\n');print(json.dumps(record,indent=2));assert passed,'Site failed; no automatic retry or raised limit'
if __name__=='__main__':main()
