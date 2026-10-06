"""Build a portable review gallery from existing, unmodified render sheets."""

import argparse
import errno
import hashlib
import html
import json
import os
import re
from pathlib import Path
import shutil
import tempfile


def _sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def _atomic_copy(source, target, digest):
    """Never truncate an inode that a frozen history entry may share."""
    target.parent.mkdir(parents=True, exist_ok=True)
    if target.is_file() and _sha(target) == digest:
        return
    fd, temporary = tempfile.mkstemp(prefix='.' + target.name + '-', dir=target.parent)
    os.close(fd)
    temporary = Path(temporary)
    try:
        shutil.copyfile(source, temporary)
        if _sha(temporary) != digest:
            raise ValueError(f'Review resource changed while copying: {source}')
        shutil.copymode(source, temporary)
        os.replace(temporary, target)
    finally:
        temporary.unlink(missing_ok=True)


def _atomic_text(target, text):
    fd, temporary = tempfile.mkstemp(prefix='.' + target.name + '-', dir=target.parent)
    temporary = Path(temporary)
    try:
        with os.fdopen(fd, 'w', encoding='utf-8') as stream:
            stream.write(text)
        temporary.chmod(target.stat().st_mode & 0o777 if target.exists() else 0o644)
        os.replace(temporary, target)
    finally:
        temporary.unlink(missing_ok=True)


def _archive_previous(output):
    previous = output / 'evidence.json'
    if not previous.exists():
        return
    evidence = previous.read_bytes()
    resources = {}
    for item in json.loads(evidence)['items']:
        for kind in ('images', 'reports', 'reference_images'):
            for entry in item.get(kind, {}).values():
                relative = Path(entry['file'])
                if (relative.is_absolute() or '..' in relative.parts
                        or not relative.parts or relative.parts[0] not in ('images', 'reports')):
                    raise ValueError(f'Unsafe archived resource path: {relative}')
                digest = entry['sha256']
                if relative in resources and resources[relative] != digest:
                    raise ValueError(f'Conflicting archived resource hashes: {relative}')
                resources[relative] = digest
    # Check every dependency before publishing an archive or changing the gallery.
    for relative, digest in resources.items():
        if _sha(output / relative) != digest:
            raise ValueError(f'Frozen review resource changed: {relative}')
    documents = {Path(name): _sha(output / name) for name in ('index.html', 'evidence.json')}
    if documents[Path('evidence.json')] != hashlib.sha256(evidence).hexdigest():
        raise ValueError('Review evidence changed while archiving')
    archive = output / 'history' / hashlib.sha256(evidence).hexdigest()[:16]
    if archive.exists():
        for relative, digest in {**resources, Path('evidence.json'): documents[Path('evidence.json')]}.items():
            if _sha(archive / relative) != digest:
                raise ValueError(f'Archived review resource changed: {archive / relative}')
        if not (archive / 'index.html').is_file():
            raise FileNotFoundError(archive / 'index.html')
        return
    archive.parent.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.archive-', dir=archive.parent) as temporary:
        staging = Path(temporary)
        for relative, digest in {**resources, **documents}.items():
            source, target = output / relative, staging / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            try:
                os.link(source, target)
            except OSError as error:
                if error.errno not in (errno.EXDEV, errno.EPERM, errno.EOPNOTSUPP):
                    raise
                _atomic_copy(source, target, digest)
            if _sha(target) != digest:
                raise ValueError(f'Review resource changed while archiving: {source}')
        os.rename(staging, archive)


