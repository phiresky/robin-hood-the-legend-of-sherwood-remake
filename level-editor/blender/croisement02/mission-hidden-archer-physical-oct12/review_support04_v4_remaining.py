"""Resume uncompleted saved poses and wood junctions into fresh evidence."""
from pathlib import Path
original=Path(__file__).with_name('review_support04_v3_junctions.py')
source=original.read_text()
needle="source=source.replace(needle,extra+needle)"
assert source.count(needle)==1
source=source.replace(needle,needle+"\nsource=source.replace(\"for phase in ['initial','transition-0','transition-1','applied']:\",\"for phase in ['transition-1','applied']:\").replace('geometry-v8-short-support04-review-v3-junctions','geometry-v8-short-support04-review-v4-remaining')")
exec(compile(source,__file__,'exec'),{'__name__':'__main__','__file__':__file__})
