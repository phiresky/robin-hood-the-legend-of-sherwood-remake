"""Review the retained cap appearance across the complete unchanged approved geometry."""
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
from restart7_wall101_retained_appearance import E
import restart7_wall101_full_review as review
from render_slots import acquire,release
if __name__=='__main__':
 acquire()
 try:
  review.D=E
  review.main()
 finally:release()
