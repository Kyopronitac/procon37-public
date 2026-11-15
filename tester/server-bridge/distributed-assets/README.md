# 採用済みD0の評価時実行ファイル

評価時に使った二つの実行ファイルを識別・照合用に保存する。本番配置ではVOID1で `compile.sh` を実行し、生成した実行ファイルを `deploy.sh` で配布する。このアーカイブは配置スクリプトの配布元には使わない。[本番操作](../../server/docs/solver-pc.md#分散ソルバd0を3台で本番に使う)を参照する。

`D0-binaries.tar.gz` は `distributed-solver` と `server-bridge` のみを含む。実装commitは `5d6c740962a1f3e20e77e13df243078df1d29ad2`、採用記録commitは `bf0807116eb882bf8cafc54ac01a535dcc26b6b4`。初回正式run `1791414715585964511-304414` の実行ファイルとSHA-256を照合してコピーした。再ビルドしたファイルではない。

| ファイル | SHA-256 |
| --- | --- |
| `D0-binaries.tar.gz`（2,045,178 byte） | `6556afdbc07aedde2c31e0598de19f078bbce178395108c0c6c2d05b42217d7a` |
| `distributed-solver` | `e80474cc5f65a30b18d8143eec656e3abe093c927d75080aed2bd9dce72a5de0` |
| `server-bridge` | `24cb1008dbb537f6941f08d2e9c31042ff5587a9215748dd9b8e8b15f933c15e` |

[manifest.json](manifest.json) に版・評価run・commit・アーカイブと実行ファイルのSHAを保存する。配置スクリプトは `compile.sh` が配布設定へ登録した各バイナリのSHAを照合して配布パッケージを作る。

保存した実行ファイルはx86-64 Linux/glibc用。solverはglibc 2.34以上、VOID1で使うbridgeは2.39以上を必要とする。配置用のコンパイルは [solver-pc.mdの手順](../../server/docs/solver-pc.md#配置前にコンパイルする)に従う。生成した配布元パスとSHAは自動登録される。
