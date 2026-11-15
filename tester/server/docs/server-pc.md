# サーバーPCのセットアップ

[対戦サーバー](../README.md)（`server`）を動かすPC（サーバーPC）の準備。サーバーPCは1台だけ用意する。
各チームのソルバPCの準備は [ソルバPCのセットアップ](solver-pc.md) を参照する。

```
[サーバーPC] 1台                                   [ソルバPC] チームごとに1台
 server serve / run
   ├ 公式簡易サーバー (procon-server)
   └ 対戦窓口 :11451  ◀──────── HTTP (LAN) ────────  run.sh / server-bridge
```

サーバーPCはソルバPCからの接続を受け付ける側なので、TCP 11451番ポートをLANに開ける必要がある。

## 1. 必要なもの

| 項目 | 内容 |
| --- | --- |
| OS | Linux / WSL（公式簡易サーバーがLinux版のため） |
| Rust | 1.88以上 |
| 公式簡易サーバー | 公式配布のLinux版 `procon-server` |
| ネットワーク | ソルバPCと同じLANにつなぐ |

## 2. 公式簡易サーバーを置く

公式配布の簡易サーバー（Linux版）を `tester/server/procon-server` に置き、実行権限を付ける。
配布物はgit管理対象外で、このリポジトリでは内容を変更しない。

```sh
chmod +x tester/server/procon-server
```

## 3. ポートを開ける

ファイアウォールで、LANからのTCP 11451番ポートへの接続を許可する。

```sh
# Ubuntu（ufw使用時）
sudo ufw allow 11451/tcp
```

別のポートを使う場合は、`server` に `--match-port` を指定し、そのポートを開ける。

### WSL2で動かす場合

既定のNATモードでは、表示されるURLがWSL内部のアドレス（`172.x.x.x`）になり、他のPCから接続できない。
次のどちらかで対処する。

- **ミラーモードにする（Windows 11 22H2以降）**：`%UserProfile%\.wslconfig` に次を書き、`wsl --shutdown` で再起動する。WindowsのIPアドレスで接続できるようになる。

  ```ini
  [wsl2]
  networkingMode=mirrored
  ```

  Windowsのファイアウォールでも11451番ポートの受信を許可する（下記の `New-NetFirewallRule`）。

- **ポート転送を設定する**：Windowsの管理者PowerShellで次を実行し、ソルバPCには `http://<WindowsのIPアドレス>:11451` を伝える。

  ```powershell
  netsh interface portproxy add v4tov4 listenport=11451 connectaddress=<WSLのIPアドレス> connectport=11451
  New-NetFirewallRule -DisplayName "procon37 server" -Direction Inbound -Protocol TCP -LocalPort 11451 -Action Allow
  ```

  WSLのIPアドレスは再起動のたびに変わることがあるので、そのときは設定し直す。

## 4. 起動してソルバPCへ伝える

```sh
cargo run --release -p server -- serve
```

GUIのサイドバーに対戦窓口のURLが表示される。CUIの `server run` では起動時に表示される。
各ソルバPCの担当者へ、次を伝える。

| 伝えるもの | 内容 |
| --- | --- |
| チーム番号 | 0, 1, 2, …。ソルバPCの `run.sh` の `TEAM` に指定する。チームNは `token-pN` で参加する |
| 対戦窓口のURL | `run.sh` の既定値 `http://192.168.0.81:11451` と異なる場合だけ。ソルバPCの `run.sh` の `SERVER` に指定してもらう |

ソルバPCから `curl '<URL>/setting?token=token-p0'` で `match has ended`（試合が無いとき）か問題のJSON（試合中）が返れば、接続できている。

## 5. 時刻を合わせる

ソルバPCは、公式簡易サーバーが返す締切時刻（UNIX時刻）と自分の時計から残り時間を計算する。
サーバーPCの時計がずれていると、全チームの回答時間がずれる。OSの自動時刻合わせを有効にしておく（Linuxでは `timedatectl` で `System clock synchronized: yes`）。

操作・設定の詳細は [CUI・GUIの操作と設定](usage.md) を参照する。
