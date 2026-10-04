"""Link separately scoped review packets from the main Croisement02 gallery."""
import html
import json
from pathlib import Path
from evidence_io import sha


def records(root):
    worker=Path(root)/'ground-receiver-review-v5'
    packet=worker/'receiver-packet.json'
    if not packet.exists():return []
    receipt=json.loads(packet.read_text());model=sha(worker/'model.blend')
    if receipt['model_sha256']!=model:raise ValueError('Supplemental ground model changed')
    for field,name in [('ground_bank_partition_sha256','inspection/ground-bank-partition.json'),('root_review_sha256','inspection/root-review.json')]:
        if receipt[field]!=sha(worker/name):raise ValueError('Supplemental ground review changed')
    review=json.loads((worker/'inspection/visual-review.json').read_text())
    if review['model_sha256']!=model or not review['ready_for_geometry_review']:raise ValueError('Ground receiver is not ready for review')
    return [dict(id='ground-receiver',name='Ground receiver and source ownership',href='../ground-receiver-review-v5/gallery/index.html',status='Geometry/domain approval pending; no texture generation',model_sha256=model,packet_sha256=sha(packet),gallery_evidence_sha256=sha(worker/'gallery/evidence.json'))]


def append_section(gallery,items):
    if not items:return
    path=Path(gallery)/'index.html';document=path.read_text()
    links=''.join('<li><a href="'+html.escape(row['href'],quote=True)+'">'+html.escape(row['name'])+'</a> — '+html.escape(row['status'])+'</li>' for row in items)
    section='<section id="supplemental-reviews"><h2>Supplemental reviews</h2><p>Separately scoped packets; decisions here do not approve the remaining map integration.</p><ul>'+links+'</ul></section>'
    if '<main>' not in document:raise ValueError('Gallery main section missing')
    path.write_text(document.replace('<main>','<main>'+section,1))
