"""Build the bounded curved bank using the full saved-model construction guards."""
import sys,json,hashlib,shutil,argparse
from pathlib import Path
R=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(R/'level-editor/refinement')]
from render_slots import acquire,release
import restart2_bank_whole_builder as whole
import restart2_bank_full_v1 as base
B=R/'level-editor/work/active/croisement03/restart2';CPU=B/'bank-curved-body-cpu-v1';OUT=B/'bank-curved-body-saved-v1'
def resources():
 assert shutil.disk_usage(R).free>=10*1024**3+32*1024**2
 mem={r.split(':')[0]:int(r.split()[1])*1024 for r in Path('/proc/meminfo').read_text().splitlines()};assert mem['MemAvailable']>=6*1024**3

def main():
 global CPU,OUT
 ap=argparse.ArgumentParser();ap.add_argument('--input',type=Path,default=CPU);ap.add_argument('--output',type=Path,default=OUT);a=ap.parse_args(sys.argv[sys.argv.index('--')+1:]if '--'in sys.argv else []);CPU=a.input.resolve();OUT=a.output.resolve()
 proof=json.loads((CPU/'source-firsthit-guard.json').read_text());assert proof['status']=='PASS_CPU_SIDEWALL_ANCHORS_AND_BANK_SEEDS';assert proof['geometry_sha256']==hashlib.sha256((CPU/'geometry.json').read_bytes()).hexdigest();assert not OUT.exists()
 resources();acquire(slots=2)
 try:
  resources();whole.O=CPU;original=base.main
  def scoped_main():
   base.O=OUT
   original()
  base.main=scoped_main;whole.run()
 finally:release()
if __name__=='__main__':main()
