"""Separate exact bank/post contact occlusion from remaining leaf occlusion."""
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
from restart2_review_sign_shrub57 import main
from render_slots import acquire,release
if __name__=='__main__':
    acquire()
    try:main(version=6,proof_name='bank-only-proof',include_keys=['north-woodland-bank','west-rock-outcrop','southwest-rock-outcrop'],obliques=False)
    finally:release()
