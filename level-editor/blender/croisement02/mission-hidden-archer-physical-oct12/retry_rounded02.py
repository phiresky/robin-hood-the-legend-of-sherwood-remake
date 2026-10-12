"""Retry the unchanged queued recipe after pre-admission termination."""
from pathlib import Path
import hashlib
ROOT=Path(__file__).resolve().parents[4]
SNAPSHOT=ROOT/'level-editor/work/croisement02-refinement/mission-hidden-archer-physical-oct12-v1/rounded02-build-recipe.py'
EXPECTED='f7807da6e019713d71b013b351ef5b1de65d25d910ffac1b3a792dcc5e0e0c6b'
source=SNAPSHOT.read_bytes()
assert hashlib.sha256(source).hexdigest()==EXPECTED
original=Path(__file__).with_name('build_models.py')
exec(compile(source,str(original),'exec'),{'__name__':'__main__','__file__':str(original)})
