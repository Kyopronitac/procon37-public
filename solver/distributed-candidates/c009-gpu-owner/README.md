# C009：共有Graph ownerだけの常駐GPU構築

C008から作成した独立候補。**ローカル検証済み・実GPU未確認・未配置・未採用**。C008/C007/D0、固定比較相手、独立gpu-labの評価済みsourceは変更していない。

評価間隔でCPU 3/-j1のrelease build、solver単体30件、GPU libraryのCPU-only 8件、localhost通信2件が成功した。GPU adapterは実生成3入力1,604,490経路、位置重複/交通/疎target 59,238経路がCPUと完全一致。既存D0 eager 1,608,270経路同値も成功。性能microbench1件はignoreし、GPUは起動していない。全5子PIDの終了を確認した。[検証記録](validation.json)とsource-manifestは`switch.py candidate()`で整合確認済み。

共有cacheのownerだけが1つの常駐OpenCL producerを試す。GPUがStarting/Busy/disabled、局所予算不足、shape不適格、結果不一致なら既存のCPU builderへ戻る。private bypassはCPUのみ。CPU/GPUは同じGraphTables assemblerでmask/potential/mobilityを作り、GPUの距離/nextは全要素検証してから公開する。

初版はVOIDLINUX-2だけ明示opt-inできる。VOIDLINUX-1/3はGPU off、全機のsearch数31/15/31と現状のhost CPU集合を維持する。GPU producer/driverはCPU 15 singleton cpusetへ自身TIDだけを移す。rootが専用groupのtasksだけを実験ユーザーへ委譲する。GPU有効化・権限委譲はまだ実機未確認で、テスト実行許可後に有限preflightを行う。

- [実装と評価前提](docs/implementation.md)
- [source識別と親版](source-manifest.json)
- [host別配置案](docs/deployment-plan.json)
- [C008の保存manifest](docs/c008-source-manifest.json)

GPU待ち・CPU組立は1つの局所deadlineを共有し、CPU fallback見積りを確保する。CPU見積りにはGPU成功時間を混ぜない。CPU構築自体の中断やdriver呼出しの強制停止は保証しない。タイムアウトしたproducerは提出経路からjoinせず、process所有者が最終回収する。

次はv2の正確性・cpuset参加・全thread CPU制約・終了回収と、実サイズowner表の少数off/on比較を評価間隔で確認する。全費用込みの効果が薄ければGPU案を保留し、有望なら固定4チーム短縮12件へ進める。

```sh
# 親担当が評価間の実行を許可してから。現在は実行しない。
. .server/toolchain/env.sh
taskset -c 3 cargo test --offline --locked --manifest-path solver/distributed-candidates/c009-gpu-owner/Cargo.toml \
  -p procon37-distributed --bin distributed-solver -j1 -- --test-threads=1 --nocapture \
  --skip wire::tests::fragmented_frames_and_oversized_frames
taskset -c 3 cargo test --offline --locked --manifest-path solver/distributed-candidates/c009-gpu-owner/Cargo.toml \
  -p procon37-distributed-gpu-lab -j1 -- --test-threads=1 --nocapture
```
