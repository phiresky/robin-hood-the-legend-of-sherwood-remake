"""Keep exact original bark shaders while adding reviewed inferred texture.

Build onto original material copies, preserving every existing image and UV.
The generated atlas is selected only where original ownership is unknown and
where the guarded view sampler produced a supported sample.
"""
import hashlib
import numpy as np


def protected_union(known_flags, overlay_alpha):
    """Conservative categorical union; native colors never define ownership."""
    items=[np.asarray(x,dtype=bool) for x in known_flags]
    items += [np.asarray(x)>0 for x in overlay_alpha]
    if not items:raise ValueError('Expected an explicit ownership domain')
    if len({x.shape for x in items})!=1:raise ValueError('Mismatched ownership domains')
    return np.logical_or.reduce(items)


def reachable_images(material):
    roots=[n for n in material.node_tree.nodes if n.type=='OUTPUT_MATERIAL']
    if len(roots)!=1:raise ValueError('Expected one material output')
    pending=roots[:];seen=set();result=[]
    while pending:
        node=pending.pop()
        if node.name in seen:continue
        seen.add(node.name)
        if node.type=='TEX_IMAGE'and node.image:result.append(node)
        pending.extend(link.from_node for socket in node.inputs for link in socket.links)
    return result


def add_generated_layer(original_material, generated_image, generated_uv,
                        generated_support_image, protected_images, *, name):
    """Create a material copy. All source nodes and packed images stay intact.

    protected_images maps packed image digest to a conservative binary known
    mask image, sampled using the exact corresponding source UV. Direct native
    overlays must be included by the caller with their authoritative alpha mask.
    No caller may infer mask permissions from source RGB.
    """
    import bpy
    mat=original_material.copy();mat.name=name;nodes=mat.node_tree.nodes;links=mat.node_tree.links
    output=next(n for n in nodes if n.type=='OUTPUT_MATERIAL');socket=output.inputs['Surface']
    if len(socket.links)!=1:raise ValueError('Unsupported original surface binding')
    original_surface=socket.links[0].from_socket
    sources=reachable_images(mat);protection=[]
    for node in sources:
        if not node.image.packed_file:raise ValueError('Source image is not immutable packed data')
        digest=hashlib.sha256(bytes(node.image.packed_file.data)).hexdigest()
        if digest not in protected_images:raise ValueError('Missing explicit source protection')
        uvlinks=list(node.inputs['Vector'].links)
        if len(uvlinks)!=1 or uvlinks[0].from_node.type!='UVMAP':raise ValueError('Original source UV is not explicit')
        lookup=nodes.new('ShaderNodeTexImage');lookup.image=protected_images[digest];lookup.interpolation='Closest';lookup.extension='CLIP';links.new(uvlinks[0].from_socket,lookup.inputs['Vector']);protection.append(lookup.outputs['Color'])
    # Zero supported generation falls back to the entire original material.
    uv=nodes.new('ShaderNodeUVMap');uv.uv_map=generated_uv
    tex=nodes.new('ShaderNodeTexImage');tex.image=generated_image;links.new(uv.outputs['UV'],tex.inputs['Vector'])
    support=nodes.new('ShaderNodeTexImage');support.image=generated_support_image;support.interpolation='Closest';support.extension='CLIP';links.new(uv.outputs['UV'],support.inputs['Vector'])
    inverse=nodes.new('ShaderNodeMath');inverse.operation='SUBTRACT';inverse.inputs[0].default_value=1;links.new(support.outputs['Color'],inverse.inputs[1]);keep=inverse.outputs[0]
    for value in protection:
        maximum=nodes.new('ShaderNodeMath');maximum.operation='MAXIMUM';links.new(keep,maximum.inputs[0]);links.new(value,maximum.inputs[1]);keep=maximum.outputs[0]
    if original_surface.type=='SHADER':
        generated=nodes.new('ShaderNodeEmission');links.new(tex.outputs['Color'],generated.inputs['Color']);mix=nodes.new('ShaderNodeMixShader');links.new(keep,mix.inputs[0]);links.new(generated.outputs[0],mix.inputs[1]);links.new(original_surface,mix.inputs[2])
    else:
        mix=nodes.new('ShaderNodeMixRGB');links.new(keep,mix.inputs[0]);links.new(tex.outputs['Color'],mix.inputs[1]);links.new(original_surface,mix.inputs[2])
    links.new(mix.outputs[0],socket);mat['bark_fill_scope']='Generated-support AND original-unknown only; original shader is fallback'
    return mat
