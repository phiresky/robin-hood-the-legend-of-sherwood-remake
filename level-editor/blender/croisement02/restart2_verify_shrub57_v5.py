"""Independent reopened checks for the private rigid-pair shrub candidate."""
import sys,argparse
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
from restart2_review_sign_shrub57 import main as joint
from restart2_check_sign_bend_sources import main as source
from restart2_check_sign_bend_ground_parity import main as ground
from render_slots import acquire,release
if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--check',required=True,choices=['joint','source','ground'])
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    acquire()
    try:
        if args.check=='joint':joint(version=5)
        elif args.check=='ground':ground(version=5)
        else:source(versions=(5,),output_name='shrub57-bend-source-audit-v5')
    finally:release()
