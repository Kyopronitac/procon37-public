#!/usr/bin/env python3
"""候補のソース一式と実行バイナリの内容識別を保存する。"""
import hashlib
import json
import sys
from pathlib import Path

SKIP = {'target', '.git', '.tester', '.game', '.server', '__pycache__'}

def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda: f.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()

def identity(root):
    root = Path(root)
    files = {str(p.relative_to(root)): digest(p) for p in sorted(root.rglob('*'))
             if p.is_file() and not SKIP.intersection(p.relative_to(root).parts)
             and p.name != 'stderr.log'}
    return {'sourceHash': hashlib.sha256(json.dumps(files, sort_keys=True).encode()).hexdigest(),
            'files': files, 'binaryHash': digest(root / 'target/release/solver')}

if __name__ == '__main__':
    print(json.dumps(identity(sys.argv[1]), indent=2))
