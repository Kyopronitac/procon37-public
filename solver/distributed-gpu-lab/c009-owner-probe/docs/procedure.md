# VOIDLINUX-2 owner GPUの有限比較計画

## 状態と識別

初稿作成時は未ビルドだったが、C006短縮評価完了後にCPU 3でビルドとCPU-only検査を実施した。release build 11.96秒、CPU-only 2.069秒、6組全一致・exit0・正常終了・子PID残存なし。記録は隣接`local-validation-20261009-r1`に保存した。実機操作は親担当が評価間の開始を指示してから実行する。

読込元は`solver/distributed-candidates/c009-gpu-owner`、source aggregate SHA-256は`9f8e182d3573dfc3514502c75ba8a4f11bb9c39be8f2dc2e4294ca675d4b533a`。その47ファイルを変更せず、`include!`で実際のprivate builderを同一moduleに読込み、測定処理だけを追加する。C009本体のCPU検証とlocalhost通信検証は完了済みだが、この測定器は別binaryである。補助ソースの識別は`source-manifest.json`、ビルド後のbinaryと転送ファイルは別の`deployment-manifest.json`へ記録する。

## 測定範囲

- VOIDLINUX-2だけで1プロセス・1context・合計6 GPU owner要求。seed 0/1/2の公開generator、4人、非公式練習場fixtureを含む既定設定を使う。16/24/32格子に対応するが、実際のcell/target/source数は結果から確認する。
- 初期化を独立計測し、Readyを待ってからCPU off/GPU onを交互に測る。各形式の1組目はCPU→GPU、2組目はGPU→CPU。CPU baselineに初期化負荷を重ねない。
- CPU offは同じプロセス内で直接`GraphTables::build`を呼ぶ。GPU actorはReadyで待機中。GPU未作成プロセスとの比較ではない。GPU側は本体の`try_build`を呼ぶため、入力変換、queue/wait、device buffer確保、転送、kernel、readback、全表検証、CPU tree DP、Graph組立を含む。
- GPU側のdeadlineは本体と同じ式で、request残り3秒、cap250ms、CPU reserveあり。private bypass/共有cache publicationは対象外。GPU失敗時は本体同様CPU fallbackを計測に含めるが、1回でもfallbackすれば「6要求GPU成功」の検証は失敗扱いとする。自動再試行しない。
- CPUとGPUのGraph aux表・全next/fingerprint・全Path scalarを追加照合する。この照合はowner timingの外、プログラム全体wall/CPUには含む。最初のGPU要求にdeferred JITがあれば初回費用として必ず残す。
- 15本の既存thread相当の**idle sentinel**を先に作成し、producerの自己TID移動でmain/sentinelが移動しないことを確認する。これは15探索threadの負荷試験ではない。
- 終了時間も独立計測し、producer状態が`Stopped`かつerrorなし、sentinelのjoin、main配置不変を確認する。正常なプロセス終了だけでdriverの正常joinと同一視しない。

## ビルド前後の手順

リポジトリrootを実際に確認してから、評価間に以下を実行する。CPU 3・cargo `-j1`を使い、C009の既存targetを再利用する。最初はlock作成が必要なので`--locked`は付けない。依存は既存registryのみ。

```sh
. .server/toolchain/env.sh
taskset -c 3 cargo build --release --offline --manifest-path solver/distributed-gpu-lab/c009-owner-probe/Cargo.toml --target-dir solver/distributed-candidates/c009-gpu-owner/target -j1
timeout --signal=TERM --kill-after=2s 20s taskset -c 3 solver/distributed-candidates/c009-gpu-owner/target/release/procon37-c009-owner-probe --cpu-only
```

CPU-onlyではGPUをロードせず、同一CPU builderを二回ずつ呼んで測定・同値照合の経路を確認する。probe_start/probe_endと全pair、exit、残存PIDを保存する。ビルド後にprobeのCargo.lockをmanifestへ加え、元C009の47 sourceとproduction binaryが変わっていないことを検証する。補助binaryのSHA-256も記録する。

