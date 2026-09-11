import json, os, re, sys
from pathlib import Path
sys.path.insert(0, str(Path.cwd()))
from tools.measure_style import UciEngine, parse_suite, sha256_file

class RecordingEngine(UciEngine):
    def read_until(self, predicate):
        self.last_output = super().read_until(predicate)
        return self.last_output

def tree(engine, fixture, profile):
    observation = engine.measure(fixture, profile)
    trace = [re.sub(r'\b(?:time|nps) \d+ ?', '', line).strip()
             for line in engine.last_output
             if line.startswith(('info depth ', 'bestmove ', 'info string personality '))]
    return {'bestmove': observation.bestmove, 'score': observation.score,
            'depth': observation.depth, 'nodes': observation.nodes, 'trace': trace}

root = Path(os.environ['JAKKOO_TEMP'])
candidate = root / 'jakgro-fitted-final'
baseline = root / 'jakgro-baseline'
rows = []
with RecordingEngine(candidate, 60) as c, RecordingEngine(baseline, 60) as b:
    for suite in ['personality', 'standard-attacks', 'sacrifice-gates']:
        for fixture in parse_suite(Path(f'tests/data/{suite}.epd')):
            for profile in [0, 100]:
                left = tree(c, fixture, profile)
                right = tree(b, fixture, profile)
                rows.append({'suite': suite, 'id': fixture.identifier,
                             'aggression': profile, 'identical': left == right,
                             'candidate': left, 'baseline': right})
summary = {'candidate_sha256': sha256_file(candidate), 'baseline_sha256': sha256_file(baseline),
           'comparisons': len(rows), 'identical': sum(r['identical'] for r in rows), 'rows': rows}
(root / 'fitted-endpoint-tree-equality.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps({k:v for k,v in summary.items() if k != 'rows'}))
for r in rows:
    if not r['identical']:
        print(json.dumps(r))
assert all(r['identical'] for r in rows)
