"""Read-only entry-level catalog size audit; never load a browser or mutate data."""
import argparse,hashlib,json,time
from pathlib import Path

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('catalog',type=Path)
    parser.add_argument('output',type=Path)
    args=parser.parse_args()
    available=int(next(line.split()[1] for line in Path('/proc/meminfo').read_text().splitlines() if line.startswith('MemAvailable:')))*1024
    if available<6*1024**3:raise RuntimeError('Need 6GiB available RAM for catalog profiling')
    started=time.monotonic();text=args.catalog.read_text();read_seconds=time.monotonic()-started
    decoder=json.JSONDecoder();offset=text.index('[')+1;rows=[];started=time.monotonic()
    while True:
        while text[offset].isspace() or text[offset]==',':offset+=1
        if text[offset]==']':break
        entry,end=decoder.raw_decode(text,offset)
        rows.append({'id':entry['id'],'source_map':entry.get('source_map'),'bytes':end-offset,
                     'editor_fields':{k:len(json.dumps(v,separators=(',',':'))) for k,v in entry.get('editor',{}).items()}})
        offset=end
    result={'catalog_bytes':args.catalog.stat().st_size,'sha256':hashlib.sha256(text.encode()).hexdigest(),
            'read_seconds':read_seconds,'entry_parse_measure_seconds':time.monotonic()-started,
            'timing_limitation':'Entry parse time includes measuring serialized field sizes; it is not pure JSON decode or browser runtime.',
            'asset_count':len(rows),'top_entries':sorted(rows,key=lambda r:r['bytes'],reverse=True)[:25],
            'map_totals':{m:sum(r['bytes'] for r in rows if r['source_map']==m) for m in {r['source_map'] for r in rows}},'rows':rows}
    args.output.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({k:v for k,v in result.items() if k not in ['rows','top_entries']}))

if __name__=='__main__':main()
