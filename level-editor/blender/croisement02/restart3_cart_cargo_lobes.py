"""Compare two tapered cargo segments and source-traced finite wood fragments."""
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
from restart3_cart_cargo_debris import *
from restart3_audit_cart_cargo import audit


def segmented(source):
    root=OUT/'restart3-south-cart';dest=root/'barrel-segmented-v2';dest.mkdir(exist_ok=False)
    fit=json.loads((root/'barrel-fit-v1/fit.json').read_text())
    frame=source['parts'][2]['frames'][-1]
    Image.open(root/'barrel-v1/source.png').save(dest/'source.png')
    scene,gray=start();paint=material(dest/'source.png');box=[1154,707,1218,832]
    angle,length,radius,cx,cy=fit['parameters'];axis=Vector((math.cos(angle),math.sin(angle),0));cross=Vector((-axis.y,axis.x,0))
    n=32;phase=math.pi/n;height=radius*math.cos(phase)
    center=point(1154+cx,707+cy,height)
    group='croisement02-south-cart-terminal-cask'
    for segment in range(2):
        midpoint=-.25+.5*segment
        profile=[(-.25,.86),(-.20,.91),(-.12,1),(.12,1),(.20,.91),(.25,.86)]
        vertices=[]
        for t,scale in profile:
            vertices.extend(center+axis*(length*(midpoint+t))+radius*scale*(cross*math.cos(phase+j*math.tau/n)+Vector((0,0,math.sin(phase+j*math.tau/n)))) for j in range(n))
        faces=[tuple(reversed(range(n))),tuple(range((len(profile)-1)*n,len(profile)*n))]
        for i in range(len(profile)-1):
            for j in range(n):faces.append((i*n+j,i*n+(j+1)%n,(i+1)*n+(j+1)%n,(i+1)*n+j))
        mesh(scene,f'Tapered cargo segment {segment+1}',vertices,faces,paint,gray,box,group)
    finish(scene,gray,dest,box,dict(status='Private two-segment hypothesis; count/material not established',asset_id=group,source_frame=frame,
        source_evidence=str(root/'barrel-axial-profile-v1/report.json'),source_evidence_sha256=sha(root/'barrel-axial-profile-v1/report.json'),
        fit=fit['parameters'],central_radius_fraction=.86,contact='Separate finite lower facets at Z0',
        limitations=['Two adjoining tapered bodies represent the native bright lobes and dark join physically; not a proven cargo count.',
                    'Dark lower-left native region remains interpreted as end face in this comparison, not conclusively identified.',
                    'Thin vertical source strip remains native-reserved, no solid role.',
                    'Hidden geometry inferred and gray; exact geometry is not approved.']))
    audit(dest)


def contours(domain):
    edges=set()
    for y,x in zip(*np.where(domain)):
        corners=[(int(x),int(y)),(int(x+1),int(y)),(int(x+1),int(y+1)),(int(x),int(y+1))]
        for a,b in zip(corners,corners[1:]+corners[:1]):
            if (b,a) in edges:edges.remove((b,a))
            else:edges.add((a,b))
    loops=[]
    while edges:
        a,b=min(edges);edges.remove((a,b));loop=[a,b]
        while loop[-1]!=loop[0]:
            choices=sorted(e for e in edges if e[0]==loop[-1])
            if not choices:raise RuntimeError('Open native boundary')
            e=choices[0];edges.remove(e);loop.append(e[1])
        p=np.array(loop[:-1]);other=np.roll(p,-1,axis=0);area=(p[:,0]*other[:,1]-p[:,1]*other[:,0]).sum()/2
        if area>=3:loops.append(loop[:-1])
    return loops


def simplify(points,tolerance=.4):
    def rdp(p):
        if len(p)<3:return p
        v=p[-1]-p[0];norm=np.linalg.norm(v)
        ds=np.linalg.norm(p-p[0],axis=1) if norm==0 else abs(v[0]*(p-p[0])[:,1]-v[1]*(p-p[0])[:,0])/norm
        index=int(np.argmax(ds))
        if ds[index]<=tolerance:return p[[0,-1]]
        return np.concatenate([rdp(p[:index+1])[:-1],rdp(p[index:])])
    p=np.array(points,dtype=float);j=int(np.argmax(np.linalg.norm(p-p[0],axis=1)))
    return np.concatenate([rdp(p[:j+1])[:-1],rdp(np.concatenate([p[j:],p[:1]]))[:-1]])


def traced_scraps(source):
    dest=OUT/'restart3-south-cart/loose-wood-traced-v3';dest.mkdir(exist_ok=False)
    frame=source['parts'][0]['frames'][-1];rgba=np.asarray(Image.open(frame['image']).convert('RGBA'))
    scene,gray=start();box=[953,844,1165,1001];records=[];selected=np.zeros(rgba.shape[:2],bool)
    regions=[('Long detached board',(54,128,68,157),.8),('Small detached chip',(88,132,99,141),.6),('Thin detached splinter',(109,138,117,142),.4)]
    for _,(left,top,right,bottom),_ in regions:selected[top:bottom,left:right]=rgba[top:bottom,left:right,3]>127
    bounded=rgba.copy();bounded[~selected,3]=0;Image.fromarray(bounded).save(dest/'source.png');Image.fromarray(selected.astype(np.uint8)*255).save(dest/'source-domain.png')
    paint=material(dest/'source.png');group='croisement02-south-cart-terminal-loose-wood'
    for name,(left,top,right,bottom),thickness in regions:
        domain=np.zeros_like(selected);domain[top:bottom,left:right]=selected[top:bottom,left:right]
        for index,loop in enumerate(contours(domain)):
            outline=simplify(loop);n=len(outline)
            tops=[point(953+x,844+y,thickness) for x,y in outline]
            vertices=[Vector((p.x,p.y,0)) for p in tops]+tops
            faces=[tuple(reversed(range(n))),tuple(range(n,2*n))]+[(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)]
            mesh(scene,f'{name} island {index}',vertices,faces,paint,gray,box,group)
            records.append(dict(name=name,island=index,source_region=[left,top,right,bottom],thickness=thickness,outline=outline.tolist()))
    finish(scene,gray,dest,box,dict(status='Private source-traced detached wood; review pending',asset_id=group,source_frame=frame,regions=records,
        limitations=['Only scoped three wood regions; all other source pixels remain unassigned by this candidate.',
                    'Boundary follows opaque native wood with <=0.4px simplification; thickness inferred.',
                    'No cart body, fence, wheels, shadows or barrel changed.']))
    audit(dest)


def main():
    source=json.loads((OUT/'state-target-evidence/south-cart/manifest.json').read_text())
    acquire()
    try:
        if '--scraps-only' not in sys.argv:segmented(source)
        traced_scraps(source)
    finally:release()

if __name__=='__main__':main()
