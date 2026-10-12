"""Audit supplemental short wood attachments using the conservative finite sweep."""
from pathlib import Path
original=Path(__file__).with_name('support04_sweep_v1.py')
source=original.read_text().replace('geometry-v8-short-support04/report.json','geometry-v8-short-support04-v4/report.json').replace("geometry-v8-support04-sweep'","geometry-v8-support04-sweep-v4'")
exec(compile(source,str(original),'exec'),{'__name__':'__main__','__file__':str(original)})
