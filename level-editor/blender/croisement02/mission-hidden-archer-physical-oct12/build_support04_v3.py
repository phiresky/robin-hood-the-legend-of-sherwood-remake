"""Evaluate support transforms while visible before applying native state hiding."""
from pathlib import Path
original=Path(__file__).with_name('build_support04_v1.py')
source=original.read_text().replace("geometry-v8-short-support04'","geometry-v8-short-support04-v3'")
needle=";stem.hide_render=phase!='initial';stem.hide_viewport=phase!='initial'"
assert source.count(needle)==1
source=source.replace(needle,'')
needle2="            rows.append(dict(phase=phase,component=component['component'],pixels=component['pixels'],\n                status='BUILT"
assert source.count(needle2)==1
source=source.replace(needle2,"            stem.hide_render=phase!='initial';stem.hide_viewport=phase!='initial'\n"+needle2)
exec(compile(source,str(original),'exec'),{'__name__':'__main__','__file__':str(original)})
