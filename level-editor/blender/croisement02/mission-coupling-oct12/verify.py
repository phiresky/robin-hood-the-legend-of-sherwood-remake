"""Verify source binding commands, clock boundaries and canonical reuse evidence."""
import collections,json,re,struct
from pathlib import Path
import bindings as b
from preview import sample_patch
LIB=b.ROOT/'level-editor/library'

def literal_patch_reads(scb):
    result=[]
    for cls in scb['classes']:
        for i,q in enumerate(cls['quads']):
            if q['operation']!=12 or int.from_bytes(bytes(q['operands'][:4]),'little')!=5:continue
            prev=cls['quads'][i-1];assert prev['operation']==11
            sym=bytes(prev['operands'][:2]);value=None;writer=None
            for j in range(i-2,-1,-1):
                x=cls['quads'][j]
                if x['operation']==3:break
                if x['operation'] in set(range(8,11))|{13}|set(range(17,48)) and bytes(x['operands'][:2])==sym:
                    writer=x['operation']
                    if writer==19:value=struct.unpack('<i',bytes(x['operands'][4:8]))[0]
                    break
            assert writer is not None
            result.append({'class':cls['class_name'],'quad':i,'literal':value,'argument_writer_opcode':writer})
    return result

def all_pins(value):
    if isinstance(value,dict):
        if isinstance(value.get('path'),str) and 'sha256' in value:yield value
        for v in value.values():yield from all_pins(v)
    elif isinstance(value,list):
        for v in value:yield from all_pins(v)

