import dataclasses
import json
import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path.cwd()))
from tools.measure_style import Fixture, UciEngine
from tools.validate_acceptance_contract import parse_epd

scratch = Path(os.environ['JAKKOO_TEMP'])
rows = []
for label in ['base', 'pv-lmr']:
    with UciEngine(scratch / ('strength-' + label), 60) as engine:
        for name in ['tactics', 'defense', 'style', 'transpositions']:
            for fen, fields in parse_epd(Path('tests/data') / (name + '.epd')):
                fixture = Fixture(fields['id'], fields['category'], fen, int(fields['nodes']), {})
                observations = [dataclasses.asdict(engine.measure(fixture, 75)) for _ in range(2)]
                rows.append({'binary': label, 'kind': 'regression', 'id': fixture.identifier,
                             'expected': {'bestmove': fields['bm'], 'score': fields['score']},
                             'observations': observations})
        suites = parse_epd(Path('tests/data/standard-acceptance.epd'))
        suites.append(('rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1',
                       {'id': 'starting-development', 'category': 'initiative'}))
        choices = {
            'starting-development': ['b1c3', 'd2d4'],
            'standard-open-king-gambit': ['c3d5', 'c1g5'],
            'standard-opposite-castle-storm': ['f1e1', 'b2b4', 'f3e5', 'd2b3'],
        }
        for fen, fields in suites:
            if fields['id'] not in choices:
                continue
            for nodes in [100_000, 400_000, 2_000_000]:
                fixture = Fixture(fields['id'], fields['category'], fen, nodes, {})
                free = {str(profile): dataclasses.asdict(engine.measure(fixture, profile))
                        for profile in [0, 75, 100]}
                restricted = {move: dataclasses.asdict(engine.measure(fixture, 0, frozenset({move})))
                              for move in choices[fields['id']]}
                rows.append({'binary': label, 'kind': 'deeper-choice', 'id': fixture.identifier,
                             'nodes': nodes, 'free': free, 'restricted': restricted})

out = scratch / 'pv-lmr-regression-probes.json'
out.write_text(json.dumps(rows, indent=2) + '\n')
for row in rows:
    if row['kind'] == 'regression':
        observed = row['observations'][0]
        changed = any(observed[key] != value for key, value in row['expected'].items())
        print(row['binary'], row['id'], 'changed' if changed else 'unchanged',
              observed['bestmove'], observed['score'], 'depth', observed['depth'])
    else:
        print(row['binary'], row['id'], row['nodes'],
              'free', {k: (v['bestmove'], v['score'], v['depth']) for k, v in row['free'].items()},
              'restricted', {k: (v['score'], v['depth']) for k, v in row['restricted'].items()})
