# C012 短縮1・初日 receiver checks の読取診断

保存された早期ログでは、初日（request 2）の `checks=0` は VOIDLINUX-1/3 の2台である。VOIDLINUX-2 は初日から `checks=20, imports=2, improved=1` を記録しており、初日を一律に省略する挙動ではない。

| request | node | checks | eligible | imports | improved |
| --- | --- | ---: | ---: | ---: | ---: |
| 2 | v1 | 0 | 0 | 0 | 0 |
| 3 | v1 | 21 | 2 | 2 | 0 |
| 2 | v2 | 20 | 2 | 2 | 1 |
| 3 | v2 | 20 | 2 | 2 | 2 |
| 2 | v3 | 0 | 0 | 0 | 0 |
| 3 | v3 | 20 | 1 | 1 | 1 |

D0由来の `plan_with_rollout` は、翌日があり制限時間が3.5秒以上なら最初の67%を実日の `plan` へ渡す。既存 cache は初期解の候補になるだけで、初日/選択時 cache による primary 探索の省略分岐はない。実日 primary の終了後に `self.cross.take()` してから forecast を行うため、forecast の探索時間は receiver checks に入らない（`solver/planner.rs:948, 1044, 1467`）。

C012では Graph/初期解の準備後、`Binding::from_solver` を一度だけ呼び、その結果を primary の終了まで保持する（`solver/planner.rs:1079`）。`CrossBus::bind` は共有 mutex を `try_lock` し、競合時は直ちに `None` を返す。ここには再試行・失敗計数がない（`cross_elite.rs:135`）。したがって、一時的な bind 競合によって、その primary 全体で受信確認がなくなる経路はコード上存在する。同じ node の他 donor の公開成功は、その receiver の Binding 成功を保証しない。

ただし、今回 v1/v3 で実際に bind が競合したとは断定できない。保存ログには bind 成否や receiver 固有の primary 進行がなく、Binding 自体の不成立、初期処理/探索速度によって250ms後の1024 iteration地点へ到達しなかった場合などを切り分けられない。全6行の `context_mismatch/rejected` は0だが、未計数の bind 競合の有無は分からない。

`checks` は1024 iteration地点で250ms周期を満たして `Binding::ready` に入ると増え、その後に停滞1秒・切替 cooldown・残750ms・通常restartまで100msの条件を判定する（`solver/planner.rs:1264`, `cross_elite.rs:263`）。これら後段条件だけでは `checks=0` を説明できない。逆に `checks>0, eligible=0` は通常起こり得る。`improved` は輸入後の直接修復で局所bestを更新した回数で、最終提出への採用・対戦得点の改善を表さない。

将来の切分けには、bind試行/成功/競合/対象外と receiver primary のiteration数・初期化時間の有界カウンタが有用。ただし本診断では実装・設定・バイナリを変更せず、実行中評価を維持した。CPU 3固定の保存ファイル読取だけを行い、ビルド・remote操作・追加実験は行っていない。

最小修正案（未実装・C012評価後の判断用）:

- 実日 primary の受信専用workerだけ、初回bind結果を「有効」「一時的なmutex競合」「恒久対象外」に区別する。off/Select/forecast、係数不一致、範囲外、context不一致、閉鎖/期限切れは再試行対象にしない。現在の `Option` だけでは競合と通常disabledを区別できない。
- 初期競合の場合だけ、既存1024 iteration地点から非blocking再bindを行う。250ms以上の間隔、明示的な有限試行数、実日primary/通信bus双方の残750ms以上を条件にし、成功・試行上限・閉鎖・期限到達で終了する案が小さい。試行上限の具体値は未決定であり、無制限retryやロック待ちを追加しない。Binding成功後の現行check/停滞/cooldown条件は維持する。
- 初回生成した完全Contextをpending状態に保持し、retryごとのGraph/Problem走査・cloneを避ける。context全値照合は省略しない。初期Context生成時間、bind試行/成功/競合/恒久対象外、retry成功までの時間と試行数を固定数のカウンタで記録する。これにより準備費とmutex競合を分ける。失敗ごとのログ行は増やさない。
- 74独立workerのRNG・restart、受信担当のlocal Elite除外、最終Result/提出経路を変えない。成功するまでreceiverは通常のD0探索を続ける。検査対象はmutex占有時の初回失敗→解放後の再bind、恒久対象外の再試行ゼロ、closed/deadline/試行上限の終了、完全context不一致拒否、Context生成一回に絞れる。

これは実コード上の取りこぼし経路への対案であり、今回の初日0件の原因が競合だった証明ではない。現評価の全72実日におけるchecks=0の比率と成績を確認してから、次候補に必要か判断する。現行C012は凍結を維持した。

識別情報:

- 入力: `solver/docs/distributed/resume-20261009/participation-start-C012-on-short-01.json`
- 入力SHA-256: `38293d9c289a9400763565a123705075e0c8d0cdbac630f1f26f251890d105f3`
- run ID: `1791515312515018350-131487`
- 早期取得日時: `2026-10-09T12:09:13.957226+09:00`（12件完了結果ではない）
- C012 source34 aggregate: `10acfa3bde81f1dddf1d80ed4ff2fbe0288395fc4eb6dd0923a410607ed8a541`
- 読取対象 `solver/planner.rs` SHA-256: `6ea875ef7a33e1956a583f3c112e403b6f4cd4d4b96ed1130481d0dff762c7c3`
- 読取対象 `cross_elite.rs` SHA-256: `2ee188add1fdc805952e2242ab4b736e7cceddc192e1450986e979a4a6579764`
- 読取対象 `pool.rs` SHA-256: `2923414cb050660191c7b402720261c764062f2af96a51bc614ca9e58af34475`
