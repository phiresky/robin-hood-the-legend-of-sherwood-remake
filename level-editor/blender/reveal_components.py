"""Explicit component selectors for a reviewed revealed projection pass."""


def filter_receivers(objects, selectors=None, *, available_objects=None):
    """Restrict named source nodes to reviewed components within one layer."""
    objects=list(objects)
    if not selectors:return objects
    catalog=list(available_objects) if available_objects is not None else objects
    permitted={}
    for selector in selectors:
        if set(selector)!={'source_node','projection_components','patch_id'}:
            raise ValueError('Receiver selector requires source_node, projection_components and patch_id')
        node=selector['source_node'];components=selector['projection_components']
        if not isinstance(node,str) or not node or node in permitted or not isinstance(components,list) or not components:
            raise ValueError('Invalid or duplicate receiver selector')
        selected=set()
        for component in components:
            if not isinstance(component,str) or not component:raise ValueError('Empty receiver component')
            matches=[o for o in catalog if o.get('source_node')==node and o.get('projection_component')==component]
            if len(matches)!=1 or matches[0].get('reveal_component_patch_id')!=selector['patch_id']:
                raise ValueError('Receiver selector does not identify one reviewed patch component')
            selected.add(matches[0])
        permitted[node]=selected
    return [o for o in objects if o.get('source_node') not in permitted or o in permitted[o.get('source_node')]]


def filter_occluders(objects, selectors=None, *, projection_label, available_objects=None):
    """Exclude only named cover components; never mutate scene visibility.

    Empty selectors preserve historical source-node-only projection exactly.
    Available objects may include hidden components when reviewing an already
    revealed scene, but each selector must still identify exactly one component.
    """
    objects=list(objects)
    if not selectors:
        return objects
    if not projection_label.startswith('interior-'):
        raise ValueError('Cover component exclusion requires an interior projection label')
    patch=projection_label.removeprefix('interior-')
    catalog=list(available_objects) if available_objects is not None else objects
    excluded=set()
    for selector in selectors:
        if set(selector)!={'source_node','projection_component','patch_id'}:
            raise ValueError('Cover selector requires source_node, projection_component and patch_id')
        if selector['patch_id']!=patch or any(not isinstance(v,str) or not v for v in selector.values()):
            raise ValueError('Cover selector does not match the projection patch')
        matches=[o for o in catalog if o.get('source_node')==selector['source_node']
                 and o.get('projection_component')==selector['projection_component']]
        if len(matches)!=1:
            raise ValueError(f'Cover selector must identify one component: {selector}, found {len(matches)}')
        obj=matches[0]
        if obj.get('reveal_component_role')!='removable-cover' or obj.get('reveal_component_patch_id')!=patch:
            raise ValueError('Selected component is not an authored cover for this patch')
        excluded.add(obj)
    return [o for o in objects if o not in excluded]
