"""Record inspected trunk associations without changing in-flight catalogs."""
import hashlib
import json
from pathlib import Path
from catalog import OUT

WOOD = {
    0:[30],1:[29],2:[25],3:[],4:[10],5:[31,32],6:[26,27,28],7:[40],
    8:[33],9:[34,35,36],10:[72],11:[37],12:[38,39],13:[41],
    14:[43,44,45,66],15:[46,47],16:[48,49],17:[42],18:[52,53],
    19:[50,51],20:[82],21:[73],22:[75],23:[61],24:[64,65],25:[74],
    71:[62,63],
}


def main():
    inventory=json.loads((OUT/'source-survey/inventory.json').read_text())
    masks={row['index']:row for row in inventory['masks']}
    all_parts=[part for group in WOOD.values() for part in group]
    if len(all_parts)!=len(set(all_parts)):raise ValueError('Duplicate proposed wood ownership')
    groups=[]
    for mask,parts in WOOD.items():
        row=masks[mask]
        x,y=row['box_top_left'];width,height=row['box_size']
        edge=[]
        if x<=0:edge.append('west')
        if y<=0:edge.append('north')
        if x+width>=1408:edge.append('east')
        if y+height>=960:edge.append('south')
        groups.append(dict(id=f'croisement01-tree-{mask:02}',wood_mask=mask,
            native_parts=parts,mask_sha256=row['mask_sha256'],map_edges=edge,
            status='grouping proposal; geometry/crown/source-wood split incomplete',
            scenery_required=not parts,
            evidence=f'source-survey/context-{mask:03}.png and native volume-footprint survey',
            limitations=['Native wood masks can include static leaves or distant artwork; source segmentation still required.',
                         'Visible narrow trunk does not establish crown width; hidden crown and out-of-map branches require an explicit inference.']))
    report=dict(source_sha256=inventory['source_sha256'],groups=groups,
        native_wood_parts=len(all_parts),logical_trees=len(groups),
        missing_obstacle_tree_masks=[3],
        canopy_domains=list(range(93,101)),
        additional_crown_note='Canopy domains 98–100 contain small foreground trees/clumps without an observed matching trunk; do not attach them to nearby stumps merely by overlap.',
        authority='Native masks, all nine mask/context sheets, three volume surveys and full-map source inspected. This is not a canonical catalog or geometry approval.')
    (OUT/'source-survey/tree-groups-proposal.json').write_text(json.dumps(report,indent=2)+'\n')
    print(len(groups),'tree groups proposed,',report['native_wood_parts'],'native wood parts')


if __name__=='__main__':main()
