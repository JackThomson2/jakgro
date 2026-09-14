import csv
import datetime
import gzip
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

root = Path(os.environ['JAKKOO_TEMP']) / 'eval-strength'
data = root / 'data'
data.mkdir(parents=True, exist_ok=True)
archive = Path('docs/tuning/data/evaluation-refit-pilot')
for name in ('training', 'development'):
    payload = gzip.decompress((archive / (name + '.filtered.txt.gz')).read_bytes())
    (data / (name + '.txt')).write_bytes(payload)
    print(name, 'rows', len(payload.splitlines()), 'sha256', hashlib.sha256(payload).hexdigest())
for path in ('src/engine/evaluation.rs', 'src/engine/evaluation/features.rs', 'src/engine/evaluation/weights.rs', 'src/engine/evaluation/placement.rs', 'src/engine/evaluation/tuning.rs'):
    target = root / 'base-source' / path
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(Path(path).read_bytes())
families = {
    'mobility': ['KNIGHT_MOBILITY', 'BISHOP_MOBILITY', 'ROOK_MOBILITY', 'QUEEN_MOBILITY', 'UNSAFE_MOBILITY_BY_PIECE'],
    'safety': ['KING_DANGER_BY_BUCKET', 'SAFE_CHECK_BY_PIECE'],
    'threats': ['THREAT_MINOR_BY_PAWN', 'THREAT_HANGING', 'THREAT_BY_LOWER_VALUE', 'THREAT_BY_PAWN_PUSH'],
}
protocol = {
    'created_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
    'base': '9fda9ac72e63564e9f164c612376f28d0b549216',
    'scope': 'Objective evaluation only. Preserve aggression 75, attacking root rules and verified-sacrifice safeguards.',
    'initial_trials': [{'family': name, 'free_blocks': blocks, 'lambda': [1.0, 0.5], 'epochs': 100, 'rate': 0.25, 'l2': 1e-6, 'min_observations': 2000, 'holdout': 0} for name, blocks in families.items()],
    'representation_trial': 'Curves use pawn-safe mobility; raw attack and style counts unchanged. Refit the same mobility blocks with material/PST held, lambda 1 and .5 at the same settings.',
    'screen': {'games': 1024, 'movetime_ms': 50, 'aggression': [75, 75], 'hash_mib': 16, 'threads': 1, 'concurrency_per_run': 16, 'book': str(archive / 'development.epd'), 'purpose': 'Selection only; all tests reported.'},
    'confirmation': 'Freeze one selected candidate and declare larger match caps before playing. Require positive paired interval, H1, at least 90% forcing retention and no safety regression. Clock evidence reported separately; no promise of a gain.',
}
(root / 'protocol-initial.json').write_text(json.dumps(protocol, indent=2) + '\n')
binaries = root / 'bin'
for command, output in [('layout', 'layout.tsv'), ('weights', 'base.tsv')]:
    result = subprocess.run([str(binaries / 'refit-score'), command], text=True, check=True, capture_output=True)
    (data / output).write_text(result.stdout)
subprocess.run([sys.executable, str(archive / 'helpers/refit-convert.py'), '--source', '--layout', str(data / 'layout.tsv'), '--out', str(data / 'source.tsv')], check=True)
assert (data / 'base.tsv').read_bytes() == (data / 'source.tsv').read_bytes()
for channel, action in [('training', 'calibrate'), ('development', 'evaluate')]:
    result = subprocess.run([str(binaries / 'refit-score'), action, str(data / (channel + '.txt')), str(data / 'base.tsv'), str(data / 'calibration.k')], text=True, check=True, capture_output=True)
    (data / ('base.' + channel + '.json')).write_text(result.stdout)
    print(channel, result.stdout.strip())
layout = list(csv.DictReader((data / 'layout.tsv').open(), delimiter='\t'))
for name, free in families.items():
    names = {row['name'] for row in layout}
    assert set(free) <= names
    hold = ','.join(dict.fromkeys(row['name'] for row in layout if row['name'] not in free))
    (data / (name + '.hold')).write_text(hold)
identities = {path.name: hashlib.sha256(path.read_bytes()).hexdigest() for path in binaries.iterdir() if path.is_file()}
(root / 'baseline-binaries.json').write_text(json.dumps(identities, indent=2) + '\n')
print('scratch', root)
print('frozen binaries', json.dumps(identities))
print('initial protocol', json.dumps(protocol))
