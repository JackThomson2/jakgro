import dataclasses
import json
import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path.cwd()))
from tools.measure_style import Fixture, UciEngine, parse_suite
from tools.validate_acceptance_contract import parse_epd

scratch = Path(os.environ['JAKKOO_TEMP'])
rows = []
fixtures = parse_suite(Path('tests/data/personality.epd'))
fixtures += parse_suite(Path('tests/data/sacrifice-gates.epd'))
fixtures += parse_suite(Path('tests/data/search-performance.epd'))
with UciEngine(scratch / 'strength-base', 60) as base, UciEngine(scratch / 'strength-pv-lmr', 60) as pilot, UciEngine(scratch / 'strength-pv75', 60) as final:
    for profile in [0, 25, 50, 74, 75, 76, 80, 99, 100]:
        reference = pilot if profile == 75 else base
        for fixture in fixtures:
            before = dataclasses.asdict(reference.measure(fixture, profile))
            after = dataclasses.asdict(final.measure(fixture, profile))
            keys = ['bestmove', 'score', 'depth', 'nodes', 'personality']
            equal = all(before[k] == after[k] for k in keys)
            rows.append({'profile': profile, 'id': fixture.identifier, 'reference': 'pilot' if profile == 75 else 'base', 'before': before, 'after': after, 'equal': equal})

out = scratch / 'pv75-scope-equality.json'
out.write_text(json.dumps(rows, indent=2) + '\n')
print('Scoped comparison:', sum(row['equal'] for row in rows), '/', len(rows), 'identical')
for row in rows:
    if not row['equal']:
        print('DIFFERENCE', row)
assert all(row['equal'] for row in rows)

rows = []
with UciEngine(scratch / 'strength-pv75', 60) as engine:
    for name in ['tactics', 'defense', 'style', 'transpositions']:
        for fen, fields in parse_epd(Path('tests/data') / (name + '.epd')):
            fixture = Fixture(fields['id'], fields['category'], fen, int(fields['nodes']), {})
            observations = [dataclasses.asdict(engine.measure(fixture, 75)) for _ in range(2)]
            rows.append({'id': fixture.identifier, 'expected': {'bestmove': fields['bm'], 'score': fields['score']}, 'observations': observations})
            print(fixture.identifier, observations[0]['bestmove'], observations[0]['score'], observations[0]['depth'])
(scratch / 'pv75-final-regressions.json').write_text(json.dumps(rows, indent=2) + '\n')
