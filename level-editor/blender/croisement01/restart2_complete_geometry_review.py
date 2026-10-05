"""Run saved native/topology evidence and ground contacts in separate FIFO leases."""
import argparse
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).parent))
import restart2_tree_audit
import render_terrain_contact
p=argparse.ArgumentParser();p.add_argument('worker');p.add_argument('--mask',required=True,type=int);a=p.parse_args(sys.argv[sys.argv.index('--')+1:])
sys.argv=['audit','--',a.worker,'--mask',str(a.mask)];restart2_tree_audit.main()
sys.argv=['contact','--',a.worker];render_terrain_contact.main()
