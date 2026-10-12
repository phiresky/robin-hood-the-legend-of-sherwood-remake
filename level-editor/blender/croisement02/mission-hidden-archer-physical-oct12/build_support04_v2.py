"""Recover support construction with explicit source-projection diagnostics."""
from pathlib import Path
original=Path(__file__).with_name('build_support04_v1.py')
source=original.read_text().replace("geometry-v8-short-support04'","geometry-v8-short-support04-v2'")
source=source.replace("assert abs(p.x-sx)<.21 and abs(-p.y*build.S-p.z*build.C-sy)<.21", "assert abs(p.x-sx)<.21 and abs(-p.y*build.S-p.z*build.C-sy)<.21, (phase, component['component'], sx, sy, list(p), list(start), list(end), list(stem.rotation_euler), list(stem.scale), -p.y*build.S-p.z*build.C-sy)")
exec(compile(source,str(original),'exec'),{'__name__':'__main__','__file__':str(original)})
