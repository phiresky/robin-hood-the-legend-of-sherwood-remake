"""Private finite terminal cart geometry with independently traced source domains."""
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
from restart3_cart_cargo_debris import *
from restart3_terminal_render import finish_terminal as finish
from PIL import ImageDraw
from scenery_geometry import Mesh
from mathutils.geometry import tessellate_polygon


def main():
    dest=OUT/'restart3-north-cart/terminal-physical-v12';dest.mkdir(exist_ok=False)
    source=json.loads((OUT/'state-target-evidence/north-cart/manifest.json').read_text());frame=source['parts'][0]['frames'][-1]
    assert sha(frame['image'])==frame['image_sha256']
    rgba=np.array(Image.open(frame['image']).convert('RGBA'));box=[1202,220,1454,362]
    scene,gray=start();labels=np.zeros(rgba.shape[:2],dtype=np.uint8);records=[];requested_masks={};paints={}
    group='croisement02-north-cart-terminal-physical'
    def claim(name,polygon):
        mask=Image.new('L',(252,142));ImageDraw.Draw(mask).polygon(polygon,fill=255)
        requested=(np.array(mask)>0)&(rgba[:,:,3]>0);owned=requested&(labels==0);index=len(records)+1;labels[owned]=index
        image=rgba.copy();image[~owned,3]=0;path=dest/(name+'.png');Image.fromarray(image).save(path)
        records.append(dict(name=name,polygon=polygon,accepted_pixels=int(owned.sum()),overlap_with_prior=int((requested&~owned).sum()),image=str(path),sha256=sha(path)))
        requested_masks[name]=requested;paints[name]=material(path);return paints[name]
    def solid(name,vertices,faces,paint):
        obj=mesh(scene,name,vertices,faces,paint,gray,box,group);obj['source_state']='north cart terminal; private geometry';return obj
    def prism(name,polygon,height,thickness,paint,direction=None):
        front=[point(1202+x,220+y,height(x,y)) for x,y in polygon]
        back=[p-(direction if direction is not None else Vector((0,0,1)))*thickness for p in front];n=len(front)
        # Explicit triangles preserve the traced concave openings.
        triangles=tessellate_polygon([front]);ids={tuple(v):i for i,v in enumerate(front)}
        faces=[tuple(tri) if isinstance(tri[0],int) else tuple(ids[tuple(v)] for v in tri) for tri in triangles]
        faces += [tuple(n+i for i in reversed(f)) for f in faces.copy()]
        faces += [(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)]
        return solid(name,front+back,faces,paint)
    roof=[(82,43),(86,36),(91,31),(94,24),(99,27),(106,27),(114,23),(127,17),(140,8),(143,4),(146,2),(152,5),(158,11),(162,20),(165,31),(163,37),(153,40),(137,41),(121,40),(107,42),(94,45)]
    rp=claim('roof-owned',roof);rh=lambda x,y:80+.17*(x-82)+.12*(43-y)
    prism('Torn finite canopy shell',roof,rh,1.2,rp)
    for name,poly in [
      ('front-drape',[(80,45),(91,45),(99,43),(112,42),(111,67),(106,78),(98,84),(97,93),(88,92),(79,87)]),
      ('rear-drape',[(133,41),(145,41),(163,37),(160,62),(157,70),(151,75),(139,71),(133,64)])]:
        paint=claim(name+'-owned',poly)
        prism(name,poly,lambda x,y:max(34 if name=='front-drape' else 26,rh(x,43-.10*(x-82))-(y-(43-.10*(x-82)))/COS+3*max(0,min(1,(y-45)/6))),.65,paint,RAY)
    panel=[(55,93),(56,52),(62,37),(64,32),(68,25),(72,30),(77,35),(81,36),(85,29),(90,18),(93,21),(91,43),(86,54),(83,75),(86,98),(69,98)]
    pp=claim('broken-front-panel-owned',panel)
    bh=lambda x,y:3+21*max(0,min(1,(x-27)/62))-17*max(0,min(1,(x-153)/9))*max(0,min(1,(y-73)/16))
    ph=lambda x,y:max(bh(x,y)+.5,90-(y-27-.25*(x-68))/COS-12*max(0,min(1,(y-31)/7))*max(0,min(1,(x-83)/7)))
    prism('Jagged standing front panel',panel,ph,2.5,pp,RAY)
    side=[(109,43),(131,41),(134,58),(138,69),(126,82),(126,102),(117,106),(98,110),(93,94),(102,85),(110,75)]
    sp=claim('cabin-side-owned',side)
    prism('Retained cabin side behind drapes',side,lambda x,y:max(2.0,rh(x,43-.10*(x-82))-(y-(43-.10*(x-82)))/COS-1.2),2,sp,RAY)
    # A true broken U-shaped bed leaves the source opening beneath the rear drape empty.
    bed=[(25,99),(43,90),(69,78),(119,65),(150,69),(158,76),(172,89),(171,96),(163,97),(154,89),(155,75),(137,77),(127,83),(127,103),(116,109),(77,115),(33,119),(28,114)]
    bp=claim('broken-bed-owned',bed)
    bh=lambda x,y:3+21*max(0,min(1,(x-27)/62))-17*max(0,min(1,(x-153)/9))*max(0,min(1,(y-73)/16))
    prism('Broken notched bed',bed,bh,3/SIN,bp,RAY)
    timber=[
      ('diagonal-front-plank',[(21,69),(24,69),(25,67),(29,70),(32,70),(36,72),(56,89),(57,99),(46,94),(28,79)],lambda x,y:bh(x,y)+2.5),
      ('left-jagged-shaft',[(0,89),(7,89),(10,88),(13,90),(16,90),(18,92),(23,94),(27,94),(33,93),(38,94),(43,97),(39,102),(29,103),(23,100),(18,98),(15,97),(11,95),(8,95),(8,93),(3,92),(0,92)],lambda x,y:1.5+.13*x),
      ('upper-broken-shaft',[(20,82),(25,78),(39,78),(44,74),(49,78),(37,83),(30,87),(22,86)],lambda x,y:bh(x,y)+2.5),
      ('raised-foretimber',[(31,93),(34,87),(35,81),(34,76),(38,68),(44,64),(48,64),(49,59),(53,59),(58,58),(58,65),(50,70),(48,75),(46,79),(49,86),(43,90),(39,95)],ph),
      ('inner-broken-brace',[(47,75),(52,71),(57,70),(59,74),(57,80),(54,82),(53,86),(49,86),(48,81)],ph),
      ('rear-facing-splinter',[(41,54),(47,52),(56,57),(59,62),(52,64),(49,58)],ph)]
    for name,poly,h in timber:
        p=claim(name+'-owned',poly);prism(name,poly,h,1.3,p,RAY)
    # The upright near wheel is a finite annulus and spokes, not a textured disk.
    wheelpoly=[(128,98),(131,102),(133,111),(132,120),(128,127),(122,132),(114,134),(107,131),(102,127),(99,121),(101,115),(104,108),(108,104),(117,102),(124,99)]
    wp=claim('upright-wheel-owned',wheelpoly);u=Vector((math.cos(.8113042059988309),math.sin(.8113042059988309),0));v=Vector((u.y,-u.x,0));Z=Vector((0,0,1))
    center=point(1320.6394510139792,335.33766190067347,19.72279719558539);n=48
    def wheel(name,c,a,b,normal,radius,inner,depth,paint,phase=0,spoke_radius=.9):
        m=Mesh()
        for off,r in [(-depth/2,radius),(-depth/2,inner),(depth/2,radius),(depth/2,inner)]:
            for i in range(n):m.vertices.append(tuple(c+normal*off+(a*math.cos(i*math.tau/n)+b*math.sin(i*math.tau/n))*r))
        for i in range(n):
            j=(i+1)%n
            for aa,bb in [(0,n),(2*n,3*n),(0,2*n),(n,3*n)]:m.faces.append((aa+i,aa+j,bb+j,bb+i))
        solid(name+' rim',m.vertices,m.faces,paint)
        m=Mesh();hub_half=3 if name=='Near upright wheel' else min(3,depth/2);m.tube(c-normal*hub_half,c+normal*hub_half,3.3,n=12)
        for i in range(10):m.tube(c,c+(a*math.cos(i*math.tau/10+phase)+b*math.sin(i*math.tau/10+phase))*(inner+.5),spoke_radius,n=6)
        solid(name+' spokes and hub',m.vertices,m.faces,paint)
    wheel('Near upright wheel',center,u,Z,v,19.72279719558539,14.80848723565496,3,wp,phase=.600288685951353,spoke_radius=1.592369924515233)
    flatpoly=[(158,92),(165,90),(170,92),(175,96),(176,102),(173,105),(166,109),(157,107),(155,103),(155,96)]
    fp=claim('fallen-wheel-owned',flatpoly)
    wheel('Displaced ground wheel',point(1368,320,2),Vector((1,0,0)),Vector((0,1,0)),Z,11.2,7.8,4,fp)
    # The source frame contains a smaller-looking displaced ring; its radius is inferred independently.
    def beam(name,a,b,r,paint):
        m=Mesh();m.tube(a,b,r,n=8);return solid(name,m.vertices,m.faces,paint)
    beam('Near axle retained stub',center-v*2,center-v*38,2.2,bp)
    for i,(x,y) in enumerate([(84,43),(159,36),(145,7)]):
        top=point(1202+x,220+y,rh(x,y)-1);bottom=point(1202+x,220+89,bh(x,89)) if i==1 else Vector((top.x,top.y,24));beam('Retained canopy post '+str(i),bottom,top,1.35,pp if i==0 else bp)
    boxpoly=[(230,102),(251,103),(251,132),(247,136),(222,135),(221,131),(223,121),(226,119),(227,114)]
    cp=claim('box-owned',boxpoly)
    top=[point(1202+x,220+y,20) for x,y in [(230,103),(256,104),(255,119),(228,118)]];bottom=[Vector((p.x-6,p.y,0)) for p in top]
    solid('Detached finite box',bottom+top,[(0,3,2,1),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)],cp)
    # Only the already traced post/roof overlap is eligible for this material split.
    rgb=rgba[:,:,:3].astype(float)
    post_overlap=requested_masks['roof-owned']&requested_masks['broken-front-panel-owned']&(rgb[:,:,0]>rgb[:,:,1]*1.1)&(rgb[:,:,1]>rgb[:,:,2]*1.1)
    requested_masks['roof-owned'] &= ~post_overlap
    # The orange timber tongue continues across the upper ring's traced bounding domain.
    yy=np.indices(labels.shape)[0]
    tongue_overlap=requested_masks['fallen-wheel-owned']&requested_masks['broken-bed-owned']&(yy<=97)
    requested_masks['fallen-wheel-owned'] &= ~tongue_overlap
    write_json(dest/'tongue-ring-role.json',dict(source_sha256=frame['image_sha256'],pixels=int(tongue_overlap.sum()),rule='Traced broken-bed overlap above the independently inspected ring rim at native y98 belongs to the continuous orange timber tongue.'))
    Image.fromarray((tongue_overlap*255).astype(np.uint8)).save(dest/'tongue-ring-role.png')
    write_json(dest/'post-roof-role.json',dict(source_sha256=frame['image_sha256'],pixels=int(post_overlap.sum()),rule='Brown pixels within the independently traced front-post and roof overlap belong to the structural post; no new source domain is granted.'))
    Image.fromarray((post_overlap*255).astype(np.uint8)).save(dest/'post-roof-role.png')
    labels[:]=0
    priority=['roof-owned','front-drape-owned','rear-drape-owned','upright-wheel-owned','fallen-wheel-owned','diagonal-front-plank-owned','left-jagged-shaft-owned','upper-broken-shaft-owned','raised-foretimber-owned','inner-broken-brace-owned','rear-facing-splinter-owned','broken-front-panel-owned','cabin-side-owned','broken-bed-owned','box-owned']
    for name in priority:
        index=next(i for i,row in enumerate(records,1) if row['name']==name);row=records[index-1];requested=requested_masks[name];owned=requested&(labels==0);labels[owned]=index
        image=rgba.copy();image[~owned,3]=0;Image.fromarray(image).save(row['image']);row.update(accepted_pixels=int(owned.sum()),overlap_with_prior=int((requested&~owned).sum()),sha256=sha(Path(row['image'])))
        for node in paints[name].node_tree.nodes:
            if node.type=='TEX_IMAGE':node.image.reload();node.image.pack()
    wood_names={'broken-front-panel-owned','cabin-side-owned','broken-bed-owned','diagonal-front-plank-owned','left-jagged-shaft-owned','upper-broken-shaft-owned','raised-foretimber-owned','inner-broken-brace-owned','rear-facing-splinter-owned'}
    wood_pool=np.isin(labels,[i for i,row in enumerate(records,1) if row['name'] in wood_names])
    for index,row in enumerate(records,1):
        domain=rgba.copy();domain[labels!=index,3]=0;dp=dest/(row['name']+'-exclusive.png');Image.fromarray(domain).save(dp);row['exclusive_ownership_image']=str(dp);row['exclusive_ownership_sha256']=sha(dp)
        if row['name'] in wood_names:
            image=rgba.copy();image[~wood_pool,3]=0;Image.fromarray(image).save(row['image']);row['sha256']=sha(Path(row['image']));row['material_source_pool']='Only the union of independently traced structural wood domains; no cloth/wheel/box or unassigned pixels.'
            for node in paints[row['name']].node_tree.nodes:
                if node.type=='TEX_IMAGE':node.image.reload();node.image.pack()
    union=rgba.copy();union[labels==0,3]=0;Image.fromarray(union).save(dest/'source.png');Image.fromarray(labels).save(dest/'ownership-labels.png')
    unknown=(rgba[:,:,3]>0)&(labels==0);Image.fromarray((unknown*255).astype(np.uint8)).save(dest/'unassigned-native.png')
    write_json(dest/'ownership.json',dict(source_sha256=frame['image_sha256'],roles=records,mutually_exclusive=True,assigned_pixels=int((labels>0).sum()),unassigned_opaque_pixels=int(unknown.sum()),limitations=['Hand-traced native material domains are hypotheses at ambiguous joins; unassigned dark regions are not granted solid ownership.']))
    finish(scene,gray,dest,box,dict(status='Private terminal wreck trial; no root/user approval',asset_id=group,source_frame=frame,source_position=[1202,300],ownership_sha256=sha(dest/'ownership.json'),
      limitations=['Terminal endpoint only, no approach/collapse motion claim.','Hidden depths, canopy support, broken bed and displaced wheel size/pose inferred; exact current-ground and connectivity review required.','Detached box dark base may include shadow; no generated textures authorized.','Initial b28b4334 and south-cart candidates unchanged.']))
    write_json(dest/'recipe.json',record_recipe(dest,Path(__file__)))

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
