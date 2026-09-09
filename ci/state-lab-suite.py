#!/usr/bin/env python3
"""Explicit paid suite. Select cases; never called by ordinary cargo tests."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cases', nargs='+', required=True)
    parser.add_argument('--model', choices=['haiku','sonnet'], default='haiku')
    args = parser.parse_args()
    if not os.environ.get('MESIMON_TEST_RUN'):
        parser.error('run through ci/test-run.py')
    results = []
    for case in args.cases:
        run = subprocess.run([sys.executable, '-B', str(Path(__file__).with_name('claude-state-e2e.py')),
                              '--case', case, '--model', args.model], capture_output=True, text=True, timeout=210)
        try:
            data = json.loads(run.stdout)
        except ValueError:
            data = dict(case=case, result='runner_error', error=run.stderr[-2000:])
        row = {key: data.get(key) for key in ('case','result','error','capture','cleanup')}
        results.append(row)
        print(json.dumps(row), flush=True)
    out = Path(__file__).resolve().parents[1] / 'target/state-lab' / ('live-suite-'+time.strftime('%Y%m%d-%H%M%S')+'.json')
    out.write_text(json.dumps(results,indent=2)+'\n')
    return int(any(row['result']!='passed' for row in results))


if __name__ == '__main__':
    sys.exit(main())
