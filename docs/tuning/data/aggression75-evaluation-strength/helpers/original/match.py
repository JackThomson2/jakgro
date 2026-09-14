import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time
sys.path.insert(0, str(Path.cwd()))
from tools import analyze_match, run_sprt

parser = argparse.ArgumentParser()
parser.add_argument('candidate')
parser.add_argument('--name')
parser.add_argument('--games', type=int, default=1024)
parser.add_argument('--movetime', type=int, default=50)
parser.add_argument('--clock')
parser.add_argument('--aggression', type=int, default=75)
parser.add_argument('--book', default='docs/tuning/data/evaluation-refit-pilot/development.epd')
parser.add_argument('--concurrency', type=int, default=16)
args = parser.parse_args()
root = Path(os.environ['JAKKOO_TEMP']) / 'eval-strength'
name = args.name or args.candidate + '-screen'
pgn = root / 'runs' / (name + '.pgn')
assert not pgn.exists(), pgn
engine = root / 'bin' / args.candidate
baseline = root / 'bin/refit-base'
runner = root / 'bin/selfplay'
book = Path(args.book).resolve()
inputs = [engine, baseline, runner, book, Path('tools/run_sprt.py'), Path('tools/analyze_match.py')]
hashes = {str(path): hashlib.sha256(path.read_bytes()).hexdigest() for path in inputs}
command = [sys.executable, 'tools/run_sprt.py', '--engine', str(engine), '--baseline-engine', str(baseline), '--runner', str(runner), '--candidate-name', args.candidate, '--baseline-name', 'Base9fda9ac', '--candidate-aggression', str(args.aggression), '--baseline-aggression', str(args.aggression), '--games', str(args.games), '--concurrency', str(args.concurrency), '--hash', '16', '--elo0', '0', '--elo1', '10', '--openings', str(book), '--baseline-revision', '9fda9ac72e63564e9f164c612376f28d0b549216', '--build-profile', 'release-locked-portable', '--pgn', str(pgn)]
command += ['--time-control', args.clock] if args.clock else ['--movetime-ms', str(args.movetime)]
record = {'command': command, 'started_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'hashes_before': hashes}
protocol = root / 'runs' / (name + '.protocol.json')
protocol.write_text(json.dumps(record, indent=2) + '\n')
start = time.monotonic()
with pgn.with_suffix('.log').open('w') as log:
    result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT)
record['elapsed_seconds'] = time.monotonic() - start
record['exit_status'] = result.returncode
record['hashes_after'] = {str(path): hashlib.sha256(path.read_bytes()).hexdigest() for path in inputs}
record['inputs_unchanged'] = record['hashes_before'] == record['hashes_after']
(root / 'runs' / (name + '.execution.json')).write_text(json.dumps(record, indent=2) + '\n')
assert record['inputs_unchanged']
if result.returncode:
    print(pgn.with_suffix('.log').read_text()[-5000:])
    raise SystemExit(result.returncode)
assert pgn.with_suffix('.arbiter.json').is_file()
summary = json.loads(pgn.with_suffix('.sprt.json').read_text())
assert summary['status'] == 'complete' and not summary['faults']
games = analyze_match.parse_pgn(pgn)
manifest, candidate, opponent, _ = analyze_match.load_manifest(pgn.with_suffix('.manifest.json'), pgn)
analysis = analyze_match.summarize(games, manifest, candidate, opponent, pgn, pgn.with_suffix('.manifest.json'))
pgn.with_suffix('.analysis.json').write_text(json.dumps(analysis, indent=2) + '\n')
expected = run_sprt.evaluate(run_sprt.pair_points_from_pgn(pgn, candidate, opponent), 0, 10, 0.05, 0.05)
assert expected == summary['result']
print(json.dumps({'name': name, 'elapsed_seconds': record['elapsed_seconds'], 'result': summary['result'], 'style': analysis['style'], 'counts': analysis['result'], 'confidence': analysis['confidence']}, indent=2))
