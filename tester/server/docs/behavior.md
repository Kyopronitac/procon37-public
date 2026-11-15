# procon-server 挙動メモ

`procon-server` (公式簡易サーバー) を起動して確認した HTTP 挙動。実装時の参照用。

## エンドポイントと応答

| メソッド/パス | 認証 | 成功 | 失敗 |
|---|---|---|---|
| `GET /setting` | token必須 | 200: 問題(公式形式、`Problem` と同形) | 401 `{"code":"AuthError","message":"invalid or missing token"}` |
| `POST /agent` | token必須 | 200: 空body | 400 平文(`SubmitKinds: ...`)／403 `{"code":"AccessTimeError","message":"agent kind submission is closed"}` |
| `GET /` | token必須 | 200: 状態 `{endsAt,day,agents,others,traffics}` | 403 `match has not started`／`match has ended` |
| `POST /` | token必須 | 200: `{"revision":N}` (N≥0) | 200: `{"revision":-1}` (不正計画)／403 `match has not started` |

- 認証: クエリ `?token=` または `Procon-Token` ヘッダ。両対応。
- 不正な計画は **400 ではなく 200 + `{"revision":-1}`** で返る。
- `revision` は全チーム横断の単調増加カウンタ (観測では1から始まる)。採否は公式API仕様どおり、受理は `revision>=0`、却下は `revision<0` で判定する。`revision` の無い応答 (403など) は却下として扱う。

## 状態 (`GET /`) の形式

```json
{ "endsAt": 1790602496, "day": 0,
  "agents": [{"kind":0,"pos":44,"fuel":20}, ...],
  "others": [{"id":1,"agents":[...]}, ...],
  "traffics": [{"pos":1,"status":0}, ...] }
```

- `day` = 完了済みの日数 (1日目中は 0)。`day == daySteps.len()` で終了。
- `others[].id` は他チームのインデックス。
- フィールドは共通クレート `Observation` (camelCase) と一致し、そのままデシリアライズ可能。

## 試合の前後の `/setting`

| 時期 | 応答 |
|---|---|
| 試合開始前(車種選択中を含む) | 200: 問題。ただし `startsAt` は `0` |
| 試合開始後 | 200: 問題。`startsAt` は試合開始(1日目の開始)の時刻 |
| 試合終了後 | 403 `match has ended` |
| トークン誤り | 401 `invalid or missing token` |

- 車種選択の締切は、どのAPIからも取得できない。本選では試合開始と同時に問題が取得可能になり、形式ごとの選択時間(60/90/120秒)で締め切られる(本選マップに関するお知らせ)。server-bridge は問題を最初に取得した時刻とマップの大きさから締切を決める。
- `endsAt` は秒未満を切り捨てた値。実際の締切より最大1秒早い。

## 時間進行

1. 起動 → `kind-deadline`(既定5s) 後に種別締切 (`-kind-deadline` で延長可能)。
2. 種別締切 → `match-start-delay`(既定5s) 後に試合開始。
3. 各日は `daySeconds[d]` 秒で自動進行。回答が無いチームは全車待機扱い。
4. 試合終了後もサーバーは生存し続ける (自動終了しない)。`GET /` は `match has ended` を返す。

## 結果の取得

- **stdout に結果出力なし、stderr は INFO ログのみ、終了後 `GET /` も遮断**。最終スコア・順位はサーバーから取得できない。
- よって対戦サーバー (`tester/server`) が対戦窓口で記録した提出から、共通エンジンで再現してスコア/Replay を生成する。

## その他

- 再提出可。最後に受理された有効回答を採用 (種別・計画とも)。
- `others` は自分以外の全チーム分の `{id, agents}` が返る。
- `startsAt` は config の値に関わらずサーバー実時刻に置き換わる。`endsAt` は各日の回答締切 (UNIX秒)。
