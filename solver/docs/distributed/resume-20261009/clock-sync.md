# 評価用LANの時刻同期

2026-10-09、問題serverが動くPCの時計へ4台を同期した。ローカルsudoは使わず、[clock_sync.py](../../../distributed-lab/clock_sync.py)を`192.168.0.81:32123/UDP`で起動した。使用する時計は問題serverと同じホストの`CLOCK_REALTIME`。外部UTCの正確性を提供する用途ではなく、評価LAN内の締切計算に使う相対同期である。

時刻サーバーは非特権ユーザーで常駐し、現在のPIDは`90752`。起動識別・bind・許可IPは`.server/distributed/clock/server-process.json`、ログは同ディレクトリの`server.log`にある。4台とこのPC以外からの要求には応答しない。標準NTPの48byte応答を返し、ローカル時計をstratum 10として通知する。パケットの送信元timestampをそのままoriginへ返すことと、時刻変換の往復をローカルで確認し、実際のchronydが選択・同期することを4台で確認した。

各機の既存chrony設定は`/var/tmp/procon37-clock-20261008T234629Z/`へ保存し、設定ファイルのSHAが変更前後で一致することを確認した。永続設定は変更していない。既存chronydへ次のruntime sourceを追加した。

```sh
sudo chronyc add server 192.168.0.81 port 32123 iburst minpoll 0 maxpoll 4 prefer trust require
sudo chronyc burst 4/8 192.168.0.81
```

`prefer`でこの時計を優先し、`trust`でこの実験の基準とする。`require`により、このsourceが失われたときに別の時刻源へ切り替えて評価条件を変えることを防ぐ。4台すべてが`Reference ID C0A80051 (192.168.0.81)`を選択したことを確認してから、**評価開始前に一度だけ**`sudo chronyc makestep`を実行した。その後は`chronyc waitsync 20 0.002 0 1`で残る補正が2ms未満であることを確認し、通常の連続補正へ移った。評価中はmakestep・chronyd再起動・時刻サーバー切替を行わない。

これはchrony公式の[runtime source追加とmakestep](https://chrony-project.org/doc/4.8/chronyc.html)、[port・prefer・trust・require](https://chrony-project.org/doc/4.8/chrony.conf.html)の指定に従う。時刻差とRTTの計算は[NTP仕様](https://www.rfc-editor.org/rfc/rfc5905.html)の4 timestamp方式を使う。

## 実測

各機から24往復し、RTTが小さい半分の時刻差中央値を使用した。値は「server時計−各機時計」。約40秒後の追跡でも4台すべてが同じsourceを選択し、Reach 377、Leap Normalを確認した。

| ホスト | 同期前の差 | 初回同期後 | 追跡時の差 | 追跡時の最小RTT |
| --- | ---: | ---: | ---: | ---: |
| VOIDLINUX-1 | −344.759ms | −0.006ms | −0.020ms | 0.262ms |
| VOIDLINUX-2 | −121.621ms | −0.019ms | −0.003ms | 0.174ms |
| VOIDLINUX-3 | +373.401ms | −0.054ms | −0.017ms | 0.199ms |
| NOMLAB | +34.017ms | +0.101ms | −0.108ms | 1.541ms |

測定にはLANの経路非対称性・ソフトウェアtimestampの誤差が含まれる。上記はミリ秒未満まで揃ったことの実測であり、無誤差の保証ではない。詳細な各サンプル、chrony tracking/sources/ntpdata、backupと変更コマンドは[全体記録](clock-sync.json)と`clock-sync-{v1,v2,v3,nomlab}.json`に保存した。

## 再開と停止

ローカルサーバーを再起動する場合はリポジトリルートを実際に確認し、記録済みPIDが停止していることとUDP32123が未使用であることを確認する。その後、通常ユーザーで起動する。

```sh
mkdir -p .server/distributed/clock
nohup python3 solver/distributed-lab/clock_sync.py serve \
  --address 192.168.0.81 --port 32123 \
  --record-dir .server/distributed/clock \
  > .server/distributed/clock/server.log 2>&1 < /dev/null &
```

遠隔chronydを再起動した場合、runtime sourceは失われる。まず`chronyc -n sources`で現状態を確認し、既存設定を新しいbackupへ保存してからsourceを追加する。上記の選択確認・評価前makestep・waitsync・実測を再実施する。測定だけなら、このスクリプトの`probe --address 192.168.0.81 --port 32123 --samples 24`を各機で使える。スクリプト自体にsudoは不要。

停止は評価を終えてから行う。4台それぞれで`sudo chronyc delete 192.168.0.81`を実行すると、追加したruntime sourceだけを取り除ける。既存設定は変更していないので、設定ファイルの書き戻しやchronyd再起動は不要。追加前に不通だった外部sourceが復旧する保証はない。

続いてローカルの`server-process.json`にあるPID・`startTicks`・`bootId`を`/proc`と照合し、そのPIDだけにSIGTERMを送る。別のPIDやchronydを一括停止しない。開発の停止報告には、この時刻サーバーを残しているかとPIDを必ず含める。
