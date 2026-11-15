#!/usr/bin/env python3
"""評価対象の保存ソース・実行バイナリ・ランチャーを記録と照合する。"""
import argparse
import hashlib
import json
import re
from pathlib import Path
from identity import digest, identity

ROOT = Path(__file__).resolve().parents[3]

def require(condition, message):
    if not condition:
        raise ValueError(message)

def verify(roster_path, remote_snapshot=None):
    roster = json.loads(Path(roster_path).read_text())
    baseline_manifest = json.loads((ROOT / 'solver/docs/parallel-pool/baseline-manifest.json').read_text())
    baseline_files = baseline_manifest['files']
    require(hashlib.sha256(json.dumps(baseline_files, sort_keys=True).encode()).hexdigest() == roster['baseline_hash'], '共通基準の識別記録相違')
    for name, expected in baseline_files.items():
        require(digest(ROOT / 'solver/parallel-pool/baseline' / name) == expected, f'共通基準の入力変更: {name}')
    members = roster['roster']
    require(sorted(e['id'] for e in members) == ['P0', 'P1', 'P2', 'P3', 'S0', 'S1'], '6体のID相違')
    require(sorted(e['team'] for e in members) == list(range(6)), 'チーム番号重複/欠落')
    remote_hashes = {}
    if remote_snapshot:
        for row in json.loads(Path(remote_snapshot).read_text()):
            remote_hashes[row['host']] = dict((path.rstrip('\r'), h) for h, path in re.findall(r'(?m)^([0-9a-f]{64})  (.+)$', row['output']))
    report = []
    for member in members:
        slot = member['id']
        if slot.startswith('P'):
            old = json.loads((ROOT / member['source_identity']).read_text())
            now = identity(member['path'])
            require(now == old, f'{slot}: 保存後のソースまたはバイナリ変更')
            require(now['sourceHash'] == member['source_hash'], f'{slot}: ソース識別相違')
            require(now['binaryHash'] == member['binary_sha256'], f'{slot}: バイナリ識別相違')
        else:
            for name, expected in member['source_hashes'].items():
                # 保護領域は識別用SHA256計算のみ。内容は参照しない。
                require(digest(ROOT / name) == expected, f'{slot}: 固定保存版の識別相違')
        launcher = ROOT / member.get('launcher_source', f'solver/parallel-pool/runtime/{slot}/launcher.sh')
        require(digest(launcher) == member['launcher_sha256'], f'{slot}: ランチャー識別相違')
        if remote_snapshot:
            user = {'192.168.0.11': 'voidlinux-1', '192.168.0.22': 'voidlinux-2', '192.168.0.33': 'voidlinux-3'}[member['host']]
            remote_root = f'/home/{user}/procon37-parallel-pool'
            hashes = remote_hashes.get(member['host'], {})
            for name, expected in [('solver', member['binary_sha256']), ('launcher', member['launcher_sha256'])]:
                require(hashes.get(f'{remote_root}/teams/{slot}/{name}') == expected, f'{slot}: リモート{name}識別相違')
            for local, remote in [('target/release/server-bridge', 'bin/server-bridge'), ('tester/server-bridge/run.sh', 'tester/server-bridge/run.sh')]:
                require(hashes.get(f'{remote_root}/{remote}') == roster['runtime_components'][local], f'{slot}: 共通実行部品の識別相違')
        report.append(slot)
    for name, expected in roster['runtime_components'].items():
        require(digest(ROOT / name) == expected, f'実行基盤の識別相違: {name}')
    return {'verified': report, 'remoteSnapshotChecked': remote_snapshot is not None,
            'note': '実評価後は親が新しく取得したリモートハッシュ記録で再確認する。正式12件と全員ACの確認は別途必要。'}

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('roster')
    parser.add_argument('--remote-snapshot')
    args = parser.parse_args()
    print(json.dumps(verify(args.roster, args.remote_snapshot), ensure_ascii=False, indent=2))
