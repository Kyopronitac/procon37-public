# C009内のGPU producer

`solver/distributed-gpu-lab`の評価済みproducer/lib/kernel/testを候補内へ固定コピーしたもの。OpenCL runtimeとkernelはコピー時点のまま。C009の局所GPU予算にCPU tree DPも含めるため、`src/lib.rs`へ`scalars_until`だけを追加した。

この候補の実GPU検証は未実施。独立gpu-labのVOIDLINUX-1合成smokeをC009/v2の評価として扱わない。親READMEとsource-manifest.jsonを参照する。通常のCPU testやcrate importではdriverをロードしない。
