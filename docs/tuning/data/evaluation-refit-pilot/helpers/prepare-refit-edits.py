import csv
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path
sys.path.insert(0, str(Path.cwd()))
from tools.splice_weights import parse_fit, _find_declaration

name = sys.argv[1]
r = Path(os.environ['JAKKOO_TEMP'])
d = r / 'refit-data'
subprocess.run([sys.executable, str(r/'refit-convert.py'), '--layout', str(d/'layout.tsv'), '--source', '--out', str(d/'current-source-weights.tsv')], check=True)
load = lambda path: [(int(x['mg']), int(x['eg'])) for x in csv.DictReader(path.open(), delimiter='\t')]
current, wanted = load(d/'current-source-weights.tsv'), load(d/(name+'.weights.tsv'))
blocks = {(b['kind'], b['name']): b for b in csv.DictReader((d/'layout.tsv').open(), delimiter='\t')}
operations = []
hashes = {}
for path, declarations in parse_fit((d/(name+'.fit.rs')).read_text()).items():
    assert path in {'src/engine/evaluation/weights.rs', 'src/engine/evaluation/placement.rs'}
    source = Path(path).read_text()
    hashes[path] = hashlib.sha256(Path(path).read_bytes()).hexdigest()
    lines = source.splitlines()
    for kind, symbol, body in declarations:
        block = blocks[kind, symbol]
        offset, length = int(block['offset']), int(block['len'])
        if current[offset:offset+length] == wanted[offset:offset+length]:
            continue
        start, end = _find_declaration(lines, kind, symbol)
        old = '\n'.join(lines[start:end])
        replacement = [lines[start].split(' =', 1)[0] + ' = [', *body, '];'] if kind == 'array' else body
        operations.append({'path': path, 'pattern': old, 'rewrite': '\n'.join(replacement)})
assert operations
out = {'candidate': name, 'source_sha256': hashes, 'operations': operations}
(d/(name+'-edits.json')).write_text(json.dumps(out, indent=2)+'\n')
print(f'Prepared {len(operations)} AST declaration rewrites for {name}')
