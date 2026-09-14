import dataclasses
import json
import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path.cwd()))
from tools.measure_style import Fixture, UciEngine

scratch = Path(os.environ['JAKKOO_TEMP'])
rows = []
positions = [
    ('queen-trade', '4k3/3q4/8/8/8/8/3Q4/4K3 w - - 0 1', ['d2d7', 'd2e2']),
    ('checked-rook-capture', '4k3/8/8/8/8/8/4r3/3QK3 w - - 0 1', ['d1e2', 'e1e2']),
]
for binary in ['eval3-pins', 'eval3-checks']:
    with UciEngine(scratch / binary, 60) as engine:
        for name, fen, moves in positions:
            for nodes in [100_000, 400_000]:
                fixture = Fixture(name, 'probe', fen, nodes, {})
                row = {'binary': binary, 'position': name, 'nodes': nodes,
                       'free': {str(a): dataclasses.asdict(engine.measure(fixture, a)) for a in [0, 75, 100]},
                       'restricted': {move: dataclasses.asdict(engine.measure(fixture, 0, frozenset({move}))) for move in moves}}
                rows.append(row)
                print(json.dumps(row), flush=True)
(scratch / 'eval3-safe-choice-probe.json').write_text(json.dumps(rows, indent=2) + '\n')
