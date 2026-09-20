"""Build a portable review gallery from existing, unmodified render sheets."""

import argparse
import hashlib
import html
import json
from pathlib import Path
import shutil


def build(index_path, output):
    index_path = Path(index_path).resolve(strict=True)
    output = Path(output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    data = json.loads(index_path.read_text())
    items = data["items"]
    ids = [item["id"] for item in items]
    if len(set(ids)) != len(ids):
        raise ValueError("Duplicate review identifiers")
    records, cards = [], []
    for number, item in enumerate(items, 1):
        figures, evidence = [], {}
        sheets = [("solid", "Solid geometry"), ("textured", "Original textures + shaded unknown surfaces")]
        if item.get("context"):
            sheets.append(("context", "Original artwork with surrounding context"))
        for key, label in sheets:
            source = Path(item[key])
            if not source.is_absolute():
                source = index_path.parent / source
            source = source.resolve(strict=True)
            relative = f"images/{number:02}-{key}.png"
            target = output / relative
            target.parent.mkdir(exist_ok=True)
            shutil.copyfile(source, target)
            digest = hashlib.sha256(source.read_bytes()).hexdigest()
            if hashlib.sha256(target.read_bytes()).hexdigest() != digest:
                raise RuntimeError(f"Review image copy differs: {source}")
            evidence[key] = {"source": str(source), "file": relative, "sha256": digest}
            figures.append(f'<figure data-kind="{key}"><figcaption>{label}</figcaption>'
                           f'<a href="{relative}" target="_blank"><img src="{relative}" '
                           f'loading="lazy" alt="{html.escape(item["name"])} — {label}"></a></figure>')
        notes = item.get("notes", "")
        if isinstance(notes, list):
            notes = " ".join(notes)
        cards.append(f'<article id="asset-{number}"><h2>{number}. {html.escape(item["name"])}</h2>'
                     f'<p class="status">{html.escape(item["status"])}</p>'
                     f'<p>{html.escape(notes)}</p><div class="sheets">{"".join(figures)}</div></article>')
        records.append({**item, "number": number, "images": evidence})
    nav = "".join(f'<a href="#asset-{n}">{n}. {html.escape(item["name"])}</a>' for n, item in enumerate(items, 1))
    document = '''<!doctype html><html lang="en"><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Derby model review</title><style>
*{box-sizing:border-box}body{margin:0;background:#171a20;color:#eee;font:16px/1.5 system-ui,sans-serif}
header,main{max-width:1700px;margin:auto;padding:24px}h1{margin:0}h2{font-size:24px}
nav{display:flex;gap:8px;flex-wrap:wrap;margin:18px 0}a{color:#afd3ff}nav a{padding:5px 10px;background:#28313f;border-radius:5px}
article{padding:20px 0 40px;border-top:1px solid #455064;scroll-margin-top:15px}.status{color:#ffd898;font-weight:600}
.sheets{display:grid;grid-template-columns:1fr 1fr;gap:16px}figure{margin:0}figcaption{padding:8px 0;color:#c2cddd}
img{display:block;width:100%;background:black}select{font:inherit;padding:6px;border-radius:5px}
figure[data-kind=context] img{width:auto;max-width:100%;max-height:400px}figure[data-kind=context]{grid-column:1/-1}
body[data-mode=solid] figure[data-kind=textured],body[data-mode=textured] figure[data-kind=solid]{display:none}
body:not([data-mode=both]) .sheets{grid-template-columns:1fr}
@media(max-width:1000px){.sheets{grid-template-columns:1fr}}
</style><body data-mode="both"><header><h1>Derby model review</h1>
<p>Geometry candidates, not generated textures. Gray means no accepted original texture.
Click any sheet for its full resolution. Review status does not imply user approval.</p>
<label>Show <select id="mode"><option value="both">Both sheets</option><option value="solid">Solid geometry</option>
<option value="textured">Original textures + gray</option></select></label><nav>'''+nav+'''</nav></header><main>'''+"".join(cards)+'''</main>
<script>document.querySelector('#mode').addEventListener('change',e=>document.body.dataset.mode=e.target.value);</script></body></html>'''
    (output / "index.html").write_text(document)
    (output / "evidence.json").write_text(json.dumps({"source_index": str(index_path), "items": records}, indent=2)+"\n")
    print(json.dumps({"gallery": str(output / "index.html"), "candidates": len(items), "images": sum(len(r["images"]) for r in records)}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("index")
    parser.add_argument("output")
    args = parser.parse_args()
    build(args.index, args.output)
