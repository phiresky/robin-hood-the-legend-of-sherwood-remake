"""Native painted shadow coverage is independent of foliage ownership channels."""


def native_shadow_metadata(material):
    """Correct the legacy cutout tag without changing coverage or adding provenance."""
    if material.get('source_ownership_semantics') or material.get('source_ownership_channel'):
        raise ValueError('Native shadow cannot discard a separate foliage ownership contract')
    material.pop('foliage_physical_opacity', None)
    material.pop('foliage_alpha_cutoff', None)
    material['opacity_semantics'] = 'physical-coverage'


def qualify_native_shadow_export(node, material):
    if node.get('extras', {}).get('source_node') != 'mission-panneau-shadow':
        raise ValueError('Shadow metadata correction requires the native shadow receiver')
    if (material.get('alphaMode') != 'MASK' or material.get('alphaCutoff') != 0.5
            or not material.get('doubleSided')
            or 'KHR_materials_unlit' not in material.get('extensions', {})
            or 'baseColorTexture' not in material.get('pbrMetallicRoughness', {})):
        raise ValueError('Native shadow requires its explicit unlit MASK texture and cutoff')
    native_shadow_metadata(material.setdefault('extras', {}))
