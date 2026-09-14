import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import sys
sys.path.insert(0, str(Path.cwd()))
from tools.splice_weights import parse_fit, _find_declaration

parser = argparse.ArgumentParser()
parser.add_argument('name')
args = parser.parse_args()
root = Path(os.environ['JAKKOO_TEMP']) / 'eval-strength'
data = root / 'data'
operations = []
for path, declarations in parse_fit((data / (args.name + '.fit.rs')).read_text()).items():
    assert path in {'src/engine/evaluation/weights.rs', 'src/engine/evaluation/placement.rs'}
    source = Path(path).read_text()
    lines = source.splitlines()
    for kind, symbol, body in declarations:
        start, end = _find_declaration(lines, kind, symbol)
        old = '\n'.join(lines[start:end])
        replacement = [lines[start].split(' =', 1)[0] + ' = [', *body, '];'] if kind == 'array' else body
        new = '\n'.join(replacement)
        if ''.join(old.split()) == ''.join(new.split()):
            continue
        operations.append({'path': path, 'pattern': old, 'rewrite': new})
assert operations
output = {'operations': operations}
(data / (args.name + '.edits.json')).write_text(json.dumps(output, indent=2) + '\n')
print(json.dumps(output, indent=2))
