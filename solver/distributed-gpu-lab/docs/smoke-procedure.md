# 実機検査手順

以下は初回実行前に確定した手順。[初回結果](v1-smoke-20261009-r1.md)は別途保存済み。再実行は新しい対戦間隔の許可・専用label・controllerの識別更新後に行い、既存出力は上書きしない。

この手順は新規ライブラリのFFI、kernel、常駐context、終了処理を小さな有界入力で確認するためのもの。C008への接続、4チーム対戦、実戦性能評価ではない。**親担当が現在の対戦終了を確認し、次の評価間隔で許可してから**1台ずつ行う。初回はVOIDLINUX-1のNVIDIA、CPU 31を使用する。VOIDLINUX-2ならCPU 15、VOIDLINUX-3ならCPU 31。既存ソルバの配置や設定は変更しない。

## 先に確定するもの

- この独立ディレクトリのsource SHA-256 manifestとrelease binary SHA-256を保存する。実際にGPUを動かすbinaryと対応させる。
- 新しい実験専用名の出力・配置・cpusetを使う。旧r3の記録や実行ファイルを上書きしない。
- 対象GPU、driver、runtimeの識別を既存inventoryと対応させる。初回NVIDIAではRusticl用環境を混ぜない。
- 外側controllerが一時的v1 cpuset `/sys/fs/cgroup/cpuset/<今回名>` を作り、`cpuset.mems`を親から継承し、`cpuset.cpus`を31に限定してから子のlauncher/monitorを移す。CLI、producer、driver worker、compiler子プロセス全体が起動時からそのgroupを継承するようにする。`taskset`だけでは不可。

既存のr3 `run_cpuset_profile.py` は旧binary hash/pathを前提にするため、そのまま新binaryの実行器には使わない。親担当の既存controllerのcpuset作成・子移動・回収手順を使用する。新CLIはgroupのmount、canonical path、membership、singleton CPU、effective CPU、allowed CPU、tasks内のTIDを検証する。不一致ならOpenCL loaderを読む前に失敗する。

## 有界CLI

```sh
# 開発機でCPU 3を使用するbuild。対戦の最終反復/正式評価中には行わない。
. .server/toolchain/env.sh
taskset -c 3 cargo build --release --manifest-path solver/distributed-gpu-lab/Cargo.toml -j1 --bin gpu-smoke

# GPUを起動しないAPI確認。実機検査の成功を代用するものではない。
taskset -c 3 solver/distributed-gpu-lab/target/release/gpu-smoke --cpu-only
```

外側controllerが設定済みのcpuset内で、次の構造のコマンドを実行する。`LAB`、`RUN`、`GROUP`には今回の配置先・未使用出力名・実際に作成したgroupを指定する。`MONITOR`には回収済みr3の`monitor.py`を使える。monitorはbinaryをhashし、10 msごとに子孫process/threadを観測し、終了時に採用した子孫を回収する。旧monitorの`source_sha256`は旧labの同居ファイルを指すため、新sourceの識別には別途保存した今回manifestを使う。

```sh
# この例の実行前に外側controllerによるcpuset設定と移動が必要。
python3 "$MONITOR" --output "$RUN" --timeout 45 --expected-cpu 31 -- \
  "$LAB/gpu-smoke" --allow-gpu --device NVIDIA --cpu 31 --cpuset "$GROUP"
```

CLIは決定的な16×16、24×24、32×32の六角格子fixtureを各1件生成する。サイズを3形式に合わせた合成入力であり、実際のserver生成3形式を読み込むものではない。各8 target（順序付き重複を含む）、7mode距離、6mode nextを、独立heap参照の全要素と照合する。time/fuel/length/fingerprintも全到達経路の個別walkで検査する。

1つのcontextで各fixtureを2回、計6要求実行する。`gpu_ready.startup_wait_ms`と各要求の`context_initialization_ms`に初期化とprogram buildを記録する。最初の要求は`first_after_init=true`、以後は常駐context再利用。`repeat=1`は同一入力・同一allocation後のwarm反復。ここでcoldとは**新process/context**であり、driverのdisk cache消去ではない。初回と再実行の差を隠さず、cache条件も保存する。

期限はstartup 8秒、各要求5秒、GPU phase 25秒、shutdown待ち1秒。driver呼出しそのものをthread内で強制中断できる保証はなく、外側monitorは45秒で専用process groupを止める（TERM待ち最大5秒、子孫回収最大5秒を別途含む）。外側controller自身にも有限期限を設け、cpuset tasksを確認し、自分が起動したprocessだけを回収する。

## 成功条件と保存

1. CLI exit 0、6要求全て`exact_table_match=true`。3種類各2要求のchecksumがそれぞれCPU参照と一致。
2. `gpu_shutdown.completed=true`、phase `Stopped`、`last_error=null`。`probe_complete.success=true`。
3. monitor exit 0、`affinity_violations=[]`、`monitor_error=null`、timeoutなし。startupからdriver/compilerの全観測threadがCPU 31のみ。
4. monitorの子孫回収結果を記録。controller終了時に今回group内tasksが空で、今回起動したsolver/probe/driver子孫/monitorが残っていないことを確認してgroupを削除。
5. JSON Lines、stderr、monitor manifest、source/binary hashes、host、時刻、CPU/GPU設定、group作成/削除結果を新規ファイルとして保存。

`execution_and_read_ms`はdispatchからblocking read完了まででありkernel単体時間ではない。CPU参照は正確性の検査器なので、その`heap_ms`をC008共有builderの性能と同一視しない。初期化、allocation、転送、GPU処理/read、全表CPU検証、CPU tree DP、shutdownの時間を区別する。今回は合成3入力だけのAPI/driver検証に留め、C008へ統合・採用しない。
