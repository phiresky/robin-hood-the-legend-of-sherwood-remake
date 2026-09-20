"""Refine the north-curtain timber shed, its side annex and chimney.

The source shows a boarded lean-to with a dark doorway and a smaller annex.
Preserve its measured roof silhouette; make eaves, the doorway and flue solid
geometry. Hidden rear surfaces cannot be reconstructed from the single view.
"""
import importlib.util
from pathlib import Path
import bpy
from mathutils import Matrix, Vector

TAG = 'north_curtain_timber_shed_refinement'
RECIPE = 'north-curtain-shed-door-flue-and-eaves-v1'
IDS = (70, 71, 72)


def _helpers():
    path = Path(__file__).with_name('derby_asset_east_courtyard_south_shelter.py')
    spec = importlib.util.spec_from_file_location('derby_south_lean_to_builder', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    mesh, prism = module._builder()
    return mesh, prism, module._chimney


def _main(source, mesh_builder, prism_builder):
    world = [source.matrix_world @ v.co for v in source.data.vertices]
    top = [world[i].copy() for i in (14, 15, 12, 13)]
    under = [v - Vector((0, 0, 2.2)) for v in top]
    roof = prism_builder(source, top, under, [6]*6, 'North curtain shed / boarded roof thickness')
    front_a, front_b, back_b, back_a = under
    def point(u, depth, z):
        value = front_a.lerp(front_b, u).lerp(back_a.lerp(back_b, u), depth)
        value.z = z
        return value
    a,b,c,d = (point(0,0,0),point(1,0,0),point(1,1,0),point(0,1,0))
    lo,hi,sill,lintel,depth=.46,.65,1.0,47.0,.16
    opening = [point(lo,0,sill),point(lo,0,lintel),point(hi,0,lintel),point(hi,0,sill)]
    inset = [point(lo,depth,sill),point(lo,depth,lintel),point(hi,depth,lintel),point(hi,depth,sill)]
    vertices=[a,b,c,d,front_a,front_b,back_b,back_a]+opening+inset
    faces=[(0,8,9,10,11,1,5,4),(0,1,11,8),(8,12,13,9),
           (9,13,14,10),(10,14,15,11),(8,11,15,12),(12,15,14,13),
           (4,5,6,7),(0,4,7,3),(1,2,6,5),(3,7,6,2),(0,3,2,1)]
    walls=mesh_builder(source,vertices,faces,[2,2,2,2,2,2,2,6,4,0,0,0],
                       'North curtain shed / closed walls with recessed doorway')
    return [('boarded roof with solid eaves',roof),('walls with recessed doorway',walls)]


def _annex(source, prism_builder):
    world=[source.matrix_world@v.co for v in source.data.vertices]
    top=[world[i].copy() for i in (16,17,18,19)]
    under=[v-Vector((0,0,2)) for v in top]
    bottom=[Vector((v.x,v.y,0)) for v in top]
    return [('side annex roof',prism_builder(source,top,under,[8]*6,'North curtain shed / annex roof')),
            ('side annex closed walls',prism_builder(source,under,bottom,[8,0,0,6,4,2],
                                                    'North curtain shed / closed annex walls'))]


def refine():
    working=bpy.data.collections['Derby Working']
    existing=[o for o in working.objects if o.get(TAG)==RECIPE and not o.hide_render]
    if existing:
        counts={i:sum(o.get('source_node')==f'building-{i:03}' for o in existing) for i in IDS}
        if len(existing)!=5 or counts!={70:1,71:2,72:2}:
            raise ValueError('Incomplete north-curtain timber shed refinement')
        return {'status':'existing','objects':[o.name for o in existing]}
    bpy.context.view_layer.update()
    mesh_builder,prism_builder,chimney_builder=_helpers()
    report=[]
    names={70:'Masonry chimney',71:'Side annex roof and walls',72:'Timber shed roof and doorway'}
    for number in IDS:
        candidates=[o for o in working.objects if o.type=='MESH' and not o.hide_render
                    and o.get('source_node')==f'building-{number:03}']
        if len(candidates)!=1:raise ValueError(f'Expected one timber shed source {number}')
        source=candidates[0]
        pieces=([('hollow masonry chimney',chimney_builder(source,mesh_builder,(0,6,4,2)))] if number==70
                else _annex(source,prism_builder) if number==71 else _main(source,mesh_builder,prism_builder))
        for label,(mesh,defects) in pieces:
            obj=bpy.data.objects.new('East Bailey North Curtain Timber Shed / '+label,mesh)
            working.objects.link(obj);obj.parent=source.parent
            bpy.context.view_layer.update();obj.matrix_world=Matrix.Identity(4)
            for key,value in source.items():obj[key]=value
            obj[TAG]=RECIPE;obj['refinement_recipe']=RECIPE
            obj['asset_name']='East Bailey North Curtain Timber Shed';obj['part_name']=names[number]
            report.append({'source_node':source['source_node'],'piece':label,'faces':len(mesh.polygons),'validation':defects})
        source.hide_render=source.hide_viewport=True
    bpy.context.view_layer.update()
    return {'status':'created','objects':report,
            'audit':{'070':'Chimney capped by a recessed eight-unit flue, original silhouette retained',
                     '071':'Smaller boarded annex; roof thickness and wall seams repaired',
                     '072':'Main boarded roof thickness; front painted doorway recessed; closed supporting shell'},
            'catalog_correction':{'asset':'East Bailey North Curtain Timber Shed',**{str(k):v for k,v in names.items()}},
            'uncertainty':'Recess depths and board thickness are inferred; exact angle of the painted open door is not recoverable',
            'remaining':'A separate angled door leaf and individual roof boards remain unresolved; hidden rear walls retain fallback atlas'}


if __name__=='__main__':result=refine()
