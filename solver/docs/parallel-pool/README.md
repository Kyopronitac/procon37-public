# 並列ソルバの世代育成

2026年10月7日開始。ユーザーの停止指示まで、指定スキルに従い育成を継続する。初期構築は世代数に含めず、正式12件で単独首位を満たした候補のcommit・通常push成功を1世代と数える。

## 固定配置

| ID | team | SSH host | CPU | 内部スレッド上限 |
| --- | ---: | --- | --- | ---: |
| P0 | 0 | 192.168.0.11 | 0-3,8-11,16-19,24-26 | 15 |
| P1 | 1 | 192.168.0.11 | 4-7,12-15,20-23,28-30 | 15 |
| P2 | 2 | 192.168.0.33 | 0-3,8-11,16-19,24-26 | 15 |
| P3 | 3 | 192.168.0.33 | 4-7,12-15,20-23,28-30 | 15 |
| S0 | 4 | 192.168.0.22 | 0 | 1 |
| S1 | 5 | 192.168.0.22 | 2 | 1 |

実機のCPU/CORE/L3対応を確認済み。同一PCの2体は物理コアを共有しない。サーバーPCは192.168.0.81、対戦窓口はTCP11451。3台すべてから試合なし時のHTTP403を取得し、LAN到達を確認済み。各SSH先の実行領域は `/home/voidlinux-N/procon37-parallel-pool/teams/<ID>`。bridgeは同じ領域の `bin/server-bridge` を使用する。

VOIDLINUX-2・3はchronyで同期済み。VOIDLINUX-1の外部NTP未同期を確認したため、ユーザーの許可に基づきVOIDLINUX-3にNTPクライアント192.168.0.11を許可し、VOIDLINUX-1に192.168.0.33を追加した。chronyc trackingでstratum4・同期成立を確認した。再起動時には同期設定を再確認する。

## 識別・再現

- [共通基準](baseline-manifest.json)：開始時commitとソース一式のSHA256。保存先は `solver/parallel-pool/baseline`。育成中は更新しない。基準の再ビルドは一時コピーで行い、Cargoによる独立workspaceのlock正規化を保存基準へ反映しない。
- [固定相手](fixed-opponents.json)：第6・第8世代、保護実装を閲覧せずビルドしたバイナリ・ソース識別用ハッシュと再現コマンド。
- [公式サーバー](official-server.json)：公式配布URL、取得ZIPとLinuxバイナリのSHA256。バイナリはgit管理対象外の `tester/server/procon-server` に配置。

候補は各専用ディレクトリで `PATH=/home/jikky/.cargo/bin:$PATH cargo build --release -p solver` により再現する。独立コピーした共通基準と専用targetを使用する。固定相手の保存版、対戦samplesの内部実装はブラックボックスとして扱う。

## 評価と再開

正式評価は `cargo run --release -p server -- run 12 --players 6 --label '並列pool・対象と段階・正式12件'`。seed0〜11、形式自動、既定時間、練習場既定有効、`.server/runs/` に保存する。`solver/parallel-pool/tools/summarize.py` は12件・全6体ACを検証し、保存された各チームの順位合計で集計する。同率首位は昇格しない。

再開状態は `.server/parallel-pool-state.json`。進行中の評価は候補と相手を固定し、プロセス・ログ・保存結果を照合して再開する。未完了・非AC・push失敗は完成世代に数えない。

正式評価の前後に `python3 solver/parallel-pool/tools/verify-inputs.py solver/docs/parallel-pool/initial-roster.json --remote-snapshot <新しく取得したSSHハッシュ記録>` で共通基準・6体の保存ソース・実行バイナリ・ランチャー・実行基盤を照合する。候補世代ではその評価用rosterを指定する。保存済みの開始前記録を終了後の実状態として代用せず、終了後にSSHでハッシュを新しく取得する。正式12件・全員AC・順位条件は `summarize.py` と併せて確認する。

## 初期評価結果

[初期6体の正式評価](initial-evaluation.md)は全72判定AC。最下位P0を対象に、第1世代の再構築を開始した。

## 第1世代の昇格判定

[P0 a10の正式評価](generation-001-evaluation.md)は全72判定AC、順位合計27で単独首位。ソースと配置を前後照合した評価済み実装を昇格対象とする。push成功は再開状態とpromotion記録で確認する。次の対象はP1。開発者が自分の候補の配置と6体短縮対戦を担当し、短縮全体1位の確認後に親が正式評価する。

固定共通基準・候補の識別一致を保つため、コピー元の公式READMEのCRLFと初期開発者の検証ログの末尾空行はそのまま保存する。新規管理記録・ツール・スキルの差分検証は成功。
