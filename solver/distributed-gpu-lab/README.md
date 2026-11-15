# 独立GPU producer試作

C007/C008の共有Graph層へ接続できる、整数最短距離表の新規試作。現時点では**未統合・未採用**。既存r3のRust/OpenCLソースは使用せず、D0/C007の公開中の自作ソースと設計から実装した。

この独立Rust workspaceは外部crate不要。`libOpenCL.so.1`を読むのは明示的に起動したproducer threadだけで、crateのimport、CPU参照、通常の単体testではGPUを初期化しない。

```sh
# リポジトリrootで実行する例
. .server/toolchain/env.sh
taskset -c 3 cargo test --manifest-path solver/distributed-gpu-lab/Cargo.toml -j1 -- --test-threads=1
```

APIの入口は`Prepared`、`Tables::verify`、`VerifiedTables::scalars`、`producer::Producer`。7modeの距離と6modeのnextを作り、distance/nextの全要素をCPUで検証してから返す。time/fuel/length/fingerprintはCPUが一度の木DPで計算する。

| 内容 | 現在の状態 |
| --- | --- |
| CPU heapと同期緩和の整数同値比較 | 単体testで確認 |
| departure費用・同距離の最大cell選択・燃料専用距離 | 単体testで確認 |
| caller deadline、GPU待ちの無期限join回避 | GPUを使わないmockで確認 |
| 常駐OpenCL context/program、再利用device buffer、動的FFI | VOIDLINUX-1で合成入力6要求・正常終了を確認 |
| OpenCL kernelのdriver上コンパイル・実GPU同値 | VOIDLINUX-1のNVIDIAで合成3サイズ各2要求、全表一致 |
| 実戦入力によるGPU速度・CPU併用効果 | 未実施 |
| producer TIDを専用cpusetへ移すlauncher | 呼出し側が用意する必要あり |
| C008への接続、対戦成績 | 未実施 |

設計・C008への接続例・制限は [docs/design.md](docs/design.md)、CPU側検証記録は [docs/validation.json](docs/validation.json)。CPU 3限定で10 testsとrelease CLIのCPU-only検査が通過し、CLIの合成16/24/32格子で80,880経路のscalarを照合した。

実機確認用の [docs/smoke-procedure.md](docs/smoke-procedure.md) は、明示的な`--allow-gpu`、既に設定済みのsingleton cpuset、有限timeoutと外側の子孫回収を必須にする。CLIは各サイズ8 targetの同じ表を2回ずつ計算し、cold初期化とwarm再利用を区別する。[初回実機結果](docs/v1-smoke-20261009-r1.md) は2026-10-09の対戦間隔で実施し、全表一致・CPU制約・正常終了・残存なしを確認した。以後のGPU測定も対戦の区切りで親担当の許可後に行う。

C008評価中にまとめた [最小統合案](docs/c008-integration-design.md) は、owner構築だけのGPU利用、CPU fallback予算、host別寿命、producer TIDだけのcpuset参加を具体化した設計書。C009の実装や追加測定はまだ行っていない。
