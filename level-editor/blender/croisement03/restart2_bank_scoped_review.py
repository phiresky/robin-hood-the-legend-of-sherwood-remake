"""Run one saved-bank review phase under the bounded shared resource lease."""
import sys,argparse,importlib,shutil
from pathlib import Path
R=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(R/'level-editor/refinement')]
from render_slots import acquire,release

def main():
 ap=argparse.ArgumentParser();ap.add_argument('folder',type=Path);ap.add_argument('phase',choices=['native','audit','neighbors']);args=ap.parse_args(sys.argv[sys.argv.index('--')+1:]);assert (args.folder/'worker.blend').exists();assert shutil.disk_usage(R).free>10*1024**3
 mem={r.split(':')[0]:int(r.split()[1])*1024 for r in Path('/proc/meminfo').read_text().splitlines()};assert mem['MemAvailable']>=6*1024**3;acquire(slots=2)
 try:
  assert shutil.disk_usage(R).free>10*1024**3
  mem={r.split(':')[0]:int(r.split()[1])*1024 for r in Path('/proc/meminfo').read_text().splitlines()};assert mem['MemAvailable']>=6*1024**3
  sys.argv=[sys.argv[0],'--',str(args.folder.resolve())];module=importlib.import_module({'native':'restart2_bank_native_contact','audit':'restart2_bank_saved_audit','neighbors':'restart2_bank_neighbor_guard'}[args.phase]);module.main();assert sum(p.stat().st_size for p in args.folder.rglob('*')if p.is_file())<=32*1024**2
 finally:release()
if __name__=='__main__':main()
