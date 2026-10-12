"""Review the saved supplemental wood support revision in all native poses."""
from pathlib import Path
original=Path(__file__).with_name('review_support04_v1.py')
source=original.read_text().replace('geometry-v8-short-support04-v3','geometry-v8-short-support04-v4').replace('geometry-v8-short-support04-review-v1','geometry-v8-short-support04-review-v2')
exec(compile(source,str(original),'exec'),{'__name__':'__main__','__file__':str(original)})
