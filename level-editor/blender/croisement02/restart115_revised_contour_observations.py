"""Recover own-frame RGB for the explicitly revised fixed butterfly contour."""
import argparse
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
from matplotlib.path import Path as Polygon
from scipy.spatial import ConvexHull
from scipy.spatial.transform import Rotation
import restart64_butterfly07_source_ownership as ownership
from restart21_butterfly07_geometry_v2 import geometry, WING, HINGES


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--fit',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    args.output.mkdir(exist_ok=False)
    fit=json.loads(args.fit.read_text())
    scale=fit['fixed_uniform_scale']
    outline=np.asarray(fit['fixed_wing_outline_scaled'])
    assert outline[:,0].min()>=0 and outline[:,0].max()<=8*scale
    assert outline[:,1].min()>=-7*scale and outline[:,1].max()<=6*scale
    def scaled_geometry(p):
        body,wings=geometry(p)
        return body*scale,[w*scale for w in wings]
    ownership.geometry=scaled_geometry
    ownership.HINGES=np.array(HINGES)*scale
    rows=[ownership.prepare(row) for row in fit['rows']]
    assert len(rows)==99
    w,h=64,96
    yy,xx=np.mgrid[:h,:w]
    uv=np.c_[(xx.ravel()+.5)/w*8*scale,((yy.ravel()+.5)/h*13-7)*scale]
    domain=Polygon(outline[:,:2]).contains_points(uv)
    best=np.zeros((4,h*w));rgb=np.full((4,h*w,4),255,np.uint8);rgb[:,:,:3]=112
    phase=np.full((4,h*w),-1,int);source_xy=np.full((4,h*w,2),-1,int)
    for c in rows:
        img=c['rgba'];body,_=scaled_geometry(c['row']['parameters'])
        polygon=Polygon((body[:,:2]+c['shift'])[ConvexHull(body[:,:2]).vertices])
        for m in c['maps']:
            xy=uv@m['matrix'][:2,:2].T+m['offset'][:2]+c['shift']
            ix,iy=np.floor(xy).astype(int).T
            valid=domain&(ix>=0)&(ix<img.shape[1])&(iy>=0)&(iy<img.shape[0])&~polygon.contains_points(xy)
            other=c['maps'][1-m['side']]
            otheruv=(xy-c['shift']-other['offset'][:2])@np.linalg.inv(other['matrix'][:2,:2]).T
            ownz=uv@m['matrix'][2,:2]+m['offset'][2]
            otherz=otheruv@other['matrix'][2,:2]+other['offset'][2]
            valid&=~(Polygon(outline[:,:2]).contains_points(otheruv)&(otherz>ownz))
            ids=np.flatnonzero(valid);ids=ids[img[iy[ids],ix[ids],3]>0]
            weight=abs(np.linalg.det(m['matrix'][:2,:2]))**2
            d=m['side']*2+m['face'];ids=ids[weight>best[d,ids]]
            best[d,ids]=weight;rgb[d,ids,:3]=img[iy[ids],ix[ids],:3]
            phase[d,ids]=c['row']['phase'];source_xy[d,ids]=np.c_[ix[ids],iy[ids]]
    surfaces=[]
    def save_surface(name,colors,phases,coordinates,shape,valid_domain):
        known=phases>=0
        # Independent direct byte readback of every assigned texture observation.
        for p in np.unique(phases[known]):
            ids=np.flatnonzero(phases==p);xy=coordinates[ids];img=rows[int(p)]['rgba']
            assert np.array_equal(colors[ids,:3],img[xy[:,1],xy[:,0],:3])
            assert np.all(img[xy[:,1],xy[:,0],3]>0)
        image=args.output/f'{name}.png'
        Image.fromarray(colors.reshape(*shape,4)).save(image)
        Image.fromarray((known.reshape(shape)*255).astype('uint8')).save(args.output/f'{name}-known.png')
        np.savez_compressed(args.output/f'{name}-provenance.npz',phase=phases.reshape(shape),source_xy=coordinates.reshape(*shape,2))
        surfaces.append(dict(name=name,known_texels=int((known&valid_domain).sum()),unknown_surface_texels=int((~known&valid_domain).sum()),image_sha256=ownership.sha(image),exact_rgb_readback=True))
    for d in range(4):
        save_surface(f'wing-{d}',rgb[d],phase[d],source_xy[d],(h,w),domain)
    w,h=64,32;yy,xx=np.mgrid[:h,:w];polar=(yy.ravel()+.5)/h*np.pi;az=(xx.ravel()+.5)/w*2*np.pi
    radii=np.array([.5,2.7,.5])*scale
    points=np.c_[radii[0]*np.sin(polar)*np.cos(az),radii[1]*np.cos(polar),radii[2]*np.sin(polar)*np.sin(az)]
    normals=points/(radii**2);normals/=np.linalg.norm(normals,axis=1)[:,None]
    best=np.zeros(h*w);rgb=np.full((h*w,4),255,np.uint8);rgb[:,:3]=112
    phase=np.full(h*w,-1,int);source_xy=np.full((h*w,2),-1,int)
    for c in rows:
        rot=Rotation.from_euler('xyz',c['row']['parameters'][:3],degrees=True).as_matrix()
        v=points@rot.T;n=normals@rot.T;xy=v[:,:2]+c['shift'];ix,iy=np.floor(xy).astype(int).T;img=c['rgba']
        valid=(n[:,2]>0)&(ix>=0)&(ix<img.shape[1])&(iy>=0)&(iy<img.shape[0])
        for m in c['maps']:
            winguv=(xy-c['shift']-m['offset'][:2])@np.linalg.inv(m['matrix'][:2,:2]).T
            z=winguv@m['matrix'][2,:2]+m['offset'][2]
            valid&=~(Polygon(outline[:,:2]).contains_points(winguv)&(z>=v[:,2]))
        ids=np.flatnonzero(valid);ids=ids[img[iy[ids],ix[ids],3]>0];ids=ids[n[ids,2]>best[ids]]
        best[ids]=n[ids,2];rgb[ids,:3]=img[iy[ids],ix[ids],:3];phase[ids]=c['row']['phase'];source_xy[ids]=np.c_[ix[ids],iy[ids]]
    save_surface('body',rgb,phase,source_xy,(h,w),np.ones(h*w,bool))
    # Outline-on-own-art evidence for anatomy review, retaining every source phase.
    sheet=Image.new('RGB',(11*144,9*144),'#303030');draw=ImageDraw.Draw(sheet)
    for c in rows:
        p=c['row']['phase'];im=Image.fromarray(c['rgba']);canvas=Image.new('RGBA',(24,24),(40,40,40,255));ox=(24-im.width)//2;oy=(24-im.height)//2;canvas.alpha_composite(im,(ox,oy));canvas=canvas.resize((120,120),Image.Resampling.NEAREST)
        pen=ImageDraw.Draw(canvas)
        for m,color in zip(c['maps'],('#ff66cc','#22ddff')):
            xy=outline[:,:2]@m['matrix'][:2,:2].T+m['offset'][:2]+c['shift']+[ox,oy]
            pts=[tuple(v*5) for v in xy];pen.line(pts+[pts[0]],fill=color,width=1)
        sx,sy=(p%11)*144,(p//11)*144;sheet.paste(canvas,(sx,sy+24));r=fit['rows'][p]
        draw.text((sx+2,sy+3),f'{p} miss{r["missing"]} bright{r["bright_missing"]}',fill='white')
    sheet.save(args.output/'own-source-outline-sheet.png')
    report=dict(status='PRIVATE_OWN_SOURCE_SURFACE_OBSERVATIONS_NOT_APPROVED',animation_index=fit['animation_index'],fit_sha256=ownership.sha(args.fit),recipe_sha256=ownership.sha(Path(__file__)),source_frames=[r['source'] for r in fit['rows']],surfaces=surfaces,uv_domain=[0,8*scale,-7*scale,6*scale],limitations=['Own-frame RGB only. No opposite face, other butterfly, interpolation or synthesis.','Geometry and pose ownership are inferred and still require refinement; exact pixel copying is not proof of correct physical surface identity.','Mixed and subpixel body observations remain ambiguous; unknown surfaces are gray.','This is anatomy/texture evidence, not a saved-model or continuous contact proof.'])
    (args.output/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(surfaces),flush=True)


if __name__=='__main__':
    main()
