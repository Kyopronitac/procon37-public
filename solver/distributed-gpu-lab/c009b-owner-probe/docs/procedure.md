# C009b有限preflightの次回手順

現時点はコード準備のみ。元C009/source/probe/失敗記録を保持する。C009bではcgroup `tasks`のTID＋改行を1回のwriteで渡し、短いwriteとエラーでは再試行しない。

1. 評価終了後、CPU3/-j1でC009bの3つの`delegated_tid`テストと必要なCPU検証を行い、本体source/binary SHAを保存する。
2. 次のコマンドで補助binaryを作り、20秒CPU-onlyで全6pair・正常終了を確認する。元C009のtargetは使用せず、C009b側targetへ保存する。

```sh
. .server/toolchain/env.sh
taskset -c 3 cargo build --release --offline --locked --manifest-path solver/distributed-gpu-lab/c009b-owner-probe/Cargo.toml --target-dir solver/distributed-candidates/c009b-gpu-owner/target -j1
timeout --signal=TERM --kill-after=2s 20s taskset -c 3 solver/distributed-candidates/c009b-gpu-owner/target/release/procon37-c009b-owner-probe --cpu-only
```

3. 親担当の実機開始指示後、実際のhomeを確認して`~/procon37-distributed/c009b-owner-probe-<新規識別子>`に新規配置する。C009bの47source＋manifest、probe source/lock/manifest、`owner-probe`binaryを識別し、全SHAを`deployment-manifest.json`の`sha256`へ保存する。
4. `LABEL`には`c009b-20261009-r1`など既存と重複しない識別子を使う。既存workerとfixed相手を変更せず、以下を1回だけ実行する。

```sh
sudo -S -p 'Password:' timeout --signal=TERM --kill-after=10s 65s python3 "$DEPLOY/solver/distributed-gpu-lab/c009b-owner-probe/tools/run_v2.py" --deployment "$DEPLOY" --label "$LABEL"
```

rootがCPU15の今回専用`procon37-c009-v2-<LABEL>`を作り、tasksのみUID1000へ0600委譲。candidate環境名`PROCON37_C009_GPU*`とgroup prefixはC009互換を維持する。producer自身TIDのみがjoinし、mainと15 idle sentinelはCPU0–15に残す。45秒monitor、65秒TERM＋10秒KILLの外側最大75秒、全子孫回収・empty group削除を要求する。現時点ではactive search負荷をかけない。

Readyを待ってからseed0/1/2の実サイズをCPU→GPU/GPU→CPU各2組で比較。初回JITを含む全6GPU要求、初期化、終了を保持する。全Graph同値、GPU fallbackなし、main/sentinel不変、UID/CPU/group制約、正常shutdown、exit0、強制回収なし、全子孫とgroupの消失をすべて確認する。join/error文脈から初回失敗の推定も確認する。

結果は新しい`docs/v2-idle-<識別子>`へ回収し、`tools/summarize.py`で集計する。失敗時は繰り返さず原因と残存を報告する。CPU/GPUの僅差だけで採用せず、初期化償却・driver CPU・共有後owner比率の小ささも考慮し、有望なら後続の実探索併用・対戦を検討する。
