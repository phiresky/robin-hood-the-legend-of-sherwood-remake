"""Consolidate one complete foliage family's saved geometry evidence for grouped review."""
import html,json,os,sys
from pathlib import Path
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE));import qualify as q

def main():
    dest=q.OUT/'profile04-geometry-card-v1';assert not dest.exists()
    construction=json.loads((q.OUT/'geometry-v8-short-support04-v4/report.json').read_text());junction_dir=q.OUT/'geometry-v8-short-support04-review-v6-geometry-junctions';junctions=json.loads((junction_dir/'report.json').read_text());review=json.loads((junction_dir/'self-review.json').read_text());assert review['status']=='PASS_TARGETED_SUPPORT_GEOMETRY_WITH_DISCLOSURES';assert len(junctions['reviews'])==11
    assert construction['model']==junctions['model']==review['model'];assert q.pin(q.ROOT/construction['model']['path'])==construction['model']
    dest.mkdir();pins=[construction['model']];parts=[]
    def img(path,label):
        pins.append(q.pin(path));return '<figure><figcaption>'+html.escape(label)+'</figcaption><a href="'+html.escape(os.path.relpath(path,dest))+'"><img loading="lazy" src="'+html.escape(os.path.relpath(path,dest))+'"></a></figure>'
    parts.append('<h1>Hidden archer04 foliage cover</h1><p>Geometry only: the four authored poses and eleven short inferred attachments to the adjacent wood. Original camera is top-left in every model sheet. Gray unseen surfaces still need texture fill.</p>')
    parts.append(img(q.OUT/'profile04-native-context-v1/native-states.png','Original artwork — all four states'))
    parts.append('<p>The higher ends attach to the two adjacent trunks. The shallow hanging-cover depth is inferred; this is not a complete new tree crown. Tiny isolated leaf details remain from the source. One 1×1 native-pixel region overlaps the neighboring shrub foliage in three poses; no wood or rigid receiver is crossed. These limited ambiguities are disclosed rather than forced into a precise physical reconstruction.</p>')
    for phase in ['initial','transition-0','transition-1','applied']:
        base=q.OUT/('geometry-v8-short-support04-review-v3-junctions'if phase in ['initial','transition-0']else'geometry-v8-short-support04-review-v4-remaining')/phase
        parts.append('<h2>'+html.escape(phase)+'</h2>');parts.append(img(base/'textured-eight.png','Saved model — existing source texture / gray unknown surfaces'));parts.append(img(base/'solid-eight.png','Saved model — solid geometry'))
    parts.append('<details><summary>Measured support junctions — diagnostic colors</summary><p>Orange marks the inferred support; brown is exact target wood geometry. Other receivers are hidden in these closeups; a separate full-scene finite-volume audit checked every support.</p>')
    for row in junctions['reviews']:
        p=q.ROOT/row['sheet']['path'];assert q.pin(p)==row['sheet'];parts.append(img(p,row['phase']+' / component '+str(row['component'])))
    parts.append('</details>')
    for path in [q.OUT/'qualification.json',q.OUT/'geometry-v8-short-support04-v4/report.json',q.OUT/'geometry-v8-support04-sweep-v4/report.json',q.OUT/'geometry-v8-short-support04-v4/residual-corner-contact.json',q.OUT/'permitted-reference-foliage-representation.json',junction_dir/'report.json',junction_dir/'self-review.json']:pins.append(q.pin(path))
    (dest/'index.html').write_text('<!doctype html><meta charset="utf-8"><title>Hidden archer04 geometry</title><style>body{background:#171b20;color:#e7ebef;font:16px system-ui;max-width:1200px;margin:32px auto;padding:0 20px}h1,h2{color:white}figure{margin:20px 0}figcaption{margin-bottom:8px;color:#cad5df}img{max-width:100%;height:auto;background:#30363e}details{border:1px solid #56616c;padding:16px}summary{cursor:pointer}p{line-height:1.5;max-width:1000px}</style>'+''.join(parts))
    scope='Four discrete authored foliage poses and eleven inferred short wood attachments. No texture-fill or animation/runtime integration approval.'
    (dest/'handoff.json').write_text(json.dumps(dict(status='GEOMETRY_READY_FOR_GROUPED_USER_REVIEW',card_id='croisement02-hidden-archer04-foliage-geometry',title='Hidden archer04 foliage cover — four states',scope=scope,model=construction['model'],gallery=q.pin(dest/'index.html'),inputs=pins,limits=['Fine leaf islands retained.','One native-pixel foliage overlap disclosed.','Gray unseen surfaces pending approved texture workflow.','Native discrete holds retained; no continuous morph asserted.','Canonical assets and qualified SCB couplings unchanged.']),indent=2)+'\n')
    print(dest/'index.html')
if __name__=='__main__':main()