FEEDBACK_SCRIPT = r"""
(() => {
  const cards = [...document.querySelectorAll('article[data-review-revision]')];
  const preview = document.querySelector('#review-export');
  const message = document.querySelector('#copy-status');
  const namespace = 'model-review-v1:' + document.title + ':';
  const key = card => namespace + card.id + ':' + card.dataset.reviewRevision;
  const drafts = new Map();
  const clean = value => value.replace(/\s+/g, ' ').trim();
  function render(card) {
    const draft = drafts.get(key(card));
    const decision = card.querySelector('.decision');
    const note = card.querySelector('.review-note');
    if (decision.value !== draft.decision) decision.value = draft.decision;
    if (note.value !== draft.note) note.value = draft.note;
    card.querySelectorAll('.decision-action').forEach(button => {
      button.setAttribute('aria-pressed', String(button.dataset.decision === draft.decision));
    });
  }
  function refresh() {
    cards.forEach(render);
    const lines = cards.flatMap(card => {
      const draft = drafts.get(key(card));
      const decision = draft.decision;
      const note = clean(draft.note);
      if (!decision && !note) return [];
      return [`${card.id}: ${decision || 'feedback'}${note ? ' — ' + note : ''} [review ${card.dataset.reviewRevision}]`];
    });
    preview.value = lines.length ? document.title + '\n' + lines.join('\n') : '';
    document.querySelector('#review-count').textContent = `${lines.length} reviewed`;
    document.querySelector('#copy-reviews').disabled = !lines.length;
  }
  function restore(card) {
    const decision = card.querySelector('.decision');
    const note = card.querySelector('.review-note');
    const status = card.querySelector('.draft-status');
    // Native form restoration can follow card positions after a gallery rebuild.
    // Only our asset-and-revision keyed draft may populate these controls.
    const draft = {decision: '', note: '', asset_id: card.id, revision: card.dataset.reviewRevision};
    drafts.set(key(card), draft);
    status.textContent = '';
    try {
      const saved = JSON.parse(localStorage.getItem(key(card)) || 'null');
      if (saved && (!saved.asset_id || saved.asset_id === card.id) &&
          (!saved.revision || saved.revision === card.dataset.reviewRevision)) {
        if ([...decision.options].some(option => option.value === saved.decision && !option.disabled)) {
          draft.decision = saved.decision;
        }
        draft.note = typeof saved.note === 'string' ? saved.note : '';
        status.textContent = 'Restored saved review';
      }
    } catch {
      status.textContent = 'Browser storage unavailable; copy your results before closing.';
    }
    render(card);
  }
  for (const card of cards) {
    restore(card);
    const decision = card.querySelector('.decision');
    const note = card.querySelector('.review-note');
    const status = card.querySelector('.draft-status');
    const save = field => {
      const draft = drafts.get(key(card));
      // Read only the field being edited. Another control may contain a value
      // restored by the browser; it must never enter our saved/exported record.
      if (field === 'decision') {
        if (![...decision.options].some(option => option.value === decision.value && !option.disabled)) {
          refresh();
          return;
        }
        draft.decision = decision.value;
      } else draft.note = note.value;
      try {
        if (!draft.decision && !draft.note) localStorage.removeItem(key(card));
        else localStorage.setItem(key(card), JSON.stringify(draft));
        status.textContent = 'Saved in this browser';
      } catch {
        status.textContent = 'Could not save in this browser; copy your results before closing.';
      }
      message.textContent = '';
      refresh();
    };
    decision.addEventListener('change', () => save('decision'));
    card.querySelectorAll('.decision-action').forEach(button => button.addEventListener('click', () => {
      decision.value = button.dataset.decision;
      save('decision');
    }));
    note.addEventListener('input', () => save('note'));
    card.querySelector('.feedback').addEventListener('focusin', () => render(card));
  }
  window.addEventListener('pageshow', () => {
    refresh();
  });
  document.addEventListener('visibilitychange', () => {
    if (!document.hidden) refresh();
  });
  document.querySelector('#clear-reviews').addEventListener('click', () => {
    let clearedStorage = true;
    try {
      const keys = Array.from({length: localStorage.length}, (_, index) => localStorage.key(index));
      for (const savedKey of keys) {
        if (savedKey && savedKey.startsWith(namespace)) localStorage.removeItem(savedKey);
      }
    } catch {
      clearedStorage = false;
    }
    for (const card of cards) {
      Object.assign(drafts.get(key(card)), {decision: '', note: ''});
      card.querySelector('.draft-status').textContent = '';
    }
    refresh();
    message.textContent = clearedStorage
      ? 'Reviews cleared for this gallery.'
      : 'Reviews cleared on this page, but browser storage could not be cleared.';
  });
  document.querySelector('#copy-reviews').addEventListener('click', async () => {
    refresh();
    try {
      await navigator.clipboard.writeText(preview.value);
      message.textContent = 'Copied. Paste into chat.';
    } catch {
      document.querySelector('#export-details').open = true;
      preview.focus();
      preview.select();
      let copied = false;
      try { copied = document.execCommand('copy'); } catch { /* Manual selection remains available. */ }
      message.textContent = copied ? 'Copied. Paste into chat.' : 'Press Ctrl+C / Cmd+C to copy the selected results.';
    }
  });
  refresh();
})();
"""

