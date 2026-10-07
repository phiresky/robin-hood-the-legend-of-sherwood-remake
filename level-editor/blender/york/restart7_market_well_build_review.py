"""One bounded render lease for the fitted well build and reopened review."""
import sys,importlib.util
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
def module(name):
 p=Path(__file__).with_name(name+'.py');spec=importlib.util.spec_from_file_location(name,p);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m
if __name__=='__main__':
 acquire()
 try:
  build=module('restart7_market_well_fitted_candidate');build.main();review=module('restart7_market_well_review');assert review.D==build.D;review.main()
 finally:release()
