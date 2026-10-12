"""Add measured short main-transition support and a second initial wood anchor."""
from pathlib import Path
original=Path(__file__).with_name('build_support04_v3.py')
source=original.read_text()
source=source.replace("source=original.read_text().replace(\"geometry-v8-short-support04'\",\"geometry-v8-short-support04-v3'\")", "source=original.read_text().replace(\"geometry-v8-short-support04'\",\"geometry-v8-short-support04-v4'\").replace('support-route-probe-v1/all-pose-attachment-candidates-v2.json','support04-missing-routes-v1/merged-candidates.json').replace('available()<14*1024**3','available()<11*1024**3').replace('Waiting14GiB','Waiting11GiB')")
exec(compile(source,str(original),'exec'),{'__name__':'__main__','__file__':str(original)})
