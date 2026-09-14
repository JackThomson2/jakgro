import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

parser = argparse.ArgumentParser()
parser.add_argument('candidate')
args = parser.parse_args()
root = Path(os.environ['JAKKOO_TEMP']) / 'eval-strength'
engine = root / 'bin' / args.candidate
commands = {
    'endpoints': ['tools/measure_style.py', '--check'],
    'sacrifice': ['tools/measure_style.py', '--suite', 'tests/data/sacrifice-gates.epd', '--check'],
    'acceptance': ['tools/measure_acceptance.py', '--suite', 'tests/data/standard-acceptance.epd', '--selected-profile', '75', '--check'],
}
results = {}
for name, arguments in commands.items():
    target = root / 'data' / (args.candidate + '.' + name)
    with target.with_suffix(target.suffix + '.log').open('w') as log:
        result = subprocess.run([sys.executable, *arguments, '--engine', str(engine), '--summary-json', str(target.with_suffix(target.suffix + '.json'))], stdout=log, stderr=subprocess.STDOUT)
    results[name] = result.returncode
    print(name, result.returncode)
    for line in target.with_suffix(target.suffix + '.log').read_text().splitlines():
        if any(word in line for word in ('FAIL', 'mismatch', 'error')):
            print(line)
(root / 'data' / (args.candidate + '.gates.json')).write_text(json.dumps(results, indent=2) + '\n')
print(json.dumps(results))
