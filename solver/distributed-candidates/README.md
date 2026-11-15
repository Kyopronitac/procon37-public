# 分散ソルバの未採用候補

採用済みD0は `../distributed` に保存しています。ここでは採用版を変更せず、独立workspaceで候補を構築します。正式4チーム12件で採用が決まるまでは採用済みとして扱いません。

- [C006：探索時間上限97%](c006-budget/README.md)：短縮12件を2回実施。順位改善は再現せず、未採用。
- [C007：経路木metadataの一括計算と経路列の遅延展開](c007-compact/README.md)：短縮12件で対戦成績の改善を確認できず、未採用。
- [C008：機内の経路表共有](c008-shared/README.md)：C007から派生。構築量を削減したが短縮12件の順位・日別種類数は悪化し、未採用。
- [C009：共有ownerのGPU経路表構築](c009-gpu-owner/README.md)：CPU側の統合検証済み。別probeの実機試験はGPU初期化前に失敗し、速度効果は未判定。
- [C009b：GPU起動gateの修正](c009b-gpu-owner/README.md)：C009を保存した独立修正版。読取レビュー済み、未ビルドで保留。
- [C010：PC間候補共有](c010-cross-elite/README.md)：短縮12件を2回実施。玉数は増えたが順位・日別種類数は悪化し、正式評価を見送り。
- [C011：翌日への終端位置評価](c011-terminal-reachability/README.md)：短縮12件で順位・得点の改善を確認できず、正式評価を見送り。

候補内のmanifest・検証記録は評価対象の識別情報として保持する。最新の対戦状況は以下の再開記録を参照する。各候補の`target/`は生成物としてGit管理から除外する。

[再開記録](../docs/distributed/resume-20261009/README.md)と[評価ハーネス](../distributed-lab/README.md)を参照してください。
