# 有界idle LAN echo計測の準備

`lan_probe.py`は**未実機実行**の計測器。既存D0/C010やharnessを変更しない。対戦中は実行せず、評価間に親担当が確認してから使う。GPU・実探索併用・通信設定変更は行わない。

リポジトリの実位置を自身から求め、既存`.server/distributed/lab-config.json`の4台のuser/addressを既存`solver/distributed/tools/ssh.py`のHOSTSと照合する。NOMLABは設定照合だけで接続しない。認証は既存SSH helperが呼出し環境から取得し、値をsource/docs/結果へ書かない。

`--execute`は既存manageと同じ`.server/distributed/lab.lock`を非blocking排他取得し、その後に`activeEvaluation`・`activeMicrobench`の両方がfalseであることを要求する。setup開始前から全cleanup完了までlockを保持する。並行した評価起動や別lab操作は先に取得した側だけが進む。状態flagだけの確認とremote起動の間に競合する隙間を作らない。既存harnessは変更しない。

```sh
# 計画表示だけ。SSHや測定は開始しない。
python3 solver/distributed-lab/lan_probe.py --run-id lan-idle-20261009-r1
# 対戦終了・親担当の開始確認後だけ。既存run IDは再利用しない。
python3 solver/distributed-lab/lan_probe.py --run-id lan-idle-20261009-r1 --execute
```

## 対象と上限

v1をclient、v2/v3を順にecho相手とする。送信元もv1の固定LAN IPへbindし、serverは各自の固定LAN IP・port49371へbindする。全3台で事前にport bind可能か確認し、使用中なら新規測定プロセスを起動せず終了する。起動時のbindにも成功する必要があり、SO_REUSEADDR/REUSEPORTは使わない。既存listenerを停止しない。

serverが受け付ける接続元はv1のLAN IPだけ。1接続・250frame、payloadは64/3517/6826/11185/32768byteを各50回、片方向最大2,718,000byte。4byte長prefixと本文をそのまま返す。長さ・総量・順番が異なる入力を受け取ったら終了する。許可外の接続は閉じ、16回で終了。クライアントは応答長と全本文の一致を確認する。

CPUは新規probeプロセスだけv1=31、v2=15、v3=31へ固定する。既存solverのaffinityは変更しない。TCP_NODELAYあり、1往復のsend/header/bodyは共通の1秒deadlineを使い、各remote計測プロセスはalarmで全体30秒上限。50回に初回も含め、TCP connect時間を別記録する。

SSH配置段階は30秒予算、測定段階は30秒予算、回収は12秒予算。したがって「30秒」は測定段階の上限で、初回配置を含むcontroller全体は通常それ以上になり得る。各SSH/SCPは最大8秒の絶対timerで制限し、既存execute()のper-read timeoutが認証で延長される場合も捕捉した自身のpexpect子を閉じる。remote serverが起動した後にSSHが失敗しても、remote自身のalarmで終了する。自動再試行はない。

v1 clientは1つのforeground SSH呼出しで両peerを測るため、正常結果を待てる実効上限は8秒（残りphase予算が短ければそれ以下）。30秒はmeasurement全体の予算とremoteの最後の終了保証であり、clientに30秒間の計測時間を与える指定ではない。8秒で終わらなければ失敗として回収し、勝手に延長・再試行しない。

## 所有プロセスの識別と回収

各台の実homeが登録値と一致することを確認し、`~/procon37-distributed-lan-probes/<run-id>`だけを新規作成する。sourceとjobのSHA/UIDを検証し、PID・`/proc`開始tick・boot ID・UID・argvを`process.json`へ保存する。background serverは起動側でも`launch.json`を保存する。新規runの既存結果・識別記録があれば再起動しない。

cleanupはpidfdを取得してから全識別を再照合し、一致する今回のPIDだけTERM、必要時KILLする。無関係なPID・listener・既存solverへsignalを送らない。zombieはargvが空でもPID/starttick/boot/UIDで識別し、実際にreapされるまで残存として扱う。worker本体は子プロセスを生成しない。setupの`ip route` subprocessは1秒上限で待機・回収してから計測を始める。

結果が成功し、識別された全PIDが消失し、強制終了不要だったときだけcontrollerを成功とする。PID再利用で別processに変わった場合はsignalを送らず記録する。回収SSHが失敗した場合はremote alarmによる終了見込みを確定した残存ゼロと扱わず、次のread-only確認を親担当へ引き継ぐ。

## 保存内容と限界

ローカル結果は`.server/distributed/lan-probes/<run-id>/`。config全体や認証値を保存せず、source SHA、各hostのjob/UID/CPU、NIC経路・sysfs speed/duplex/MTU、各サイズ50個の往復値、p50/p95/max、実行時刻、正常/失敗と回収記録を残す。remote側にも同じsource/jobとprocess/result/ログを残し、上書きしない。NIC speedはリンクが報告する値で、実効帯域の測定ではない。

これはPythonによるframed TCP echoのidle往復時間。D0の2ms control loop、C010のJSON full-context encode/decode、候補の再評価、offer→取込、CPU探索との競合を測ったとは扱わない。帯域飽和試験でもない。

後続の競合計測は別の評価間に実施する。同一入力・同一CPU配置の有限D0探索と通信off/onを組にし、各条件のiteration/提出時間とecho分布を併記する。候補本体にはoffer sequenceごとのpublish→enqueue→flush、receiver parse→mailbox→取込→再評価をローカルmonotonicで記録し、network待ちとrestart/取込待ちを分離する。異なるPCのInstantを直接差し引かず、cross-host遅延は同期誤差付きの推定か往復境界として報告する。

## 準備時の軽い確認

2026-10-09、CPU3でsourceと埋込みpreflightのAST構文確認を行った。別の一時ディレクトリ内のlockに対して独立子プロセスを起動し、保持中は非blocking取得を拒否、解放後は取得できることを確認した。実際の評価用lockには触れていない。activeEvaluation・activeMicrobenchはそれぞれtrueで拒否、両falseで通過し、zombieを残存PIDとして識別する確認も通過した。子プロセスはcommunicate/wait相当のsubprocess.runで回収し、一時ディレクトリを削除した。SSH・socket計測・GPU・solver buildは実行していない。
