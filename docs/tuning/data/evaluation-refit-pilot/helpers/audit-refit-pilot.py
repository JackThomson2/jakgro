import argparse
import csv
import gzip
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('--bin-dir', required=True, type=Path)
parser.add_argument('--scratch', required=True, type=Path)
args = parser.parse_args()
binaries, scratch = args.bin_dir.resolve(), args.scratch.resolve()
root = Path(__file__).resolve().parent.parent
helpers = root / 'helpers'
scratch.mkdir(parents=True, exist_ok=True)
sys.path.insert(0, str(Path.cwd()))
from tools.run_sprt import evaluate, pair_points_from_pgn

sha = lambda raw: hashlib.sha256(raw).hexdigest()
index = json.loads((root/'sha256.json').read_text())
for name, digest in index.items():
    assert sha((root/name).read_bytes()) == digest, name
print(f'Verified {len(index)} indexed artifacts')

regenerated = scratch/'regenerated'
subprocess.run([str(binaries/'refit-books'), 'generate', 'tools/data/openings.epd', str(root/'historical.fen'), str(regenerated)], check=True)
for name in ['training.epd','training.keys','development.epd','development.keys','confirmation.epd','confirmation.keys','historical.keys']:
    assert (regenerated/name).read_bytes() == (root/name).read_bytes(), name
assert (regenerated/'lineage.tsv').read_bytes() == gzip.decompress((root/'lineage.tsv.gz').read_bytes())
lineage = list(csv.DictReader((regenerated/'lineage.tsv').open(), delimiter='\t'))
parent_sets = {c: {x['parent'] for x in lineage if x['channel']==c} for c in ['training','development','confirmation']}
assert [len(parent_sets[c]) for c in ['training','development','confirmation']] == [32,8,8]
assert all(not parent_sets[a]&parent_sets[b] for a,b in [('training','development'),('training','confirmation'),('development','confirmation')])
keys = {c: set((root/(c+'.keys')).read_text().splitlines()) for c in ['historical','training','development','confirmation']}
assert all(not keys[a]&keys[b] for a,b in [('historical','training'),('historical','development'),('historical','confirmation'),('training','development'),('training','confirmation'),('development','confirmation')])

records = [('training','training-match.json','training'),('development','development-match.json','development')]
models = ['conservative','weaker-prior','hybrid','features-only']
records += [(n+'-development',n+'-development.json','development') for n in models]
total = 0
for name, summary_name, channel in records:
    summary = json.loads((root/summary_name).read_text())
    manifest_path = root/(name+'.manifest.json')
    manifest = json.loads(manifest_path.read_text())
    assert summary['status']=='complete' and not summary['faults']
    assert manifest['execution']['return_code']==0 and not manifest['execution']['faults']
    assert manifest['execution']['inputs_unchanged']
    assert sha(manifest_path.read_bytes()) == summary['inputs']['manifest_sha256']
    assert summary['inputs']['openings_sha256'] == sha((root/(channel+'.epd')).read_bytes())
    raw = gzip.decompress((root/(name+'.pgn.gz')).read_bytes())
    assert sha(raw)==summary['inputs']['pgn_sha256']==manifest['execution']['pgn_sha256']
    starts = re.findall(r'^\[FEN "([^"]+)"\]', raw.decode(), re.M)
    assert len(starts)==summary['result']['games']
    assert all(' '.join(f.split()[:4]) in keys[channel] for f in starts)
    assert all(' '.join(f.split()[:4]) not in keys['confirmation'] for f in starts)
    pgn = scratch/(name+'.pgn')
    pgn.write_bytes(raw)
    shutil.copyfile(manifest_path, scratch/manifest_path.name)
    pairs = pair_points_from_pgn(pgn,summary['engines']['candidate'],summary['engines']['baseline'])
    assert evaluate(pairs,0,10,0.05,0.05)==summary['result']
    analysis_path=scratch/(name+'-analysis.json')
    subprocess.run([sys.executable,'tools/analyze_match.py','--pgn',str(pgn),'--json',str(analysis_path)],check=True)
    assert json.loads(analysis_path.read_text())==json.loads((root/(name+'-analysis.json')).read_text())
    total += summary['result']['games']
    print(name, summary['result']['games'], 'games: hashes and statistics verified')

for name in ['training','development']:
    subprocess.run([str(binaries/'refit-tune-base'),'extract','--pgn',str(scratch/(name+'.pgn')),'--skip-plies','16','--out',str(scratch/(name+'.raw.txt'))],check=True)
for name in ['historical.keys','training.keys','development.keys','confirmation.keys']:
    shutil.copyfile(root/name,scratch/name)
environment=dict(os.environ, JAKKOO_TEMP=str(binaries))
subprocess.run([sys.executable,str(helpers/'prepare-refit-data.py'),'--directory',str(scratch)],check=True,env=environment)
assert json.loads((scratch/'sample-audit.json').read_text())==json.loads((root/'sample-audit.json').read_text())
for name in ['training','development']:
    assert (scratch/(name+'.filtered.txt')).read_bytes()==gzip.decompress((root/(name+'.filtered.txt.gz')).read_bytes())

for name in models:
    converted=scratch/(name+'.weights.tsv')
    subprocess.run([sys.executable,str(helpers/'refit-convert.py'),str(root/(name+'.fit.rs')),'--layout',str(root/'layout.tsv'),'--out',str(converted)],check=True)
    assert converted.read_bytes()==(root/(name+'.weights.tsv')).read_bytes()==(root/(name+'-applied.tsv')).read_bytes()
    for channel in ['training','development']:
        measured=json.loads(subprocess.check_output([str(binaries/'refit-score'),'evaluate',str(scratch/(channel+'.filtered.txt')),str(converted),str(root/'calibration.k')],text=True))
        expected=json.loads((root/(name+'.'+channel+'-loss.json')).read_text())
        assert measured['positions']==expected['positions'] and abs(measured['mse']-expected['mse'])<1e-12
    print(name, 'emitted-model loss verified')

original=json.loads((root/'provenance.json').read_text())['original_source_sha256']
for name,digest in original.items():
    assert sha(Path(name).read_bytes())==digest, name
assert not json.loads((root/'provenance.json').read_text())['confirmation_played']
print(f'Audit passed: {total} games; deterministic books and sample reproduction; independent data partitions; all emitted model losses; original engine weights retained')
