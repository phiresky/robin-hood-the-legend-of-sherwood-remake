"""Annotated geometric sections of frozen candidate; no image or model editing."""
import json
from pathlib import Path
import numpy as np
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
from matplotlib.collections import LineCollection,PolyCollection
from matplotlib.patches import Rectangle
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2/restart61-room-architecture-audit-v1';r=json.loads((W/'geometry.json').read_text());objects=r['objects'];colors={'neighbor-baseline':'#8fa9ba','static':'#47515d','cover':'#d49120','receiver':'#d54343'}
def style(o):return 'neighbor-baseline'if o['kind']=='neighbor-baseline'else'receiver'if o['receiver']else'cover'if o['cover']else'static'
def section(o,axis,value,axes):
 v=np.array(o['vertices']);lines=[]
 for tri in o['triangles']:
  ps=v[tri];hits=[]
  for a,b in zip(ps,np.roll(ps,-1,axis=0)):
   if (a[axis]-value)*(b[axis]-value)<0:
    p=a+(b-a)*(value-a[axis])/(b[axis]-a[axis]);hits.append(p[list(axes)])
  if len(hits)==2:lines.append(hits)
 return lines
fig,axs=plt.subplots(1,2,figsize=(16,9))
for ax,applied in zip(axs,[False,True]):
 for o in objects:
  if o['kind']=='neighbor-baseline'and o['source']in {x['source']for x in objects if x['kind']=='candidate'}:continue
  if applied and o['cover']:continue
  segs=section(o,2,150,(0,1));ax.add_collection(LineCollection(segs,colors=colors[style(o)],linewidths=2 if o['receiver']else .8,alpha=.9))
 for i,o in enumerate(x for x in objects if x['receiver']):
  b=o['bounds'];ax.text((b[0][0]+b[0][1])/2,(b[1][0]+b[1][1])/2,str(i+1),color='#a00000',fontsize=11,bbox=dict(facecolor='white',alpha=.8,edgecolor='none'))
 ax.add_patch(Rectangle((2373,-1840),8,20,fill=False,edgecolor='magenta',linestyle='--',linewidth=2));ax.annotate('Floor beyond folded wall\n(no enclosure here)',(2377,-1830),(2392,-1772),arrowprops=dict(arrowstyle='->'),fontsize=9)
 ax.set(xlim=(2270,2500),ylim=(-2015,-1660),xlabel='World X',ylabel='World Y',title=('Covered'if not applied else'Revealed')+' — horizontal section Z=150');ax.set_aspect('equal');ax.grid(alpha=.15)
fig.suptitle('York room architecture audit: gray = approved gatehouse; gold = switched cover; red = inferred receivers; blue = neighboring baseline castle',fontsize=12);fig.tight_layout();fig.savefig(W/'top-sections.png',dpi=150);plt.close(fig)
fig,axs=plt.subplots(1,2,figsize=(16,7))
for ax,x in zip(axs,[2377,2405]):
 for o in objects:
  if o['cover']:continue
  if o['kind']=='neighbor-baseline'and o['source']in {a['source']for a in objects if a['kind']=='candidate'}:continue
  ax.add_collection(LineCollection(section(o,0,x,(1,2)),colors=colors[style(o)],linewidths=2 if o['receiver']else .9))
 ax.axhline(200,color='#d54343',alpha=.2,linestyle='--');ax.set(xlim=(-1990,-1770),ylim=(90,320),xlabel='World Y',ylabel='World Z',title=f'Revealed side section X={x}');ax.set_aspect('equal');ax.grid(alpha=.15)
 ax.annotate('Wall tops Z=200;\nno dedicated ceiling',(-1820,200),(-1860,285),arrowprops=dict(arrowstyle='->'),fontsize=10)
fig.suptitle('The source-ray aperture removes surfaces through depth; an enclosed room is not established by these sections');fig.tight_layout();fig.savefig(W/'side-sections.png',dpi=150);plt.close(fig)
fig,ax=plt.subplots(figsize=(12,10))
for o in objects:
 if o['kind']!='neighbor-baseline':continue
 if not o['source'].startswith('building-7'):continue
 v=np.array(o['vertices']);tri=[v[t][:,[0,1]]for t in o['triangles']];ax.add_collection(PolyCollection(tri,facecolors='#91a6b5',edgecolors='#566b7a',linewidths=.2,alpha=.08));b=o['bounds'];ax.text(sum(b[0])/2,sum(b[1])/2,o['source'],fontsize=8)
for i,o in enumerate(x for x in objects if x['receiver']):
 b=o['bounds'];ax.add_patch(Rectangle((b[0][0],b[1][0]),b[0][1]-b[0][0],b[1][1]-b[1][0],fill=False,edgecolor='#d54343',linewidth=2));ax.text(sum(b[0])/2,sum(b[1])/2,str(i+1),color='#b00000')
ax.set(xlim=(2200,2600),ylim=(-2080,-1510),xlabel='World X',ylabel='World Y',title='Plan projection with neighboring castle source geometry (baseline context, not refreshed assets)');ax.set_aspect('equal');ax.grid(alpha=.15);fig.tight_layout();fig.savefig(W/'neighbor-plan.png',dpi=150)
(W/'receiver-key.json').write_text(json.dumps({i+1:o['name']for i,o in enumerate(x for x in objects if x['receiver'])},indent=2))
