"""Pure controlled apply/reset snapshots; never execute mission AI or sequences."""
def frame_at(row,updates,loop=False):
    if not isinstance(updates,int) or updates<0:raise ValueError('Explicit eligible update count required')
    if updates==0:return {'frame':row['frames'][0],'counter':65535}
    tick=updates-1
    if loop:tick%=sum(f['duration'] for f in row['frames'])
    for frame in row['frames']:
        if tick<frame['duration']:return {'frame':frame,'counter':tick}
        tick-=frame['duration']
    raise ValueError('Transition has terminated')

def sample_patch(binding,command,eligible_updates=0):
    """Command is a controlled snapshot epoch, not a requested gameplay action.

    A reset addresses this patch only. Callback latch, actor positions and
    queued sequence state belong to the caller's mission snapshot.
    """
    if command not in ('initial','apply','reset'):raise ValueError('Unknown snapshot epoch')
    states=binding['art']['states'];patch=binding['patch']
    if command in ('initial','reset'):
        row=states['initial'];draw=frame_at(row,eligible_updates,True) if row else None
        return {'phase':'initial','applied':False,'fx_active':row is not None,'draw':draw,'background':'baseline-restored','background_frame':None,'object_membership':'old','reset_callback_latch':False}
    if not isinstance(eligible_updates,int) or eligible_updates<0:raise ValueError('Explicit eligible update count required')
    row=states['transition'];end=row['eligible_updates_to_termination'] if row else 0
    if eligible_updates<end:
        return {'phase':'transition','applied':False,'fx_active':True,'draw':frame_at(row,eligible_updates),'background':'baseline','background_frame':None,'object_membership':'old'}
    final=states['final'];draw=frame_at(final,eligible_updates-end,True) if final else None
    return {'phase':'applied','applied':True,'fx_active':final is not None,'draw':draw,'background':'transition-terminal-persisted' if patch['integrate_in_background'] else 'baseline', 'background_frame':row['frames'][-1] if patch['integrate_in_background'] else None,'object_membership':'new'}
