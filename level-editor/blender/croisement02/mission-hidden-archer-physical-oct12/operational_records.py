"""Read the explicitly relocated operational copy of a retained archer record."""
import hashlib,json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[4]
PACKET=ROOT/'level-editor/work/storage-maintenance/migration-20261012/croisement02/archers/operational.json'
def sha(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def read_record(path):
 path=Path(path);index=json.loads(PACKET.read_text());row=index['records'][str(path)]
 assert sha(path)==row['historical_sha256'],path
 target=Path(row['operational_path']);assert sha(target)==row['operational_sha256'],target
 return json.loads(target.read_text())
