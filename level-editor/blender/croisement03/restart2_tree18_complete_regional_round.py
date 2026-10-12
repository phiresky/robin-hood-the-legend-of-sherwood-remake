"""One bounded lease for corrected junction and complete regional saved context."""
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import restart2_tree18_five_track_saved as wood
import restart2_tree18_regional_joined_saved as regional
import render_slots


def main():
    regional.guard.resources();render_slots.acquire(slots=2);originals=[]
    try:
        regional.guard.resources();seen=set()
        for module in [wood.base.guard,wood.review.guard,regional.guard]:
            if id(module)not in seen:seen.add(id(module));originals.append((module,module.release));module.release=lambda:False
        wood.main();regional.main()
        folders=[wood.B/'tree18-five-track-saved-v2',regional.OUT]
        total=sum(p.stat().st_size for folder in folders for p in folder.rglob('*')if p.is_file());assert total<=32*1024**2;print({'complete_regional_round_bytes':total})
    finally:
        for module,function in originals:module.release=function
        render_slots.release()


if __name__=='__main__':main()