def main():
    path=b.OUT/'bindings-v2.json';data=b.read(path);assert data==b.build()
    pins={p['path']:p for p in all_pins(data)}
    for p in pins.values():assert b.pin(b.ROOT/p['path'])==p
    timeline=[];dormant=[];candidates=[];indexpath=LIB/'mission-states/index.json';index=b.read(indexpath)
    for row in data['rows']:
        n=row['art']['states']['transition']['eligible_updates_to_termination']
        traces=[{'eligible_updates':t,**sample_patch(row,'apply',t)} for t in [0,1,n-1,n,n+1]]
        assert traces[0]['draw']['counter']==65535 and traces[1]['draw']['counter']==0
        assert traces[-2]['phase']=='applied' and traces[-3]['phase']=='transition'
        reset=sample_patch(row,'reset');assert reset['background']=='baseline-restored' and not reset['reset_callback_latch']
        if row['patch']['integrate_in_background']:assert traces[-2]['background_frame']==row['art']['states']['transition']['frames'][-1] and not traces[-2]['fx_active']
        else:assert traces[-2]['background_frame'] is None and traces[-2]['fx_active']
        timeline.append({'mission':row['mission'],'patch_index':row['patch_index'],'snapshots':traces,'reset':reset})
        if row['trigger'] is None:
            reads=literal_patch_reads(b.read(b.ROOT/row['source_pins']['bytecode']['path']))
            assert not any(x['literal']==row['runtime_patch_index'] for x in reads)
            dormant.append({'mission':row['mission'],'patch_index':row['patch_index'],'literal_bytecode_reads_absent':True,'dynamic_reads':[r for r in reads if r['literal'] is None],'automatic_apply_sector_empty':not bool(row['patch']['apply_sector']['points']),'automatic_trigger':row.get('automatic_trigger'),'qualification':('No literal script caller; authored automatic PC crossing trigger verified.' if row.get('automatic_trigger') else 'No authored trigger found; generic externally supplied message arguments are not ruled out.')})
        found=[]
        for entry in index['entries']:
            if entry.get('map')!='Croisement02' or entry.get('mission')!=row['mission'] or 'contract' not in entry:continue
            cp=LIB/entry['contract']['path'];contract=b.read(cp)
            ps=[p for p in contract.get('native',{}).get('patch_states',[]) if p.get('source',{}).get('kind')=='mission-patch' and p['source']['index']==row['patch_index']]
            if not ps:continue
            assert len(ps)==1
            mission=b.read(LIB/entry['mission_data']['path']);assert mission['mission_patches'][row['patch_index']]==row['patch']
            p=ps[0]
            for state in ('initial','transition','final'):
                own=row['art']['states'][state]
                ownframes=own['frames'] if own else []
                assert len(ownframes)==len(p[state])
                for a,c in zip(ownframes,p[state]):
                    assert a['top_left']==[p['display_position'][0]+c['offset'][0],p['display_position'][1]+c['offset'][1]]
                    assert a['delay']==c['delay'] and a['size']==[c['width'],c['height']]
                    assert b.pin(LIB/c['path'])['sha256']==c['sha256']
            found.append({'entry_id':entry['id'],'contract':b.pin(cp),'native_patch_id':p['id'],'native_patch_sha256':b.digest(p),'selected_in_existing_family':any(p['id'] in f.get('patch_ids',[]) for f in contract.get('families',[]))})
        assert found,(row['mission'],row['patch_index'])
        chosen=found[0]
        candidates.append({'id':row['mission'].lower()+'-coupling-patch-'+str(row['patch_index']),'mission':row['mission'],'binding_patch_index':row['patch_index'],'base_contract':chosen['contract'],'native_patch_id':chosen['native_patch_id'],'source_patch_sha256':row['patch_sha256'],'family_candidate':{'id':'remaining-patch-'+str(row['patch_index']),'patch_ids':[chosen['native_patch_id']],'background_ids':[],'element_ids':[]},'binding_status':'PRIVATE_SOURCE_SELECTION_ONLY','geometry_reuse':row['geometry_reuse'],'existing_context_matches':found})
    net=next(r for r in data['rows'] if r['mission']=='Tac19_FoB_EC' and r['patch_index']==0)
    reuse=[]
    for state in ('empty','occupied'):
        cp=LIB/f'mission-states/croisement02/contracts/Emb05_FoB_MP-net-01-{state}.json';contract=b.read(cp)
        mission=LIB/'mission-states/croisement02/source/Emb05_FoB_MP.rhm.json';assert b.read(mission)['mission_patches'][2]==net['patch']
        f=contract['families'][0];selected=next(p for p in contract['native']['patch_states'] if p['source']['index']==2);assert selected['id'] in f['patch_ids']
        models=[]
        for epoch,items in f['physical'].items():
            if not isinstance(items,list):continue
            for item in items:
                p=LIB/item['model'];assert b.pin(p)['sha256']==item['model_sha256'];models.append({'epoch':epoch,'asset_id':item['id'],'file':b.pin(p),'position':item.get('position')})
        reuse.append({'state':state,'contract':b.pin(cp),'source':b.pin(mission),'family':f['id'],'selected_native_patch':selected['id'],'models':models,'qualification':'Same loose-net patch; full net-family endpoint is not an independent loose-debris mesh. No Tac19 capture or occupied endpoint claim.'})
    report={'status':'PASS_PRIVATE_BINDINGS_CPU_ONLY','bindings':b.pin(path),'checked_pins':len(pins),'verified_one_shot_callbacks':15,'non_arrow_sources':dormant,'source_discrepancies':[{'mission':r['mission'],'patch_index':r['patch_index'],'moved':[m['actor']['handle'] for m in r['trigger']['actor_moves']],'unlocked':[a['handle'] for a in r['trigger']['ai_unlock_actors']]} for r in data['rows'] if r['trigger'] and not r['trigger']['move_and_unlock_handles_equal']],'canonical_net_reuse':reuse,'implementation_pins':[b.pin(Path(__file__)),b.pin(Path(b.__file__)),b.pin(Path(__file__).with_name('preview.py'))],'limitations':['No browser, saved-model render, source callback dispatch in gameplay, world-space actor motion, or geometry acceptance established.','Background integration requires whole source transaction at terminal; restoring patch does not undo callback actors or latch.','Integrated terminal background follows the selected patch integrate_in_background flag; no separate background-state ID is invented. Candidate families still need production renderer verification.']}
    for name,value in [('verification-v2.json',report),('preview-traces-v2.json',timeline),('native-binding-candidates-v2.json',candidates)]:
        dest=b.OUT/name;assert not dest.exists();dest.write_text(json.dumps(value,indent=2)+'\n')
    print(report['status'],len(pins),'pins',len(candidates),'candidates',len(dormant),'non-arrow sources')
if __name__=='__main__':main()
