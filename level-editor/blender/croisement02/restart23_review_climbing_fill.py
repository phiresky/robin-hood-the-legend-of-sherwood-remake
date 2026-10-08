"""Reopen private climbing texture bakes for exact native and complete view checks."""
import sys,json,shutil
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from render_slots import acquire,release
BASE=OUT/'restart14-hidden-archer/climbing-texture-bake-v1';CAP=32*1024**2

def budget(*unused):
 used=sum(p.stat().st_size for p in BASE.rglob('*') if p.is_file())
 assert used<CAP and shutil.disk_usage(BASE).free>=10*1024**3+CAP-used

def main():
 budget()
 assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024>=6*1024**3
 for state in ['initial','applied']:
  record=json.loads((BASE/f'profile-05-{state}/bake-validation.json').read_text())
  assert record['observed_material_atlases_exact'] and record['original_uv_layer_exact']
 import restart14_hidden_archer_review_v12 as review
 review.small_budget=budget;review.main(BASE)
 import restart17_hidden_archer_contact as contact
 contact.ROUND=BASE;contact.DEST=BASE/'rock-bank-contact-v1';contact.budget=budget;contact.main()
 budget()
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
