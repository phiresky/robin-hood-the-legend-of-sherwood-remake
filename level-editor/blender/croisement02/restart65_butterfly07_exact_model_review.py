"""Compare exact observed RGB selection on the unchanged private butterfly rig."""
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import restart64_butterfly07_model_review as review
review.ATLAS=review.ROOT/'level-editor/work/active/croisement02/restart65-butterfly07-exact-observation-atlas-v1'
review.OUT=review.ROOT/'level-editor/work/active/croisement02/restart65-butterfly07-exact-model-review-v1'
if __name__=='__main__':
 review.acquire(2)
 try:review.main()
 finally:review.release()
