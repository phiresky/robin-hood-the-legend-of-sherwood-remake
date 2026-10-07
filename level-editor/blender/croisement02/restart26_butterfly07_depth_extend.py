"""Extend only disconnected depth windows, retaining every prior exact sample."""
import json
import numpy as np
import restart26_butterfly07_depth_refinement as sample
import restart14_butterfly07_anatomy_contacts as audit
OUT=audit.B/'butterfly07-v3-depth-outer-samples-v1'
EXTRA=np.array([-10.,-9.,-8.,-7.,-6.5,6.5,7.,8.,9.,10.])

def main():
    assert not OUT.exists();assert audit.reader.sha(sample.FIT)==sample.FIT_SHA
    OUT.mkdir();original=sample.OUT
    fit=json.loads(sample.FIT.read_text());fit['rows']=[r for r in fit['rows'] if 15<=r['phase']<=34]
    path=OUT/'window-fit.json';path.write_text(json.dumps(fit,indent=2)+'\n')
    audit.OUT=OUT/'contacts';audit.main(path,depth_trials={f'offset{float(v)}':np.full(99,v).tolist() for v in EXTRA},sweep_subdivisions=0,geometry_helper=sample.HELPER,geometry_helper_sha256=sample.HELPER_SHA,receiver_asset_ids=None,first_witness_only=True)
    first=original/'contacts/report.json';second=OUT/'contacts/report.json';a=json.loads(first.read_text());b=json.loads(second.read_text())
    assert a['map_sha256']==b['map_sha256'] and a['geometry_binding']==b['geometry_binding']
    hashes={r['asset']:r['model_sha256'] for r in a['assets']}
    for row in b['assets']:
        if row['asset'] in hashes:assert hashes[row['asset']]==row['model_sha256']
    combined=dict(a);combined['results']=dict(a['results']);combined['results'].update({k:v for k,v in b['results'].items() if k.startswith('depth-trial:')})
    combined['combined_evidence']=[dict(path=str(p),sha256=audit.reader.sha(p))for p in [first,second]]
    combined['status']='UNION_OF_TWO_PINNED_EXACT_SAMPLE_AUDITS'
    folder=OUT/'combined';(folder/'contacts').mkdir(parents=True);(folder/'contacts/report.json').write_text(json.dumps(combined,indent=2)+'\n')
    # Select the newly sampled windows separately; successful earlier windows are unchanged.
    sample.OUT=folder;sample.OFFSETS=np.sort(np.r_[sample.OFFSETS,EXTRA]);sample.WINDOWS=[(15,26),(27,34)];sample.select()
    new=json.loads((folder/'corridor.json').read_text());old=json.loads((original/'corridor.json').read_text());windows={tuple(w['phases']):w for w in old['windows']}
    for window in new['windows']:windows[tuple(window['phases'])]=window
    new['windows']=[windows[key]for key in sorted(windows)];new['constraints']['trust_region_Z_outer_windows']=[-10,10];new['constraints']['sample_spacing_Z_outer_windows']='0.5 at transition to old bound, then1.0';new['earlier_corridor_sha256']=audit.reader.sha(original/'corridor.json');new['limits'].append('Outer samples only for15–26 and27–34; earlier windows retain their original±6 constraints.')
    (OUT/'combined-corridor.json').write_text(json.dumps(new,indent=2)+'\n')

if __name__=='__main__':main()
