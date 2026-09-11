import argparse
import json
import shutil
import subprocess
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('--out', required=True, type=Path)
args = parser.parse_args()
out = args.out.resolve()
out.mkdir(parents=True, exist_ok=True)
here = Path(__file__).resolve().parent
subprocess.run(['cargo', 'build', '--release', '--locked', '--bin', 'jakgro', '--bin', 'selfplay'], check=True)
shutil.copyfile('target/release/jakgro', out/'refit-base')
shutil.copymode('target/release/jakgro', out/'refit-base')
shutil.copyfile('target/release/selfplay', out/'selfplay')
shutil.copymode('target/release/selfplay', out/'selfplay')
result = subprocess.run(['cargo', 'build', '--release', '--locked', '--features', 'tuning', '--bin', 'tune', '--message-format=json'], check=True, text=True, stdout=subprocess.PIPE)
artifacts = []
for line in result.stdout.splitlines():
    try:
        item = json.loads(line)
    except json.JSONDecodeError:
        continue
    if item.get('reason') == 'compiler-artifact':
        artifacts.append(item)

def library(name):
    artifact = next(x for x in artifacts if x['target']['name'] == name and 'lib' in x['target']['kind'])
    return next(f for f in artifact['filenames'] if f.endswith('.rlib'))

cozy, jakgro = library('cozy_chess'), library('jakgro')
shutil.copyfile('target/release/tune', out/'refit-tune-base')
shutil.copymode('target/release/tune', out/'refit-tune-base')
for name in ['refit-books', 'refit-score']:
    command = ['rustc', '--edition=2024', '-O', '-C', 'panic=abort', '-C', 'lto=fat', '-C', 'codegen-units=1', str(here/(name+'.rs')), '--extern', 'cozy_chess='+cozy, '-L', 'dependency=target/release/deps', '-o', str(out/name)]
    if name == 'refit-score':
        command += ['--extern', 'jakgro='+jakgro]
    subprocess.run(command, check=True)
print('Built frozen engine/fitter and both helpers in', out)
