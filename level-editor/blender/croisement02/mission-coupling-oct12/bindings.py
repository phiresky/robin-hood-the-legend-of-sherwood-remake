"""Build private mission patch bindings from frozen bytecode and native sprite rows."""
import collections,hashlib,json,re,struct
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[4]
WORK=ROOT/'level-editor/work/croisement02-refinement'
OUT=WORK/'mission-coupling-oct12-v1'
DATA=ROOT/'datadirs/fullgame_gog_hackable/Data'
REQUIREMENTS=WORK/'restart333-remaining-patch-coupling-requirements-v1/report.json'
SIGNATURES=ROOT/'crates/robin_script_types/src/natives/signatures.rs'
def read(p):return json.loads(p.read_text())
def digest(x):return hashlib.sha256(json.dumps(x,sort_keys=True,separators=(',',':')).encode()).hexdigest()
def pin(p):return {'path':str(p.relative_to(ROOT)),'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}
def registry():
    s=SIGNATURES.read_text().split('original {',1)[1].split('rust_extensions {',1)[0]
    rows=re.findall(r'^\s*(\w+) => \("[^"]+", \[(.*?)\],',s,re.M)
    assert len(rows)==265
    return [(name,len(re.findall(r'\("',args))) for name,args in rows]

def trace_callback(cls,already_done=False):
    """Evaluate only the small, pure one-shot callback instruction subset.

    Native operations produce typed commands; no game state, AI or sequence
    scheduler runs. Unsupported opcodes/natives fail instead of synthesizing data.
    """
    funcs={f['name']:f for f in cls['functions']};ip=funcs['ActivatedByArrow']['address']
    assert cls['member_variables']==[{'ty':{'tag':'Int','native_type_name':''},'name':'deja_fait','address':0}]
    mem={0x4000:int(already_done)};stack=[];result=None;events=[];native=registry()
    permitted={'ApplyPatch','SetActorLocation','Start','Then','Thanx','RecordPlayAnimFreeze','RecordTimer','RecordUnlockAI'}
    for _ in range(500):
        q=cls['quads'][ip];op=q['operation'];b=bytes(q['operands']);s=lambda n:struct.unpack_from('<H',b,n)[0];u=lambda n:struct.unpack_from('<I',b,n)[0]
        nextip=ip+1
        if op in (1,3):pass
        elif op in (6,4):return events,mem[0x4000]
        elif op==7:pass
        elif op==19:mem[s(0)]=struct.unpack_from('<i',b,4)[0]
        elif op==17:mem[s(0)]=mem[s(2)]
        elif op==41:mem[s(0)]=int(mem[s(2)]==mem[s(4)])
        elif op==14:nextip=u(0)
        elif op in (15,16):
            if (mem[s(0)]!=0)==(op==15):nextip=u(4)
        elif op==11:stack.append(mem[s(0)])
        elif op==13:mem[s(0)]=result
        elif op==12:
            name,n=native[u(0)];assert len(stack)>=n
            args=list(reversed([stack.pop() for _ in range(n)]))
            if name in ('GetActorScript','GetLocationScript','GetPatchScript'):
                result={'kind':name[3:-6].lower(),'index':args[0]}
            elif name in permitted:
                events.append({'quad':ip,'native':name,'args':args});result=1
            else:raise ValueError(('Unsupported callback native',name))
        else:raise ValueError(('Unsupported callback opcode',op,ip))
        ip=nextip
    raise ValueError('Callback instruction budget exceeded')

def actor_table(proto,mission):
    assert proto['element_chunk_order']==['Animation','Patch']
    assert mission['element_chunk_order'][:2]==['Patch','Element']
    assert mission['element_group_order']==['Animal','BeamMe','Civilian','PcToRescue','Soldier','Target']
    rows=[]
    for group,values in [('animations',proto['animations']),('map_patches',proto['patches']),('mission_patches',mission['mission_patches']),('civilians',mission['civilians']),('pcs_to_rescue',mission['pcs_to_rescue']),('soldiers',mission['soldiers']),('targets',mission['targets'])]:
        for i,value in enumerate(values):
            if 'patches' in group:assert value['element_fx']['sprite']['frame_profile_name']
            rows.append({'handle':len(rows),'group':group,'index':i,'source_record_sha256':digest(value),'source':value})
    return rows

def states(patch,ambiance):
    sprite=patch['element_fx']['sprite'];directory=DATA/'Animations'/ambiance/(sprite['frame_profile_name']+'.rhs.d')
    if not directory.exists():directory=DATA/'Animations/Day'/(sprite['frame_profile_name']+'.rhs.d')
    manifest=directory/'manifest.json';profile=next(p for p in read(manifest)['profiles'] if p['name']==sprite['profile_name']);result={}
    for name,action,valid in [('initial',148,'start_animation_valid'),('transition',149,'transition_animation_valid'),('final',150,'end_animation_valid')]:
        if not patch[valid]:result[name]=None;continue
        row=next(r for r in profile['rows'] if r['action_id']==action);frames=[]
        for i,f in enumerate(row['frames']):
            path=directory/profile['name']/row['path']/f['file'];a=np.array(Image.open(path).convert('RGBA'));rgb=a[:,:,:3].astype(np.uint16);v=(rgb[:,:,0]>>3)<<11|(rgb[:,:,1]>>2)<<5|(rgb[:,:,2]>>3)
            transparent=(a[:,:,3]==0)|(v==0x07c0);shadow=(v==31)&~transparent;body=~(transparent|shadow)
            frames.append({'index':i,'image':pin(path),'delay':f['delay'],'duration':f['delay']+1,'sound_id':f['sound_id'],'size':[a.shape[1],a.shape[0]],'top_left':[sprite['position_x']+f['offset_x'],sprite['position_y']+f['offset_y']], 'body_pixels':int(body.sum()),'shadow_pixels':int(shadow.sum()),'transparent_pixels':int(transparent.sum())})
        result[name]={'action':action,'frames':frames,'eligible_updates_to_termination':sum(f['duration'] for f in frames)}
    return {'manifest':pin(manifest),'profile_sha256':digest(profile),'profile_center':[profile['center_x'],profile['center_y']],'states':result}

def build():
    proto_path=DATA/'Levels/Croisement02.rhp.json';proto=read(proto_path);requirements=read(REQUIREMENTS);rows=[];missions={}
    for item in requirements['rows']:
        name=item['mission'];index=item['patch_index']
        if name not in missions:
            mp=DATA/'Levels'/(name+'.rhm.json');sp=DATA/'Levels'/(name+'.scb.json');tp=DATA/'Levels'/(name+'.scb.ts')
            m=read(mp);scb=read(sp);text=tp.read_text();table=actor_table(proto,m)
            missions[name]=(m,scb,text,table,{'mission':pin(mp),'bytecode':pin(sp),'inspection_source':pin(tp)})
        m,scb,text,table,pins=missions[name];patch=m['mission_patches'][index];runtime=len(proto['patches'])+index
        assert patch['element_fx']['sprite']['profile_name']==item['profile']
        ambiance={1:'Day',2:'Fog',4:'Night',8:'Attack'}[m['header']['ambiance']]
        art=states(patch,ambiance);matches=[]
        for cls in scb['classes']:
            if not cls['class_name'].startswith('cible_hidden_archer'):continue
            events,done=trace_callback(cls)
            if not any(e['native']=='ApplyPatch' and e['args'][0]['index']==runtime for e in events):continue
            assert done==1 and trace_callback(cls,True)==([],1)
            targets=[r for r in table if r['group']=='targets' and r['source']['script_class']==cls['class_name']];assert len(targets)==1
            target=targets[0];moves=[];unlocks=[];companions=[]
            for e in events:
                if e['native']=='SetActorLocation':
                    actor,loc=e['args'];resolved=table[actor['index']];assert resolved['group']=='soldiers';point=m['script_objects']['points'][loc['index']]
                    moves.append({'actor':resolved,'location_index':loc['index'],'location':point})
                elif e['native']=='RecordUnlockAI':unlocks.append(table[e['args'][0]['index']])
                elif e['native']=='ApplyPatch':
                    pi=e['args'][0]['index'];p=(proto['patches']+m['mission_patches'])[pi];companions.append({'runtime_index':pi,'patch_sha256':digest(p),'source':p})
            matches.append({'target':target,'callback_class':cls['class_name'],'callback_bytecode_sha256':digest(cls),'one_shot_member':'deja_fait','events':events,'actor_moves':moves,'ai_unlock_actors':unlocks,'move_and_unlock_handles_equal':{r['actor']['handle'] for r in moves}=={r['handle'] for r in unlocks},'patch_commands':companions})
        assert len(matches)<=1
        # Inspect all literal references plus the generic God-message dispatcher.
        calls=[{'value':v,'line':text[:ma.start()].count('\n')+1} for ma in re.finditer(r'GetPatchScript\(/\*iPosition\*/ ([^)]+)\)',text) for v in [ma.group(1)]]
        exact=[c for c in calls if c['value']==str(runtime)]
        if not matches:assert not exact
        row={'mission':name,'patch_index':index,'runtime_patch_index':runtime,'profile':item['profile'],'source_pins':pins,'patch_sha256':digest(patch),'patch':patch,'art':art,'trigger':matches[0] if matches else None,
             'trigger_qualification':'BYTECODE_VERIFIED_ONE_SHOT_ARROW_CALLBACK' if matches else 'NO_AUTHORED_LITERAL_TRIGGER_OR_TARGET_ASSOCIATION_FOUND',
             'patch_reference_scan':{'exact_references':exact,'nonliteral_references':[c for c in calls if not c['value'].isdigit()],'scope':'Entire mission inspection source; generic external message arguments remain possible, not authored gameplay evidence.'},
             'semantics':{'mask_policy':'none-for-base-patch-fx','draw_stage':'background-effect-creation-order' if patch['element_fx']['sprite']['elevation']==0 else 'ordered-fx','clock':'25Hz eligible FX updates from authoritative clock; freeze/inactive contribute zero; reset frame counter65535; no second clock','apply':'Select action149 frame0 reset; on terminal eligible update swap background/objects, then final row or hide FX.','integrated_terminal':'Persist last transition frame in restored background' if patch['integrate_in_background'] else 'No persistent background write','reset':'ForceReset restores initial artwork/background and object membership. Does not reset target deja_fait, soldier positions or recorded sequences. Full callback replay needs authoritative mission snapshot restore.','actor_placement':'SetActorLocation uses authored point mapXY/layer/sector, recomputes projection obstacle/world/display order; reactivates only an actor previously sent to NoWhere.','shadow':'Legacy transparency5650x07c0 and shadow5650x001f; destination retention0.9Fog/0.6otherwise when source shadow blit enabled.','ordering':'Source placement/polyline preserved; no physical-depth or gameplay-trigger proof claimed.'},
             'geometry_reuse':{'status':'NO_APPROVAL_TRANSFER','rationale':'Variants01–04 are distinct source shapes; approved variant05 does not qualify these. Revealed soldiers reuse their native actor identities; no new static archer mesh.'}}
        rows.append(row)
    for row in rows:
        points=row['patch']['apply_sector']['points']
        if row['trigger'] is None and points:
            row['trigger_qualification']='SOURCE_VERIFIED_PC_PATCH_CROSSING'
            row['automatic_trigger']={'kind':'pc-enters-apply-sector','polygon':points,'layer':row['patch']['layer'],'conditions':['crossed an active patch boundary','actor is PC','patch active','new map position inside apply polygon','patch not already applied'],'coupled_actor':'the crossing PC is an occupant, not an automatically captured-body attachment','source_behavior':'Enter registers occupant including carried person; Apply starts patch. Definitive completion disables further application. No mission-specific capture command is supplied.'}
    # The Tac19 loose-net source exactly matches the selected Emb05 family.
    net=next(r for r in rows if r['mission']=='Tac19_FoB_EC' and r['patch_index']==0)
    donor_path=DATA/'Levels/Emb05_FoB_MP.rhm.json';donor=read(donor_path)['mission_patches'][2];assert donor==net['patch']
    net['geometry_reuse']={'status':'EXACT_SOURCE_AND_PLACEMENT_REUSE_CANDIDATE','donor_mission':pin(donor_path),'donor_patch_index':2,'same_patch_record':True,'candidate_assets':['croisement02-initial-net-01','croisement02-net-piege01-empty-final-0','croisement02-net-piege01-occupied-final-0'],'rationale':'Exact source identity qualifies comparison to existing endpoint family; piege01g is loose debris, not proof that empty/occupied captured-body state is present in Tac19. No Tac19 capture association found; do not select occupied variant without authority.'}
    return {'version':1,'scope':'Private source bindings and deterministic diagnostic trace; no canonical integration or gameplay completion','requirements':pin(REQUIREMENTS),'proto':pin(proto_path),'native_registry':pin(SIGNATURES),'rows':rows,'counts':dict(collections.Counter(r['trigger_qualification'] for r in rows))}

def main():
    assert not (OUT/'bindings-v2.json').exists();OUT.mkdir(exist_ok=True)
    result=build();(OUT/'bindings-v2.json').write_text(json.dumps(result,indent=2)+'\n')
    print(result['counts'])
if __name__=='__main__':main()
