"""Check axial support spans for the complete pile's accumulated gravity loads."""
import json,math,hashlib
import numpy as np
from catalog import OUT


def main():
    dest=OUT/'restart2-state/log-triangular-pile-v5';fit=json.loads((dest/'fit.json').read_text());bank=json.loads((dest/'reopened-support-audit.json').read_text());u=np.array(fit['axis']);nodes={};proof=[]
    for row in fit['records']:
        lo,hi=float(np.dot(row['start'],u)),float(np.dot(row['end'],u));mass=math.pi*row['radius']**2*(hi-lo);nodes[(row['layer'],row['column'])]=dict(lo=lo,hi=hi,load=mass,moment=mass*(lo+hi)/2)
    for (layer,column),node in sorted(nodes.items(),reverse=True):
        center=node['moment']/node['load']
        if layer==0:
            support=next(r for r in bank['bank_support']if r['column']==column);lo,hi=[node['lo']+t*(node['hi']-node['lo'])for t in support['contact_parameter_span']];assert lo<=center<=hi,(layer,column,center,lo,hi);proof.append(dict(layer=layer,column=column,total_load=node['load'],axial_load_center=center,support='approved bank',support_span=[lo,hi]));continue
        parents=[nodes[(layer-1,column+j)]for j in [0,1]];ranges=[[max(node['lo'],p['lo']),min(node['hi'],p['hi'])]for p in parents];lo=max(ranges[0][0],2*center-ranges[1][1]);hi=min(ranges[0][1],2*center-ranges[1][0]);assert lo<=hi,(layer,column,center,ranges);first=min(max(center,lo),hi);positions=[first,2*center-first]
        for parent,t in zip(parents,positions):parent['load']+=node['load']/2;parent['moment']+=node['load']/2*t
        proof.append(dict(layer=layer,column=column,total_load=node['load'],axial_load_center=center,support='two complete lower logs',contact_spans=ranges,load_positions=positions))
    (dest/'axial-load-support.json').write_text(json.dumps(dict(status='PASS axial load centers remain inside physical support spans',fit_sha256=hashlib.sha256((dest/'fit.json').read_bytes()).hexdigest(),bank_audit_sha256=hashlib.sha256((dest/'reopened-support-audit.json').read_bytes()).hexdigest(),records=proof,limitations=['Uniform density and symmetric ideal circular contacts are assumed.','This checks axial tipping/support, not a friction or dynamic motion simulation.','Small faceted contact deficits are recorded by the reopened support audit.']),indent=2)+'\n');print(len(proof),'supported bodies')
if __name__=='__main__':main()
