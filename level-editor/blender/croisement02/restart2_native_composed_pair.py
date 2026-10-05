"""Arrange the independent native coverage render beside its exact source image."""
import argparse,json,hashlib
from pathlib import Path
from PIL import Image

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 p=argparse.ArgumentParser();p.add_argument('workspace',type=Path);p.add_argument('output',type=Path);a=p.parse_args();w=a.workspace.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False);coverage=w/'inspection/source-coverage';report=json.loads((coverage/'report.json').read_text());digest=sha(w/'model.blend')
 if report['model_sha256']!=digest:raise ValueError('Stale native source render')
 source=Image.open(coverage/'source.png').convert('RGB');actual=Image.open(coverage/'render.png').convert('RGBA')
 if source.size!=actual.size:raise ValueError('Native crop mismatch')
 display=Image.new('RGBA',source.size,(80,80,80,255));display.alpha_composite(actual);sheet=Image.new('RGB',(source.width*2,source.height));sheet.paste(source,(0,0));sheet.paste(display.convert('RGB'),(source.width,0));sheet.resize((sheet.width*2,sheet.height*2),Image.Resampling.NEAREST).save(out/'source-comparison.png');(out/'evidence.json').write_text(json.dumps(dict(model_sha256=digest,source_render_sha256=sha(coverage/'render.png'),source_image_sha256=sha(coverage/'source.png'),coverage_report_sha256=sha(coverage/'report.json'),comparison_sha256=sha(out/'source-comparison.png'),scope='Exact source and saved full own-tree native projection, adjacent independently owned assets absent.'),indent=2)+'\n')
if __name__=='__main__':main()
