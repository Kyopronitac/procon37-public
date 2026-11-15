# 入出力と接続境界

この設計は実機検査前に作成したもの。初回のVOIDLINUX-1実機検査は後に完了し、[検査結果](v1-smoke-20261009-r1.md)へ保存した。以下の「未検証」は設計凍結時点を示し、C008未統合・未採用の状態は継続している。

このライブラリは共有cacheや探索を持たない。C008の`GraphTables::build(&PreparedGraph)`で必要な距離/nextだけを供給する独立producerであり、GPUの効果が不十分なら取り込まずに済む。固定ソルバ、既存の探索配分、TCP protocolは変更していない。

## Preparedと出力

入力`Prepared`は次のowned配列を持つ。呼出し側は完全に検査済みの`Arc<Prepared>`を1要求として渡す。

| 配列 | 形と意味 |
| --- | --- |
| `neighbors` | cellごとの6方向`u16`。欠番は`u16::MAX`。双方向の隣接を検査する |
| `passable` | cellごとに0/1。blocked cellも配列からは除かない |
| `durations` / `costs` | C007の最終交通反映後の出発セル費用。1〜4 / 1〜2 |
| `shops` | そのcellが店なら1。brandやstockは経路重みへ加えない |
| `targets` | C008の`positions[..targets]`を順序・重複保持で渡す |

cell数上限は1024、target列は重複を含め1088。距離配列のindexは`(mode * target_count + target_index) * cells + cell`。mode0〜5はC007の6経路重み、mode6は燃料のみ。`next`はmode0〜5だけ同じlayoutを使い、target・blocked・unreachableの未使用値は0。距離のunreachable値は`i32::MAX`。型と上限の検査後にだけOpenCLへ配列を送る。

```rust,ignore
// C008側の境界例。C008のコードはこの試作では変更していない。
let input = Arc::new(Prepared {
    neighbors: /* map.neighbor(cell, dir)、欠番NO_CELL */,
    passable: /* terrain != 3 */,
    durations: prepared.durations.clone(),
    costs: prepared.costs.clone(),
    shops: prepared.spot_at.iter().map(|&i| u8::from(i >= 0)).collect(),
    targets: prepared.positions[..prepared.targets].iter().map(|&c| c as u16).collect(),
});
match gpu.compute(input, absolute_deadline) {
    Ok(computed) => {
        let scalar = computed.tables.scalars()?;
        // 各(mode,target)をRouteTreeへ変換し、Pathのsourceは元positionsを保つ。
        // scalar.times/fuels/lengths/fingerprintsも同じflattened index。
        // fuel_to[target]はraw.distancesのmode6から読む。
        // CPU検証/木DP/Graph補助表作成の時間もGraph構築総時間へ含める。
    }
    Err(reason) => {
        // 時間が残るときだけ既存のC008 CPU builderへ。
        // Starting/Busy/期限切れ/driver失敗をログに残し、GPU成功に数えない。
    }
}
```

本ライブラリにはCPU fallbackの自動選択を入れていない。C008側が残り時間・共有cache・有効な既存候補を見て判断する。`cpu_reference`は有限テストの参照としても使えるが、C008比較のCPU builderを無断で置き換えるものではない。

## kernelと検証

`shortest.cl`は1workgroupを1(mode,target)へ割り当て、local memoryの距離2面で同期緩和する。mode6を含め全て整数。cell数roundまでに無変更を確認できなければ失敗を返す。kernelの実GPUコンパイル・実行はまだ確認していない。

距離が確定してから、`d[next] + weight[source] == d[source]`を満たす最大cell番号をnextにする。これはC007の`BinaryHeap<(Reverse(distance),cell)>`とstrict更新が選ぶ順番である。**edgeが出発cellのみに依存し、全edgeが正、同距離をcell降順に処理し、同値更新しない場合だけ成立**する。

CPU検証は全mode/target/cellのBellman式、nextの同値tie、shape、INF、未使用next値を確認する。正の重みによる距離の減少でtree DPを行い、time/fuelは実際の出発cellを積算する。fingerprintはC007の64bit wrapping演算と同じ。mode重複maskの最終的な全経路照合はC008側で維持する。

## 常駐threadと寿命

`Producer::spawn(config, gate)`はproducer threadを作るが、OpenCLに入る前に`gate(tid)`を呼ぶ。gateで外側controllerへTIDを通知して移動完了を待つ。その後、ライブラリが`/sys/fs/cgroup/cpuset/<group>`のv1 mount・thread membership・singleton CPU・現在のallowed CPU・tasks内TIDを確認してからloaderを読む。現在のVOID機の構成に限定した確認であり、別cgroup配置を推測して許可しない。

初期化中は`Starting`、準備後は`Ready`。呼出し側はStartingならCPUを使い、初期化完了を提出経路で待たない。全OpenCL操作、context/program/bufferの所有と破棄はproducer内だけで行う。GPU deviceだけを列挙し、CPU OpenCL deviceへの暗黙fallbackはしない。

同時GPU要求は1つのみ。`compute`はbusy時に即座に失敗を返し、成功した要求も絶対deadlineまでしか待たない。期限切れなら以後のGPU要求を停止する。遅れて完了した結果のreceiverは破棄されるため、提出候補へ勝手に反映されない。各要求は事前/事後のproducer confinementを確認する。driver thread全体の監視・回収は外側controllerが追加する必要がある。

driver内部の呼出しは強制中断できるとは仮定していない。`shutdown(deadline)`はcallerの待ちを制限し、期限切れでもdriver threadをjoinしない。`Drop`もjoinしない。GPUで停止したprocessの最終回収は既存の外側監視に任せる。正常shutdownも実GPUでは未検証。`Status.last_error`はproducer終了後も保持するので、`Stopped`だけを成功根拠にしない。

## 次の確認

CPU単体testの通過はGPU kernelやFFIの実行成功を意味しない。rootが許可した評価間隔で、まずsingleton cpuset内の1producerで初期化・1要求・全表同値・正常終了・残存なしを確認する。次に複数要求によるcontext/buffer再利用、timeout時の外側回収を測る。

その後で同一入力のC008 CPU builderと比較する。初期化・upload・dispatch/read・CPU検証・木DP・Graph補助表・終了を含む合計を計上する。CPU共有で構築量が小さくなっている場合はGPU統合を見送る。実験バイナリのCPU testsしか済んでいない状態で、採用版や固定相手へ配置しない。
