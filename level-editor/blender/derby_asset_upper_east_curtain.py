"""Source-traced openings along the upper east curtain's three parapet runs."""
import bpy
from derby_asset_lower_east_curtain import _rebuild

ASSET = "derby-upper-east-curtain"


def refine():
    bpy.context.view_layer.update()
    working = bpy.data.collections["Derby Working"]
    specs = {
        "building-124": [
            # Eight gaps along the gate-to-bend run, four on the middle
            # diagonal, and two at the keep approach. Existing walk elevation
            # and the end joins remain those of the measured source footprint.
            (33,32,[(center-.0275,center+.0275) for center in
                    (.07,.19,.32,.44,.56,.68,.80,.925)],27),
            (32,39,[(center-.055,center+.055) for center in
                    (.14,.39,.64,.89)],27),
            (39,38,[(.16,.36),(.63,.83)],27),
        ],
        "building-123": [],
    }
    report = []
    for node, spans in specs.items():
        source = next(o for o in working.objects if o.get("source_node") == node
                      and not o.get("lower_east_rebuilt"))
        if source.get("lower_east_refined"):
            report.append({"source":node,"skipped":"already refined"})
        else:
            report.append(_rebuild(source,spans))
    return {"asset":ASSET,"changes":report,
            "review":"Both parts checked against covered artwork; walkway and keep/gate joins preserved.",
            "concealed_surfaces":"Near-grazing stone uses the shared packed masonry donor."}
