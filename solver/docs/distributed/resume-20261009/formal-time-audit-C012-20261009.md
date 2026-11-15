# C012の正式時間への切替経路：静的監査

**新しいstageで`manage.py run`の`--short-ms`を省略すれば、4チーム共通の公式時間を使用する設定となる。保存済みC012の起動経路に、10秒固定やテスト用の探索予算上書きは見つからなかった。** これはソース・配置保存記録の静的監査であり、正式runの実測や成功を示すものではない。

監査UTC: 2026-10-09T03:13:27.695953+00:00。対象はswitch `20261009T120816-C012-isolated-cooperation-on`。C012進行中結果は読まず、CPU3で軽い読取とSHA照合だけを行った。遠隔操作、ビルド、対戦開始、プロセス変更はない。[SHA・launcher本文・確認条件のJSON](formal-time-audit-C012-20261009.json)に証拠を保存した。

## 対象の一致

- C012の34ソース集約SHA: `10acfa3bde81f1dddf1d80ed4ff2fbe0288395fc4eb6dd0923a410607ed8a541`。各ファイルとmanifestが一致した。
- C012実行ファイルSHA: `574dd99a8ff0d20035acccca3f4e207ad2d7701308bc10374694137aa0f33016`。ローカルファイル、validation、switch記録、3台のswitch後保存ハッシュが一致した。
- switchのplanned-config内のローカル登録ファイルを再ハッシュし、固定server・bridge・単一2体・launcherを含め一致した。4台の保存remoteハッシュもplanned-configに一致した。
- VOIDのlauncher保存本文はinstall記録とswitch後ハッシュに一致。NOMLABは過去に実機から保存した本文を再ハッシュし、**今回C012のswitch後ハッシュと一致**することを改めて確認した。今回は実機を読み直していない。

## 時間が渡る経路

1. [manage.py](../../../distributed-lab/manage.py)は省略された`--short-ms`を`None`として扱う。既存stageを再利用せず、新規run-configへ`timeLimitMs: null`を書く。`--time-limit-ms`をserverへ渡すのは明示された場合だけであり、古い10秒設定を流用しない。`--output-dir`も付けない。
2. [serverのrunner](../../../../tester/server/src/runner.rs)は時間指定があるときだけ問題の`daySeconds`を書き換える。正式時は公式サンプル・練習場fixtureの日程を維持し、手続き生成の場合は既定の60秒/日を用いる。車種選択は形式に応じた60・90・120秒を公式serverの`-kind-deadline`へ渡す。同じ問題と日程が全4チームへ適用される。
3. [team_runner.py](../../../distributed-lab/team_runner.py)はPATH/LANGだけの環境でbridgeを起動する。4チーム分の保存runnerハッシュはこの読取ソースと一致し、`--select-ms`やソルバへの時間上書き引数を渡さない。manageも親環境の`PROCON37_*`、`SELECT_MS`、`TIME_LIMIT_MS`を除去する。
4. [server-bridge](../../../../tester/server-bridge/src/main.rs)は公式選択締切と各日の`endsAt`から残り時間を求め、通常の通信余裕500msを控除して各ソルバへ送る。公式日程は同じでも、受信時刻の差でソルバに渡る「残りms」が完全一致するとは限らない。
5. C012の[runtime](../../../distributed-candidates/c012-isolated-cooperation/runtime.rs)はリクエスト時間から通常の回収余裕を引き、peerには残り時間から150msを引いた予算を送る。[pool](../../../distributed-candidates/c012-isolated-cooperation/pool.rs)は受け取ったdeadlineから実際の残り時間を計算する。10秒固定の上限はない。worker側の600000msは異常に長い入力を拒否する上限であり、10秒の短縮指定ではない。

## C012本来の探索予算と短縮指定の違い

C012の[planner](../../../distributed-candidates/c012-isolated-cooperation/solver/planner.rs)はD0の
`min(limit - 110ms, limit × 91%)`をそのまま持つ。当日探索67%と残りのrollout、選択時62%と残りのforecastもD0と同じ式である。これらをD0ソースと直接照合した。

C012固有の250ms確認、停滞1秒、cooldown1秒、残り750ms、restart余裕100ms、外部枝の25ms修復枠は、協調探索の動作条件である。25msは外来枝のbeamを制限する協調deadlineで、探索全体を25msへ切り詰める設定ではない。C006の97%係数やGPU用の計測器もC012には入っていない。

**正式評価で外すのはテスト用の`--short-ms`だけである。91%係数等を外すと凍結C012とは別実装になり、評価済み版とは扱えなくなる。**

## 固定配置と環境

VOID3台は`env -i`で、PATH、future weight 0、reserve weight 15、通信tokenのファイルパスだけを与える。スレッドは32・16・32、CPU集合は0-31・0-15・0-31、`--cross-elite on`。固定単一2体も`env -i`を使い、CPU0・2で起動する。NOMLABのlauncherは環境を消さないが、その手前の固定team runnerが最小環境にし、launcherが追加するのは固定のfuture/reserve weightだけである。

NOMLABの実配置は`--threads 16`、CPU0-15。[再接続inventory](reconnected-inventory.json)ではRyzen 9 5900HSの8物理コア・16論理CPUであり、最初の指定スペック32論理CPUとは異なる実機を16論理CPU全使用で固定している。この監査では設定を変更していない。根拠は既存の[復元記録](local-restoration.json)と[CPU環境記録](cpu-environment.json)にも残る。

## 正式runで別途確認すること

実行する場合の形は次のとおり。これは手順例であり、この監査では実行していない。

```sh
taskset -c 1,3,5,7 python3 solver/distributed-lab/manage.py run C012-formal-01 \
  --label '分散C012・隔離協調on・正式12件'
```

新規stageを使い、生成commandに`--time-limit-ms`/`--output-dir`がなく、run-configの時間がnullであることを確認する。12件seed0〜11・形式各4件・練習場有効・固定4チームのまま、problem.jsonの日程、公式選択時間、全4bridgeの時間記録、3台参加、全48 AC、前後SHA一致を実際のrunで確認する。

固定相手はブラックボックスとして、起動ラッパー・公開通信・識別用ハッシュだけを確認した。内部実装の閲覧や逆解析はしていない。本監査後の変更や、未実行の正式runの成績・締切遵守までは保証しない。
