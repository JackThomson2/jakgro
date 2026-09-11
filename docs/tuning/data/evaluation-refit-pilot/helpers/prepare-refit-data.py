import argparse
import hashlib
import json
import os
import subprocess
from pathlib import Path


def clean(lines, keys, forbidden):
    assert len(lines) == len(keys)
    retained, retained_keys = [], []
    seen = set()
    dropped = {'forbidden_position': 0, 'legal_en_passant': 0, 'duplicate_position': 0}
    for line, key in zip(lines, keys):
        fields = key.split()
        assert len(fields) == 4
        if key in forbidden:
            dropped['forbidden_position'] += 1
        elif fields[3] != '-':
            dropped['legal_en_passant'] += 1
        elif key in seen:
            dropped['duplicate_position'] += 1
        else:
            seen.add(key)
            retained.append(line)
            retained_keys.append(key)
    assert len(retained) + sum(dropped.values()) == len(lines)
    return retained, retained_keys, dropped


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--self-test', action='store_true')
    parser.add_argument('--directory', type=Path)
    args = parser.parse_args()
    if args.self_test:
        result = clean(['a', 'b', 'c', 'd', 'e'], ['A w - -', 'A w - -', 'B w - e3', 'C w - -', 'D w - -'], {'C w - -'})
        assert result == (['a', 'e'], ['A w - -', 'D w - -'], {'forbidden_position': 1, 'legal_en_passant': 1, 'duplicate_position': 1})
        print('Sample filtering self-test passed')
        return
    directory = args.directory or Path(os.environ['JAKKOO_TEMP']) / 'refit-data'
    helper = Path(os.environ['JAKKOO_TEMP']) / 'refit-books'
    keysets = {name: set((directory / (name + '.keys')).read_text().splitlines()) for name in ['historical', 'training', 'development', 'confirmation']}
    report = {}
    training = set()
    for name in ['training', 'development']:
        raw = directory / (name + '.raw.txt')
        keypath = directory / (name + '.raw.keys')
        subprocess.run([str(helper), 'keys', str(raw), str(keypath)], check=True)
        lines = raw.read_text().splitlines()
        keys = keypath.read_text().splitlines()
        forbidden = keysets['historical'] | keysets['confirmation']
        forbidden |= keysets['development'] if name == 'training' else training
        kept, kept_keys, dropped = clean(lines, keys, forbidden)
        assert kept
        output = directory / (name + '.filtered.txt')
        output.write_text('\n'.join(kept) + '\n')
        (directory / (name + '.sample.keys')).write_text('\n'.join(kept_keys) + '\n')
        if name == 'training':
            training = set(kept_keys)
        else:
            assert not set(kept_keys) & training
        assert not set(kept_keys) & keysets['confirmation']
        report[name] = {'raw_samples': len(lines), 'retained_samples': len(kept), 'dropped': dropped,
                        'raw_sha256': hashlib.sha256(raw.read_bytes()).hexdigest(),
                        'retained_sha256': hashlib.sha256(output.read_bytes()).hexdigest()}
    (directory / 'sample-audit.json').write_text(json.dumps(report, indent=2, sort_keys=True) + '\n')
    print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == '__main__':
    main()
