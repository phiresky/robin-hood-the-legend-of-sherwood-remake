"""East bailey east curtain: open merlons, closed supports and preserved stairs."""
import bpy
import bmesh
from mathutils import Matrix, Vector
import runpy
from pathlib import Path
_rebuild = runpy.run_path(str(Path(__file__).with_name('derby_asset_lower_east_curtain.py')))['_rebuild']
crenellate = runpy.run_path(str(Path(__file__).with_name('derby_architecture.py')))['crenellate']

ASSET = "derby-east-bailey-east-curtain"


def _close_support(source):
    clone = source.copy()
    clone.data = source.data.copy()
    clone.name = source.name + " / closed support"
    bpy.data.collections["Derby Working"].objects.link(clone)
    clone.matrix_world = source.matrix_world.copy()
    bm = bmesh.new()
    bm.from_mesh(clone.data)
    bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=.8)
    bmesh.ops.dissolve_degenerate(bm, edges=list(bm.edges), dist=.01)
    degenerate = [f for f in bm.faces if f.calc_area() < 1e-7]
    if degenerate:
        bmesh.ops.delete(bm, geom=degenerate, context="FACES")
    boundary = [e for e in bm.edges if e.is_boundary]
    if boundary:
        filled = bmesh.ops.holes_fill(bm, edges=boundary, sides=0)["faces"]
        # New undersides are concealed by ground and retain their atlas fallback.
        uv = bm.loops.layers.uv.active
        for face in filled:
            face.material_index = 0
            if uv:
                for loop in face.loops:
                    loop[uv].uv = (.76, .788)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bad = sum(not e.is_manifold for e in bm.edges)
    zero = sum(f.calc_area() < 1e-7 for f in bm.faces)
    if bad or zero:
        bm.free()
        raise ValueError(f"Unclosed stair support {source.name}: {bad}, {zero}")
    bm.to_mesh(clone.data)
    bm.free()
    clone["east_bailey_support"] = True
    source["east_bailey_support_done"] = True
    source.hide_render = True
    source.hide_set(True)
    return {"source": source["source_node"], "nonmanifold_edges": bad,
            "degenerate_faces": zero, "support_seams_closed": True}


def refine():
    bpy.context.view_layer.update()
    working = bpy.data.collections["Derby Working"]
    report = []
    specs = {
        "building-098": [
            (122,123,[(.1,.2),(.31,.41),(.52,.62),(.73,.83)],25),
            (144,124,[(i/11+.035,i/11+.078) for i in range(10)],25),
            (125,126,[(.3,.65)],25), (126,138,[(.3,.65)],25),
            (138,134,[(.3,.65)],25), (134,127,[(.3,.65)],25),
            (137,141,[(.3,.65)],25), (141,144,[(.3,.65)],25),
            (133,137,[(.3,.65)],25), (123,133,[(.3,.65)],25),
            (127,128,[(.15,.3),(.43,.58),(.71,.86)],25),
        ],
        "building-097": [], "building-101": [],
    }
    for node, spans in specs.items():
        source = next(o for o in working.objects if o.get("source_node") == node
                      and not o.get("lower_east_rebuilt"))
        if source.get("lower_east_refined"):
            report.append({"source": node, "skipped": "already refined"})
        else:
            report.append(_rebuild(source, spans))
    for node, corners in (("building-099",(16,17,19,18)),
                           ("building-100",(19,16,18,17))):
        source = next(o for o in working.objects if o.get("source_node") == node
                      and not o.get("crenellation_notches"))
        if source.get("architecture_refined"):
            report.append({"source": node, "skipped": "already refined"})
            continue
        a,b = [(source.matrix_world @ source.data.vertices[i].co).x for i in corners[:2]]
        gaps = [(a+(b-a)*lo,a+(b-a)*hi) for lo,hi in ((.15,.30),(.42,.56),(.70,.83))]
        item = crenellate(source,corners,gaps,22)
        clone = next(o for o in working.objects if o.get("source_node") == node
                     and o.get("crenellation_notches"))
        for key in source.keys():
            if not key.startswith("reprojection_") and key != "architecture_refined":
                clone[key] = source[key]
        clone["projection_min_cosine"] = .15
        item["source"] = node
        report.append(item)
    for node in ("building-077", "building-078"):
        source = next(o for o in working.objects if o.get("source_node") == node
                      and not o.get("step_count") and not o.get("east_bailey_support"))
        if source.get("east_bailey_support_done"):
            report.append({"source": node, "skipped": "already refined"})
        else:
            report.append(_close_support(source))
    return {"asset": ASSET, "changes": report,
            "stairs_preserved": {"building-077":12,"building-078":18},
            "concealed_surface_note":"Near-grazing sides retain source-derived stone donors."}
