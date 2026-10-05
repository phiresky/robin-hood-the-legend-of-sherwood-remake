"""Source first-hit validation for the subdivided rigid-pair candidate."""
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
from restart2_raster_shrub57_sources import main
from render_slots import acquire,release
if __name__=='__main__':
    acquire()
    try:main(versions=(6,),name='shrub57-source-raster-v2')
    finally:release()
