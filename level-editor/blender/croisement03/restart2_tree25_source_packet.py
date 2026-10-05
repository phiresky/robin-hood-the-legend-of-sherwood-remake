"""Document tree25 source traces without importing or modifying a model."""
import ast,json,hashlib
from pathlib import Path
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement03-refinement'
def main():
    recipe=Path(__file__).with_name('restart2_tree25_wood.py');parsed=ast.parse(recipe.read_text());paths=next(ast.literal_eval(n.value) for n in parsed.body if isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='PATHS' for t in n.targets))
    output=OUT/'restart2/tree25-source';output.mkdir(parents=True,exist_ok=True);source=OUT/'baseline/covered.png';image=Image.open(source).crop((1145,590,1395,825)).convert('RGB').resize((1000,940),Image.Resampling.NEAREST);draw=ImageDraw.Draw(image)
    for number,path in enumerate(paths):
        color=['red','cyan','magenta','orange','yellow'][number];points=[((x-1145)*4,(y-590)*4) for x,y,radius in path];draw.line(points,fill=color,width=3)
        for index,(x,y) in enumerate(points):
            draw.ellipse((x-4,y-4,x+4,y+4),fill=color);draw.text((x+5,y),f'{number}:{index}',fill=color,stroke_fill='black',stroke_width=1)
    image.save(output/'proposed-branch-trace.png')
    (output/'proposed-branch-trace.json').write_text(json.dumps(dict(paths=paths,recipe_sha256=hashlib.sha256(recipe.read_bytes()).hexdigest(),source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),source_crop=[1145,590,1395,825],scale=4,status='Private construction trace, not finished geometry or appearance review',inferences=['Visible fork centerlines approximate native artwork at pixel scale.','Branch radii, circular section, root depth and concealed continuations are inferred.','Only lower positive bark is assigned in the first worker; upper branch leaf ownership and crown are unfinished.']),indent=2)+'\n')
if __name__=='__main__':main()
