# 世代別の評価・登録記録

[保存版の概要](../README.md)

各世代の対戦設定、実装の識別情報、評価結果、昇格判断、前後比較を保存しています。旧 `solver/docs/pool/` は育成の管理・引き継ぎに用いた記録置き場であり、個別ソルバの開発者だけが読むためのメモではありません。評価の根拠として必要なため、2026年10月07日のユーザー指示により、内容を保持してこのディレクトリへ集約しました。

| 世代 | 説明 | 機械可読記録 | 当時の対戦設定 |
| --- | --- | --- | --- |
| 第1世代 | [評価・昇格](generation-001.md) | [JSON](generation-001.json) | [設定](generation-001-config.json) |
| 第2世代 | [評価・昇格](generation-002.md) | [JSON](generation-002.json) | [設定](generation-002-config.json) |
| 第3世代 | [評価・昇格](generation-003.md) | [JSON](generation-003.json) | [設定](generation-003-config.json) |
| 第4世代 | [評価・昇格](generation-004.md) | [JSON](generation-004.json) | [設定](generation-004-config.json) |
| 第5世代 | [評価・昇格](generation-005.md) | [JSON](generation-005.json) | [設定](generation-005-config.json) |
| 第6世代 | [評価・昇格](generation-006.md) | [JSON](generation-006.json) | [設定](generation-006-config.json) |
| 第7世代 | [評価・昇格](generation-007.md) | [JSON](generation-007.json) | [設定](generation-007-config.json) |
| 第8世代 | [生成方法・登録結果・前後比較](generation-008.md) | [JSON](generation-008.json) | [設定](generation-008-config.json) |

第8世代は、従来のsolver-pool方式で独立に新規設計した世代ではありません。第7世代を引き継いだ継続改善から、既定フルテストで最良記録と同率の現採用v117を選んで登録しました。世代番号と生成方法を区別して記録しています。

第6・7世代の実装保存版は [solver06](../solver06/README.md) と [solver07](../solver07/README.md) にあります。正式昇格時のコミットから追加保存し、元の評価記録を保持しています。

## 補足記録

- [第4世代の中断記録](generation-004-interrupted.json)
- [第7・第6世代の新旧基準フル比較](generation-007-vs-006-full-20261006.json)
- [以前の端末移行・引き継ぎ記録](HANDOFF.md)
- [今回の移管元・移管先・内容照合](migration-20261007.json)
- [第8世代のbefore / after公式比較](generation-008-before-after.json)
- [第6・7世代の実装追加保存と照合](source-registration-006-007-20261007.json)
- [第6〜8世代の保存形式統一と照合](format-unification-006-008-20261007.json)

過去の記録に含まれる絶対パス、旧保存先、対戦条件、実行IDは、当時の証拠として保持しています。現在の実行指示ではありません。設定を再利用する場合は当該環境のパスと条件を確認してください。移管時に新しい性能評価やソルバ変更は行っていません。
