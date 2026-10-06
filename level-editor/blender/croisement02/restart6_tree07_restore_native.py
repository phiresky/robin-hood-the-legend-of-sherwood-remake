"""Restore frozen bark RGB/alpha around a separately reviewed generated fill."""
import hashlib,json
from pathlib import Path
import numpy as np


def restore_atlas(guard_path, object_name, candidate, generated_provenance, physical_support):
    """Return a guarded array; this helper never opens or saves a Blender model.

    Generated support must come from the guarded camera sampler and actual face
    UV rasterization, not a color threshold. Unsupported original atlas regions,
    every original observed texel, and all alpha are immutable.
    """
    guard_path=Path(guard_path)
    guard=json.loads(guard_path.read_text())
    if guard['status']!='PASS':raise ValueError('Source provenance is not validated')
    rows=[r for r in guard['records'] if r['object']==object_name]
    if len(rows)!=1:raise ValueError('Receiver outside approved lower wood scope')
    row=rows[0];path=Path(row['original_atlas_path'])
    if hashlib.sha256(path.read_bytes()).hexdigest()!=row['original_atlas_sha256']:raise ValueError('Original source atlas changed')
    saved=np.load(path);original=saved['rgba'];known=saved['known'];candidate=np.asarray(candidate);generated=np.asarray(generated_provenance);support=np.asarray(physical_support)
    if candidate.shape!=original.shape or generated.shape!=known.shape or support.shape!=known.shape:raise ValueError('Atlas dimensions changed')
    if support.dtype!=np.bool_:raise ValueError('Physical UV support must be boolean')
    if not np.isfinite(candidate).all():raise ValueError('Candidate contains invalid samples')
    editable=(generated==2)&support&~known
    result=original.copy();result[editable,:3]=candidate[editable,:3]
    if not np.array_equal(result[known],original[known])or not np.array_equal(result[:,:,3],original[:,:,3])or not np.array_equal(result[~editable],original[~editable]):raise AssertionError('Protection invariant failed')
    return result,dict(object=object_name,editable_texels=int(editable.sum()),protected_known_texels=int(known.sum()),protected_known_rgba_exact=True,alpha_exact=True,unsupported_rgba_exact=True)
