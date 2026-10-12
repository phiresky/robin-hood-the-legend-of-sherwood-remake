"""Bake/review the one explicitly approved-input appearance retry in its own scope."""
import sys,time,shutil
from pathlib import Path
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
import bake_approved03_04_textures_v1 as bake
import review_approved03_04_textures_v1 as review
from render_slots import acquire,release
mode=sys.argv[sys.argv.index('--')+1];assert mode in ['bake','review']
base=bake.BASE/'retry03-transition-1-v1';exp=base/'experiment';dest=base/'baked'
if mode=='bake':bake.preflight('03-transition-1',exp)
while True:
 while bake.available()<11*2**30:print('Waiting11GiB for archer retry',flush=True);time.sleep(30)
 assert shutil.disk_usage(base).free>=10*2**30
 acquire()
 try:
  if bake.available()<10*2**30:continue
  if mode=='bake':bake.main('03-transition-1',experiment=exp,destination=dest)
  else:review.main('03-transition-1',folder=dest)
  break
 finally:release()