def build(index_path, output, *, pending_only=False, map_name=None):
    index_path = Path(index_path).resolve(strict=True)
    output = Path(output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    data = json.loads(index_path.read_text())
    map_name = map_name if map_name is not None else data.get("map", "Derby")
    if not isinstance(map_name, str) or not map_name.strip():
        raise ValueError("Review gallery requires a nonempty map name")
    texture_review = data.get("review_kind") == "texture"
    grouping_review = data.get("review_kind") == "grouping"
    title = html.escape(map_name.strip() + (" grouping review" if grouping_review else " review" if texture_review else " model review"))
    description = ("Generated textures baked onto the approved geometry. Review the actual mesh views and every additional state. "
                   "Click any sheet for its full resolution. Texture approval is a separate decision."
                   if texture_review else "Geometry candidates, not generated textures. Gray means no accepted original texture. "
                   "Click any sheet for its full resolution. Review status does not imply user approval.")
    texture_mode_label = "Baked textures" if texture_review else "Original textures + gray"
    if grouping_review:
        description = ('Review asset names and which source parts belong together. Approve grouping confirms ownership only; '
                       'it does not approve geometry completion, textures, or publication. Request changes and describe the correction below.')
    items = data["items"]
    if pending_only:
        items = [item for item in items if not (str(item.get("user_approval", "")).lower().startswith("approved")
                 and item.get("technical_eligible", True))]
    ids = [item["id"] for item in items]
    if len(set(ids)) != len(ids):
        raise ValueError("Duplicate review identifiers")
    _archive_previous(output)
    records, cards = [], []
    status_counts = data.get('status_counts', {})
    status_summary = ('<p>' + html.escape(', '.join(
        f'{count} {status}' for status, count in sorted(status_counts.items()))) +
        '.</p>') if status_counts else ''
    missing = data.get('without_packets', [])
    missing_section = ''
    if missing:
        rows = ''.join('<tr><td>' + html.escape(item['name']) + '</td><td><code>' +
                       html.escape(item['id']) + '</code></td><td>' + html.escape(item['status']) +
                       '</td><td>' + html.escape(item.get('reason', '')) + '</td></tr>' for item in missing)
        missing_section = ('<section><h2>Assets awaiting complete review packets</h2>'
                           '<p>These assets are still in progress and are not ready for approval.</p>'
                           '<table><thead><tr><th>Asset</th><th>ID</th><th>Status</th><th>Details</th></tr></thead>'
                           '<tbody>' + rows + '</tbody></table></section>')
    for number, item in enumerate(items, 1):
        asset_id = item["id"]
        if not re.fullmatch(r"[a-zA-Z0-9_-]+", asset_id):
            raise ValueError(f"Unsafe review identifier: {asset_id}")
        figures, evidence = [], {}
        # Context references supplement an already reviewed texture packet. Keep
        # their archive hashes separate from the decision/revision fingerprint.
        reference_evidence, reference_figures = {}, []
        for reference in item.get('artwork_references', []):
            identifier = reference['id']
            if not re.fullmatch(r'[a-zA-Z0-9_-]+', identifier) or identifier in reference_evidence:
                raise ValueError('Unsafe or duplicate artwork reference identifier')
            source = Path(reference['path'])
            if not source.is_absolute():
                source = index_path.parent / source
            source = source.resolve(strict=True)
            digest = hashlib.sha256(source.read_bytes()).hexdigest()
            if digest != reference['sha256']:
                raise ValueError('Original artwork reference changed before gallery copy')
            relative = f'images/{asset_id}-artwork-{identifier}-{digest[:16]}.png'
            target = output / relative
            target.parent.mkdir(exist_ok=True)
            _atomic_copy(source, target, digest)
            if hashlib.sha256(target.read_bytes()).hexdigest() != digest:
                raise RuntimeError('Artwork reference copy differs')
            reference_evidence[identifier] = {**reference, 'file': relative}
            label = html.escape(reference['label'])
            reference_figures.append(f'<figure data-kind="artwork-reference-context"><figcaption>{label}</figcaption>'
                f'<a href="{relative}" target="_blank"><img src="{relative}" loading="lazy" alt="{label}"></a></figure>')
        animation_figures, animation_keys = {}, {}
        sheets = [("solid", item.get("solid_label", "Solid geometry")),
                  ("textured", item.get("textured_label", "Original textures + shaded unknown surfaces"))]
        if grouping_review:
            sheets = [("solid", item.get("solid_label", "Selected geometry — west")),
                      ("east_solid", item.get("east_solid_label", "Selected geometry — east"))]
        if item.get("context"):
            sheets.append(("context", "Original artwork with surrounding context"))
        for key, label in (("source_comparison", "Original artwork / before / corrected"),
                           ("source_comparison_secondary", "Additional source-camera comparison"),
                           ("source_trace", "Numbered source artwork corners"),
                           ("projection_errors", "Corrected mesh projected onto original artwork")):
            if item.get(key):
                sheets.append((key, item.get(key + "_label", label)))
        for key, label in (("revealed_solid", "Revealed interior geometry"),
                           ("revealed_textured", "Revealed interior original textures + shaded unknown surfaces"),
                           ("revealed_context", "Original revealed artwork with surrounding context")):
            if item.get(key):
                sheets.append((key, label))
        if item.get('stored_material_textured'):
            sheets.append(('stored_material_textured', 'Actual saved UVs and atlas materials'))
        for state in item.get('stored_material_states', []):
            if state.get('sheet'):
                key = 'stored_material_' + state['id'] + '_textured'
                item[key] = state['sheet']
                sheets.append((key, 'Actual saved materials: ' + html.escape(state['id'])))
        if item.get('endpoint_reviews'):
            sheets = []
            for endpoint in item['endpoint_reviews']:
                state = endpoint['id']
                if not re.fullmatch(r'[a-zA-Z0-9_-]+', state):
                    raise ValueError('Unsafe endpoint review identifier')
                for field, label in (('solid', 'solid geometry — all eight views'),
                                     ('textured', 'source projection — all eight views'),
                                     ('stored_material_textured', 'actual saved materials — all eight views'),
                                     ('context', 'original endpoint artwork')):
                    if endpoint.get(field):
                        key = 'endpoint_' + state + '_' + field
                        item[key] = endpoint[field]
                        sheets.append((key, state.capitalize() + ': ' + label))
        for state in item.get('animation_reviews', []):
            identifier = state['id']
            if not re.fullmatch(r'[a-zA-Z0-9_-]+', identifier) or identifier in animation_figures:
                raise ValueError('Unsafe or duplicate animation state identifier')
            animation_figures[identifier] = []
            for field, label in (('solid', 'Solid geometry'), ('textured', 'Source textures'),
                                 ('context', 'Original state artwork')):
                key = 'animation_' + identifier + '_' + field
                item[key] = state[field]
                animation_keys[key] = identifier
                sheets.append((key, state['name'] + ': ' + label))
        texture_state_ids = set()
        texture_labels = {'solid': 'solid geometry', 'textured': 'actual saved materials',
                          'source_comparison': 'approved source textures',
                          'source_comparison_secondary': 'source-preserved generated sheet',
                          'source_trace': 'raw generated reference'}
        for state in item.get('texture_states', []):
            identifier = state['id']
            if not re.fullmatch(r'[a-zA-Z0-9_-]+', identifier) or identifier in texture_state_ids:
                raise ValueError('Unsafe or duplicate texture state identifier')
            texture_state_ids.add(identifier)
            for field in state['image_fields']:
                sheets.append(('texture_state_' + identifier + '_' + field,
                               state['name'] + ': ' + state.get('image_labels', {}).get(field, texture_labels[field])))
        for key, label in sheets:
            source = Path(item[key])
            if not source.is_absolute():
                source = index_path.parent / source
            source = source.resolve(strict=True)
            digest = hashlib.sha256(source.read_bytes()).hexdigest()
            relative = f"images/{asset_id}-{key}-{digest[:16]}.png"
            target = output / relative
            target.parent.mkdir(exist_ok=True)
            _atomic_copy(source, target, digest)
            digest = hashlib.sha256(source.read_bytes()).hexdigest()
            if hashlib.sha256(target.read_bytes()).hexdigest() != digest:
                raise RuntimeError(f"Review image copy differs: {source}")
            evidence[key] = {"source": str(source), "file": relative, "sha256": digest}
            destination = animation_figures[animation_keys[key]] if key in animation_keys else figures
            destination.append(f'<figure data-kind="{key}"><figcaption>{html.escape(label)}</figcaption>'
                           f'<a href="{relative}" target="_blank"><img src="{relative}" '
                           f'loading="lazy" alt="{html.escape(item["name"])} — {label}"></a></figure>')
        notes = item.get("notes", "")
        if isinstance(notes, list):
            notes = " ".join(notes)
        reports = {}
        report_links = []
        report_specs = [("validation", "Validation report"),
                           ("ownership", "Source ownership evidence"),
                           ("review", "Worker review and limitations"),
                           ("disclosure", "Candidate assumptions and limitations"),
                           ("stored_material_audit", "Stored UV/material audit"),
                           ("stored_material_glb", "Actual exported GLB")]
        for state in item.get('stored_material_states', []):
            key = 'stored_material_' + state['id'] + '_audit'
            if Path(state['audit']).is_file():
                item[key] = state['audit']
                report_specs.append((key, 'Stored material audit: ' + html.escape(state['id'])))
        if item.get('endpoint_reviews'):
            report_specs = []
            for endpoint in item['endpoint_reviews']:
                for field, label in (('validation', 'validation'), ('ownership', 'source ownership'),
                                     ('review', 'worker review'), ('frames', 'frozen cameras'),
                                     ('stored_material_audit', 'stored material audit'),
                                     ('stored_material_glb', 'actual exported GLB')):
                    if endpoint.get(field):
                        key = 'endpoint_' + endpoint['id'] + '_' + field
                        item[key] = endpoint[field]
                        report_specs.append((key, endpoint['id'].capitalize() + ': ' + label))
        for state in item.get('texture_states', []):
            for field in state['report_fields']:
                report_specs.append(('texture_state_' + state['id'] + '_' + field,
                                     state['name'] + ': ' + field))
        for key, label in report_specs:
            if not item.get(key):
                continue
            source = Path(item[key])
            if not source.is_absolute():
                source = index_path.parent / source
            source = source.resolve(strict=True)
            digest = hashlib.sha256(source.read_bytes()).hexdigest()
            relative = f"reports/{asset_id}-{key}-{digest[:16]}{source.suffix}"
            target = output / relative
            target.parent.mkdir(exist_ok=True)
            _atomic_copy(source, target, digest)
            digest = hashlib.sha256(source.read_bytes()).hexdigest()
            if hashlib.sha256(target.read_bytes()).hexdigest() != digest:
                raise RuntimeError(f"Review report copy differs: {source}")
            reports[key] = {"source": str(source), "file": relative, "sha256": digest}
            report_links.append(f'<a href="{relative}" target="_blank">{label}</a>')
        paired_note = ''
        if item.get('texture_states'):
            paired_note = '<p><strong>Texture approval covers the main views and every additional state shown below.</strong></p>'
        if item.get('endpoint_reviews'):
            paired_note = '<p><strong>Paired endpoint review: approval covers both initial and applied models.</strong></p>'
            paired_note += '<ul>' + ''.join('<li>' + html.escape(endpoint['id'].capitalize() + ': ' + endpoint['status']) +
                ' · model <code>' + html.escape(endpoint.get('model_sha256') or 'missing') + '</code></li>'
                for endpoint in item['endpoint_reviews']) + '</ul>'
            if item.get('endpoint_review_errors'):
                paired_note += '<p>' + html.escape('; '.join(item['endpoint_review_errors'])) + '</p>'
        binding = {'images': {key: value['sha256'] for key, value in evidence.items()},
                   'reports': {key: value['sha256'] for key, value in reports.items()}}
        if item.get('model'):
            model = Path(item['model'])
            if not model.is_absolute():
                model = index_path.parent / model
            binding['model'] = hashlib.sha256(model.read_bytes()).hexdigest()
        revision = hashlib.sha256(json.dumps(binding, sort_keys=True).encode()).hexdigest()
        approval_disabled = '' if item['status'] == 'ready-for-user' and item.get('technical_eligible', True) else ' disabled'
        buttons = (f'<div><button class="decision-action" type="button" data-decision="approved"{approval_disabled}>Approve grouping</button> '
                   '<button class="decision-action" type="button" data-decision="needs refinement">Request changes</button></div>') if grouping_review else ''
        controls = (f'<fieldset class="feedback"><legend>Your review</legend>{buttons}'
                    f'<label class="decision-field">Decision <select class="decision" autocomplete="off" '
                    f'name="decision-{asset_id}-{revision[:16]}" aria-label="Decision for {asset_id}">'
                    '<option value="">Not decided</option>'
                    f'<option value="approved"{approval_disabled}>Approve</option>'
                    '<option value="needs refinement">Needs refinement</option></select></label>'
                    f'<label class="note-field">Feedback <textarea class="review-note" rows="2" autocomplete="off" '
                    f'name="feedback-{asset_id}-{revision[:16]}" '
                    f'aria-label="Feedback for {asset_id}" placeholder="What should change, or any notes?"></textarea></label>'
                    '<span class="draft-status" aria-live="polite"></span></fieldset>')
        animation_sections = ''
        if animation_figures:
            animation_sections = '<p>Your decision covers the main views and these states of the same asset.</p>'
            for state in item['animation_reviews']:
                animation_sections += ('<details class="animation-state"><summary>' + html.escape(state['name']) +
                    '</summary><p>' + html.escape(state.get('description', '')) + '</p><div class="sheets">' +
                    ''.join(animation_figures[state['id']]) + '</div></details>')
        cards.append(f'<article id="{asset_id}" data-review-revision="{revision[:16]}"><h2>{number}. {html.escape(item["name"])}</h2>'
                     f'<p><code>{html.escape(item["id"])}</code></p>'
                     f'<p class="status">{html.escape(item["status"])}</p>'
                     f'<p>{html.escape(notes)}</p>{paired_note}<p>{" · ".join(report_links)}</p>'
                     f'<div class="sheets">{"".join(reference_figures)}{"".join(figures)}</div>{animation_sections}{controls}</article>')
        records.append({**item, "number": number, "images": evidence, "reports": reports,
                        "review_revision": revision, **({"reference_images": reference_evidence} if reference_evidence else {})})
    nav = "".join(f'<a href="#{item["id"]}">{n}. {html.escape(item["name"])}</a>' for n, item in enumerate(items, 1))
    document = '''<!doctype html><html lang="en"><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>'''+title+'''</title><style>
*{box-sizing:border-box}body{margin:0;background:#171a20;color:#eee;font:16px/1.5 system-ui,sans-serif}
header,main{max-width:1700px;margin:auto;padding:24px}h1{margin:0}h2{font-size:24px}
nav{display:flex;gap:8px;flex-wrap:wrap;margin:18px 0}a{color:#afd3ff}nav a{padding:5px 10px;background:#28313f;border-radius:5px}
article{padding:20px 0 40px;border-top:1px solid #455064;scroll-margin-top:15px}.status{color:#ffd898;font-weight:600}
.sheets{display:grid;grid-template-columns:1fr 1fr;gap:16px}figure{margin:0}figcaption{padding:8px 0;color:#c2cddd}
img{display:block;width:100%;background:black}select{font:inherit;padding:6px;border-radius:5px}
.feedback{margin:16px 0;padding:12px;border:1px solid #455064;border-radius:6px;display:grid;gap:8px}
.feedback label{display:grid;gap:4px}.feedback select{width:fit-content}
textarea{font:inherit;width:100%;padding:8px;background:#202731;color:#eee;border:1px solid #65758c;border-radius:5px;resize:vertical}
button{font:inherit;padding:8px 14px;cursor:pointer;border-radius:5px}.draft-status,#copy-status{color:#b9d4b8;font-size:14px}
.review-export{position:sticky;bottom:0;background:#202731;border-top:1px solid #65758c;padding:12px 24px;z-index:2}
.review-export details{max-width:1000px}.review-export textarea{max-height:220px}
table{border-collapse:collapse;width:100%}th,td{text-align:left;padding:8px;border-bottom:1px solid #455064;overflow-wrap:anywhere}
[hidden]{display:none!important}
.animation-state{border:1px solid #455064;border-radius:6px;padding:12px;margin-top:16px}.animation-state summary{cursor:pointer;font-weight:600}
figure[data-kind$=context] img{width:auto;max-width:100%;max-height:400px}figure[data-kind$=context]{grid-column:1/-1}
body[data-mode=solid] figure[data-kind$=textured],body[data-mode=textured] figure[data-kind$=solid]{display:none}
body:not([data-mode=both]) .sheets{grid-template-columns:1fr}
@media(max-width:1000px){.sheets{grid-template-columns:1fr}}
</style><body data-mode="both"><header><h1>'''+title+'''</h1>
<p>'''+description+'''</p>
'''+(f'<p><strong>{data.get("total_groups", len(items))} catalog assets'
      +(f' plus {data["supplemental_count"]} separate terrain packet' if data.get('supplemental_count') else '')+
      f'; {len(items)} pending review packets'
      f' and {len(missing)} assets awaiting packets.</strong></p>' if 'total_groups' in data else '')+'''
'''+status_summary+'''
'''+(f'<p><strong>Approved models are hidden. {len(items)} displayed packets; '
      f'{sum(item["status"] == "ready-for-user" for item in items)} ready for your decision.</strong> '
      'Items marked validation-pending or fix-needed are still being worked on.</p>' if pending_only else '')+'''
<label>Show <select id="mode"><option value="both">Both sheets</option><option value="solid">Solid geometry</option>
<option value="textured">'''+texture_mode_label+'''</option></select></label>
<label>Assets <select id="readiness"><option value="all">All pending assets</option>
<option value="ready">Ready for review</option></select></label><nav>'''+nav+'''</nav></header><main>'''+"".join(cards)+missing_section+'''</main>
<footer class="review-export"><button id="copy-reviews" type="button">Copy review results</button>
<button id="clear-reviews" type="button" title="Clear saved decisions and notes for this gallery">Clear reviews</button>
<span id="review-count"></span> <span id="copy-status" role="status"></span>
<details id="export-details"><summary>Preview / copy manually</summary>
<textarea id="review-export" readonly rows="5" aria-label="Review results to paste into chat"></textarea></details>
<small> Choices are saved in this browser for this revision. Paste the results into chat to submit them.</small></footer>
<script>
document.querySelector('#mode').addEventListener('change',e=>document.body.dataset.mode=e.target.value);
document.querySelector('#readiness').addEventListener('change',event=>{
  const onlyReady=event.target.value==='ready';
  for(const card of document.querySelectorAll('article')){
    const hidden=onlyReady&&card.querySelector('.status').textContent!=='ready-for-user';
    card.hidden=hidden;
    const link=document.querySelector('nav a[href="#'+card.id+'"]');
    if(link) link.hidden=hidden;
  }
});
</script><script>'''+FEEDBACK_SCRIPT+'''</script></body></html>'''
    if grouping_review:
        document = document.replace('<body data-mode="both">', '<body data-mode="both" data-review-kind="grouping">')
        document = document.replace('</style>', '''
body[data-review-kind=grouping] .sheets{grid-template-columns:repeat(4,minmax(0,1fr));gap:12px}
body[data-review-kind=grouping] figure[data-kind$=context]{grid-column:auto}
body[data-review-kind=grouping] figcaption{font-size:14px;min-height:54px}
body[data-review-kind=grouping] figure a{display:flex;align-items:center;justify-content:center;height:min(220px,32vh);background:#202731}
body[data-review-kind=grouping] figure img{width:auto;height:auto;max-width:100%;max-height:100%;object-fit:contain}
body[data-review-kind=grouping] .feedback{grid-template-columns:auto minmax(0,1fr);column-gap:20px}
body[data-review-kind=grouping] .feedback .decision-field{grid-column:1}
body[data-review-kind=grouping] .feedback .note-field{grid-column:2;grid-row:1 / span 2}
body[data-review-kind=grouping] .feedback .draft-status{grid-column:1/-1}
@media(max-width:1000px){body[data-review-kind=grouping] .sheets{grid-template-columns:repeat(2,minmax(0,1fr))}}
@media(max-width:600px){body[data-review-kind=grouping] .feedback{grid-template-columns:1fr}body[data-review-kind=grouping] .feedback .note-field{grid-column:1;grid-row:auto}}
</style>''')
        document = document.replace('Both sheets', 'Both geometry views').replace('Solid geometry</option>', html.escape(data.get('solid_view_label', 'West view'))+'</option>')
        document = document.replace(texture_mode_label+'</option>', html.escape(data.get('east_view_label', 'East view'))+'</option>')
        document = document.replace('body[data-mode=solid] figure[data-kind$=textured],body[data-mode=textured] figure[data-kind$=solid]',
            'body[data-mode=solid] figure[data-kind=east_solid],body[data-mode=textured] figure[data-kind=solid]')
        document = document.replace('<nav>'+nav+'</nav>',
            '<p><label>Find an asset <input id="asset-search" type="search" placeholder="Name or stable asset ID"></label></p>'
            '<details><summary>Jump to an asset</summary><nav>'+nav+'</nav></details>')
        document = document.replace('</style>', 'button[aria-pressed=true]{outline:3px solid #83caa3}input[type=search]{font:inherit;padding:8px;width:min(650px,100%)}\n</style>')
        document = document.replace('</body>', '''<script>
function filterGroupingAssets(){
  const q=document.querySelector('#asset-search').value.toLowerCase();
  const ready=document.querySelector('#readiness').value==='ready';
  for(const card of document.querySelectorAll('article')){
    card.hidden=!(card.id+' '+card.querySelector('h2').textContent).toLowerCase().includes(q)||(ready&&card.querySelector('.status').textContent!=='ready-for-user');
    const link=document.querySelector('nav a[href="#'+card.id+'"]');if(link)link.hidden=card.hidden;
  }
}
document.querySelector('#asset-search').addEventListener('input',filterGroupingAssets);
document.querySelector('#readiness').addEventListener('change',filterGroupingAssets);
</script></body>''')
    _atomic_text(output / "index.html", document)
    _atomic_text(output / "evidence.json", json.dumps({"source_index": str(index_path), "items": records,
                                                    "without_packets": missing}, indent=2)+"\n")
    print(json.dumps({"gallery": str(output / "index.html"), "candidates": len(items), "images": sum(len(r["images"]) for r in records)}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("index")
    parser.add_argument("output")
    parser.add_argument("--pending-only", action="store_true", help="Hide explicitly approved candidates")
    parser.add_argument("--map-name", help="Map name; defaults to the manifest map or Derby for legacy manifests")
    args = parser.parse_args()
    build(args.index, args.output, pending_only=args.pending_only, map_name=args.map_name)
