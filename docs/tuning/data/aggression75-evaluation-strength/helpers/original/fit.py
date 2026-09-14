import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

parser = argparse.ArgumentParser()
parser.add_argument('family', choices=['mobility', 'safety', 'threats'])
parser.add_argument('label', choices=['1', '0.5'])
parser.add_argument('--representation', default='raw')
args = parser.parse_args()
root = Path(os.environ['JAKKOO_TEMP']) / 'eval-strength'
data = root / 'data'
name = f'{args.representation}-{args.family}-{args.label}'
binary = root / 'bin' / ('refit-tune-base' if args.representation == 'raw' else args.representation + '-tune')
scorer = root / 'bin' / ('refit-score' if args.representation == 'raw' else args.representation + '-score')
command = [str(binary), 'fit', '--positions', str(data / 'training.txt'), '--out', str(data / (name + '.fit.rs')), '--holdout', '0', '--lambda', args.label, '--epochs', '100', '--rate', '0.25', '--l2', '1e-6', '--min-observations', '2000', '--hold', (data / (args.family + '.hold')).read_text()]
started = time.monotonic()
with (data / (name + '.fit.log')).open('w') as log:
    result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT)
record = {'name': name, 'command': command, 'exit_status': result.returncode, 'elapsed_seconds': time.monotonic() - started, 'fitter_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'representation': args.representation}
if result.returncode != 0:
    (data / (name + '.record.json')).write_text(json.dumps(record, indent=2) + '\n')
    print((data / (name + '.fit.log')).read_text())
    raise SystemExit(result.returncode)
subprocess.run([sys.executable, 'docs/tuning/data/evaluation-refit-pilot/helpers/refit-convert.py', str(data / (name + '.fit.rs')), '--layout', str(data / 'layout.tsv'), '--out', str(data / (name + '.tsv'))], check=True)
load = lambda path: [(int(row['mg']), int(row['eg'])) for row in csv.DictReader(path.open(), delimiter='\t')]
base, fitted = load(data / 'base.tsv'), load(data / (name + '.tsv'))
changes = [i for i, (before, after) in enumerate(zip(base, fitted)) if before != after]
blocks = list(csv.DictReader((data / 'layout.tsv').open(), delimiter='\t'))
held = set((data / (args.family + '.hold')).read_text().split(','))
for block in blocks:
    if block['name'] in held:
        lo, length = int(block['offset']), int(block['len'])
        assert base[lo:lo + length] == fitted[lo:lo + length], block
record['changed_pairs'] = len(changes)
record['max_coordinate_delta'] = max((abs(a - b) for before, after in zip(base, fitted) for a, b in zip(before, after)), default=0)
record['changed_blocks'] = [block['name'] for block in blocks if any(int(block['offset']) <= i < int(block['offset']) + int(block['len']) for i in changes)]
record['emitted_sha256'] = hashlib.sha256((data / (name + '.tsv')).read_bytes()).hexdigest()
for channel in ('training', 'development'):
    output = subprocess.run([str(scorer), 'evaluate', str(data / (channel + '.txt')), str(data / (name + '.tsv')), str(data / 'calibration.k')], text=True, check=True, capture_output=True).stdout
    (data / (name + '.' + channel + '.json')).write_text(output)
    record[channel] = json.loads(output)
record['deltas'] = [{'index': i, 'before': base[i], 'after': fitted[i]} for i in changes]
(data / (name + '.record.json')).write_text(json.dumps(record, indent=2) + '\n')
print(json.dumps({key: value for key, value in record.items() if key not in ('command', 'deltas')}, indent=2))
print((data / (name + '.fit.log')).read_text())
