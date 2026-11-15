# C013: 探索継続中の提出候補配信

C012 の分散探索と日中の解共有を引き継ぎ、VOIDLINUX-1 が探索途中の有効解を JSON Lines で返す候補版です。C012 の保存済み実装・評価は変更していません。採用・正式評価は未実施です。

各探索スレッドは現在日の最良解を最大 1 秒に一度、専用の 1 要素スロットへ非待機で公開します。探索ループを再起動しません。内部で翌日を試す rollout の解を現在日の提出に混ぜません。スナップショットは現在の解を実行する前の盤面と、その解に対応する `last`・探索キャッシュ・過去の収集履歴を含みます。集約側でも元の Task に対して検証します。

中間解には現在日の探索目的値を使い、探索完了時の候補は別に管理して C012 の rollout による最終選択方法を維持します。途中の目的値と最終目的値を混ぜて最終解を選びません。すべての探索が期限内に完了しない場合だけ、検証済みの中間解を最終応答へ利用します。

## 起動と通信

coordinator に値を取らない `--stream-responses` を追加します。全 3 台は同じ VERSION=4 のバイナリを使います。`--cross-elite on` と既存の 32・16・32 論理スレッド構成は維持します。追加の通信スレッドはありません。

```text
{"requestId":42,"answer":[...],"final":false}
{"requestId":42,"answer":[...],"final":true}
```

coordinator は直ちに安全な初期解を配信し、その後は約 1 秒間隔で変更された中間解を配信します。すべて stdout を flush します。外部への 5・15・25・35・45・55 秒の HTTP 提出は server-bridge のスケジューラが担当します。`--stream-responses` を省くと従来の最終応答 1 回だけになります。

リモート worker は `Progress {session, request, digest, candidate}` を送ります。Progress はその探索を完了扱いにせず、`pending` を維持します。各 worker/control 間は 1 要素に集約し、通信ソケットに書きかけがある場合は新しい中間解をキューへ追加しません。最終 Result は未送信の中間フレームを置き換えます。session/request/digest/worker 範囲を検査し、古い中間応答を破棄します。

次の plan リクエストに、実際に受理された直前の answer を `previousAnswer` として付加できます。最終の更新提出が失敗して前の中間提出が有効な場合も、その answer と一致する保存スナップショットへ戻してから処理します。不明な answer はエラーにし、最終提出が成功したと誤認して続けません。各フェーズの保存数は最大 128 個で、中間配信を 127 個までに制限して最終応答の枠を確保します。

## 検証

`.server/toolchain/env.sh` の環境で CPU 1・3・5・7、Cargo jobs=4、test threads=1 を使用します。

```sh
source .server/toolchain/env.sh
CARGO_BUILD_JOBS=4 taskset -c 1,3,5,7 cargo test --release --offline --manifest-path solver/distributed-candidates/c013-scheduled-submission/Cargo.toml -- --test-threads=1
```

最終実施結果は `validation.json`、完全ログは `docs/validation.log`、ビルド入力の識別は `source-manifest.json` を参照してください。新規テストでは探索中の公開、現在日の実行前状態、rollout と古い要求の遮断、リモート Progress のコンテキストと pending、最終フレーム優先、先に受理された answer の次日復元を検証します。
