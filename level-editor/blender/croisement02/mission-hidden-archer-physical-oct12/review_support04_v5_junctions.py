"""Resume remaining junctions with only each measured wood receiver visible."""
from pathlib import Path
original=Path(__file__).with_name('review_support04_v3_junctions.py')
source=original.read_text()
needle="source=source.replace(needle,extra+needle)"
assert source.count(needle)==1
changes='''
source=source.replace("for phase in ['initial','transition-0','transition-1','applied']:","for phase in []:")
source=source.replace('geometry-v8-short-support04-review-v3-junctions','geometry-v8-short-support04-review-v5-junctions')
source=source.replace("        phase=row['phase']\\n", "        phase=row['phase']\\n        if phase=='initial' and row['component']==1:continue\\n        target=row['candidate']['target']['node']\\n        matched=[]\\n        for receiver in b.bpy.context.scene.objects:\\n            if receiver.type=='MESH' and receiver not in meshes:\\n                receiver.hide_render=receiver.name!=target\\n                if not receiver.hide_render:matched.append(receiver.name)\\n        assert len(matched)==1,(target,matched)\\n")
source=source.replace('available()<11*1024**3','available()<14*1024**3').replace('Waiting11GiB','Waiting14GiB')
source=source.replace('Wood-only context; full-scene finite support contact audit is separate.','Only the measured target wood is visible per junction; full-scene finite support contact audit is separate.')
'''
source=source.replace(needle,needle+changes)
exec(compile(source,__file__,'exec'),{'__name__':'__main__','__file__':__file__})