専用deploymentにはprobe binaryを`owner-probe`として置くほか、C009の47ファイル、source-manifest、probe一式を元の相対配置で保存する。既存`~/procon37-distributed`直下に今回だけの`c009-owner-probe-<識別子>`を新規作成し、既存worker・設定・実行ファイルを触らない。全転送ファイルのSHA-256を`deployment-manifest.json`の`sha256`辞書へ入れる。認証値は保存しない。

## 実機実行と回収

開始直前に親担当へ連絡し、v2の既存ソルバがidleで対戦未実行であることを確認する。ユーザーのSSH操作許可の範囲で、今回のroot controllerを1回だけ起動する。`DEPLOY`は実在する新規deployment絶対パス、`LABEL`は未使用英数字等の識別子へ置換する。

```sh
sudo -S -p 'Password:' timeout --signal=TERM --kill-after=10s 65s python3 "$DEPLOY/solver/distributed-gpu-lab/c009-owner-probe/tools/run_v2.py" --deployment "$DEPLOY" --label "$LABEL"
```

outerは65秒でTERM、さらに10秒でKILLの最大75秒。controller自身のmonitorは45秒、通常cleanupは8秒以内、empty group cleanupは5秒以内。TERMを受けたcontrollerは例外経由で所有process groupとadopted descendantsを回収する。KILLまで必要なら成功と扱わず、再試行前に原因と残存状態を親へ報告する。

controllerはroot・VOIDLINUX-2・専用deployment名と全ファイルSHAを検証する。今回専用`/sys/fs/cgroup/cpuset/procon37-c009-v2-<LABEL>`だけを新規作成し、cpuset CPU15とmems等の制御はroot管理、`tasks`だけvoidlinux-2へ0600で委譲する。probeはvoidlinux-2 UID・CPU0–15で起動。C009 producerが自分のTIDだけを専用groupへ移し、その後driverを初期化する。既存main/sentinelは元group・CPU0–15のまま。

monitorはCPU0、10ms間隔でprobe/子孫のthreadを読み、producerがgate前に一時CPU0–15にいる場合以外は、driverとproducerのCPU15/group一致・全UID一致を要求する。driver CPUは観測thread ticksの下限、全process CPUはprobe内の`/proc/self/stat`で別記録する。kernel内部threadや観測間隔内に消えたthreadを完全に測れるものではない。

正常条件は全6pair同値・GPU fallbackなし・producer正常停止・main/sentinel不変・exit0・CPU制約違反なし・強制回収なし・子孫なし・tasks空・group削除。結果は`results-<LABEL>`へ新規保存し、既存結果は上書きしない。終端後、controller/probe/専用groupの残存なしを別途read-onlyで確認して親へ即連絡する。

## 費用と続行判断

ローカルへ結果を回収して`python3 tools/summarize.py <results-dir>`を使い、各形式2組のCPU/GPU owner全費用、初期化・終了、6要求総量、初回要求の内訳、process/driver CPUを確認する。2回の小計測は候補の足切りであり対戦改善の証拠ではない。

初回要求を含めた平均節約量を`s`、独立初期化＋終了時間を`F`として、同じ形式比率を繰り返す保守的な損益分岐を`s > 0`のとき`ceil(F/s)`要求とする。初回JITを除外した数字だけを成果として示さない。

C008 v2の完了済みjobs1–83ではrequests 2685、builds 1404、owner 483、private bypass 921、wait timeout/failure 0。ownerはbuildの約34.4%、requestの約18.0%なのでowner-onlyの上限は限られる。`483*s-F`は全ownerがGPUを使え、入力比率も同じという仮定での参考値にすぎず、busy/readiness/deadlineで受理率は下がる。v2 workerは複数試合persistentなので初期化を償却できるが、毎試合再起動するv1 coordinatorへそのまま外挿しない。

実サイズでGPU ownerがCPUより遅い、改善がばらつき程度、初期化やCPU競合を含め利益が薄い、償却要求が現実的owner数を大きく超える場合はGPU候補を保留する。明確な費用削減が見えた場合だけ、次段階の実探索併用と4チーム短縮評価を親担当と調整する。GPU利用自体を採用理由にしない。
