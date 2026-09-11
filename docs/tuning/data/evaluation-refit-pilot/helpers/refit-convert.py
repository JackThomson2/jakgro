import argparse
import csv
import re
import sys
from pathlib import Path
sys.path.insert(0, str(Path.cwd()))
from tools.splice_weights import parse_fit, _find_declaration

parser = argparse.ArgumentParser()
parser.add_argument('fit', nargs='?', type=Path)
parser.add_argument('--layout', required=True, type=Path)
parser.add_argument('--source', action='store_true')
parser.add_argument('--out', required=True, type=Path)
args = parser.parse_args()
blocks = list(csv.DictReader(args.layout.open(), delimiter='\t'))
if args.source:
    parsed = {}
    sources = {p: Path(p).read_text().splitlines() for p in ['src/engine/evaluation/weights.rs', 'src/engine/evaluation/placement.rs']}
    for block in blocks:
        kind, name = block['kind'], block['name']
        path = 'src/engine/evaluation/placement.rs' if kind == 'table' else 'src/engine/evaluation/weights.rs'
        start, end = _find_declaration(sources[path], kind, name)
        parsed[kind, name] = '\n'.join(sources[path][start:end])
else:
    assert args.fit is not None
    parsed = {}
    for declarations in parse_fit(args.fit.read_text()).values():
        for kind, name, body in declarations:
            assert (kind, name) not in parsed
            parsed[kind, name] = '\n'.join(body)

count = max(int(b['offset']) + int(b['len']) for b in blocks)
weights = [None] * count
used = set()
for block in blocks:
    kind, name = block['kind'], block['name']
    offset, length = int(block['offset']), int(block['len'])
    text = parsed[kind, name]
    if kind == 'table':
        columns = []
        for phase in ['middle_game', 'end_game']:
            m = re.search(rf'{phase}:\s*\[(.*?)\]', text, re.S)
            assert m, (kind, name, phase)
            body = re.sub(r'//[^\n]*', '', m.group(1))
            columns.append([int(x) for x in re.findall(r'-?\d+', body)])
        assert len(columns[0]) == len(columns[1]) == length
        pairs = list(zip(*columns))
    else:
        pairs = [tuple(map(int, m)) for m in re.findall(r'ScorePair::new\(\s*(-?\d+)\s*,\s*(-?\d+)\s*\)', text)]
        repeat = re.search(r'\[\s*ScorePair::new\([^)]*\)\s*;\s*(\d+)\s*\]', text)
        if repeat:
            assert len(pairs) == 1
            pairs *= int(repeat.group(1))
        assert len(pairs) == length, (name, len(pairs), length)
    assert all(weights[i] is None for i in range(offset, offset + length))
    weights[offset:offset + length] = pairs
    used.add((kind, name))
assert all(w is not None for w in weights)
assert set(parsed) == used
args.out.write_text('index\tmg\teg\n' + ''.join(f'{i}\t{mg}\t{eg}\n' for i, (mg, eg) in enumerate(weights)))
print(f'Converted {len(blocks)} blocks, {len(weights)} MG/EG pairs to {args.out}')
