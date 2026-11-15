# C009b：GPU producerのcpuset自己TID書込み修正候補

C009から独立コピーした、**未ビルド・未実行・未採用**の候補。元C009と初回失敗probeは変更しない。GPUの探索内容、owner-only範囲、deadline、CPU fallback、配置案はC009と同一。

C009のv2初回probeは起動gateで`EINVAL`となり、GPU速度を測定できなかった。cgroup `tasks`への可変format書込みが複数writeに分かれた可能性があるため、TID＋改行を事前にformatし、`write`を1回だけ行って全byte数の一致を要求する。短いwriteやエラーで再書込みせずCPU fallbackへ戻す。open/writeのエラーには操作の文脈を加える。

変更と次回手順は[docs/change.md](docs/change.md)。保存元の[source manifest](docs/c009-source-manifest.json)が親版を識別する。対戦中はビルド・リモート操作をしない。
