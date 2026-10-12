"""Locate qualified crown contacts in the original authored foliage components."""
import json, sys
from pathlib import Path
import numpy as np
from scipy.ndimage import label
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
from matplotlib.patches import Rectangle
HERE = Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
import qualify as q

def main():
    dest = q.OUT/'profile01-crown-contact-source-v1'; assert not dest.exists()
    contact_path = q.OUT/'geometry-v6-all-pose-contacts/report.json'
    contact = json.loads(contact_path.read_text())
    qual = json.loads((q.OUT/'qualification.json').read_text())['rows'][0]
    frames = qual['art']['states']['initial']['frames'] + qual['art']['states']['transition']['frames']
    rows = [r for r in contact['rows'] if r['profile'] == 1]
    fig, axes = plt.subplots(2,2,figsize=(12,8),layout='constrained')
    records = []
    for ax,row,frame in zip(axes.ravel(),rows,frames):
        assert q.pin(q.ROOT/frame['image']['path']) == frame['image']
        rgba = np.array(q.rgba(frame)); labs,_ = label(rgba[:,:,3]>0,np.ones((3,3)))
        sizes = np.bincount(labs.ravel());sizes[0] = 0;main = int(sizes.argmax())
        cells = sorted(set(tuple(w['source_cell']) for w in row['witnesses']))
        assert all(w['node'] == 'Northwest Tree 00 / Crown' for w in row['witnesses'])
        x0,y0 = map(int,frame['top_left'])
        assert all(labs[y-y0,x-x0] == main for x,y in cells)
        ax.set_facecolor('#252a30');ax.imshow(rgba,interpolation='nearest',extent=[x0,x0+rgba.shape[1],y0+rgba.shape[0],y0])
        for x,y in cells: ax.add_patch(Rectangle((x,y),1,1,fill=False,edgecolor='#ff7862',linewidth=.9))
        ax.set_title(f"{row['phase']}: {len(cells)} crown-contact cells, all in main leaf component")
        ax.set_xlabel('Native X');ax.set_ylabel('Native Y')
        records.append(dict(phase=row['phase'],source=frame['image'],cells=cells,
            main_component_pixels=int(sizes[main]),all_contacts_in_main_component=True,
            interpretation='Right-hand main foliage volume meets existing Tree00 crown; a leaf-to-leaf attachment hypothesis, not a demonstrated wood branch.'))
    fig.suptitle('Profile01 native source and qualified Tree00 crown contacts — geometry diagnostic')
    dest.mkdir();fig.savefig(dest/'source-contacts.png',dpi=140);plt.close(fig)
    (dest/'report.json').write_text(json.dumps(dict(status='SOURCE_CONTACT_INTERPRETATION_PENDING_JOINED_VISIBILITY',
        contacts=q.pin(contact_path),records=records,diagram=q.pin(dest/'source-contacts.png'),recipe=q.pin(Path(__file__)),
        limits=['Original source RGB/alpha unchanged; colored outlines are diagnostic annotations only.',
        'Contacts provide a plausible foliage connection, not exact hidden branch structure.',
        'Joined source visibility remains a separate required check.']),indent=2)+'\n')
    print(dest)

if __name__ == '__main__': main()
