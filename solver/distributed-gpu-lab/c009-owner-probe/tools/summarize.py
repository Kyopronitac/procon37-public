#!/usr/bin/env python3
"""Summarize one completed idle probe; no remote operations or GPU startup."""
import argparse
import json
import math
from pathlib import Path
import statistics


def summarize(directory):
    controller = json.loads((directory / "controller.json").read_text())
    events = [json.loads(line) for line in (directory / "probe.jsonl").read_text().splitlines()]
    pairs = [row for row in events if row.get("event") == "pair"]
    starts = [row for row in events if row.get("event") == "producer_ready"]
    ends = [row for row in events if row.get("event") == "probe_end"]
    valid = (controller.get("success") and len(starts) == len(ends) == 1 and len(pairs) == 6
             and {(row["seed"], row["repetition"]) for row in pairs} == {(s, r) for s in range(3) for r in range(2)}
             and all(row.get("exactGraphMatch") and not row.get("fallback") for row in pairs)
             and ends[0].get("normalShutdown") and ends[0].get("success"))
    if not valid:
        return {"validScreeningRun": False, "controller": controller,
                "reason": "failed/partial probe is not performance evidence", "completedPairs": pairs}
    start, end = starts[0], ends[0]
    formats = []
    for seed in range(3):
        rows = [row for row in pairs if row["seed"] == seed]
        cpu = [row["cpuMs"] for row in rows]
        gpu = [row["gpuIncludingFallbackMs"] for row in rows]
        formats.append({"seed": seed, "cells": rows[0]["cells"], "targets": rows[0]["targets"], "sources": rows[0]["sources"],
                        "cpuMs": cpu, "gpuOwnerAllCostsMs": gpu, "cpuMeanMs": statistics.mean(cpu), "gpuMeanMs": statistics.mean(gpu),
                        "savedMeanMs": statistics.mean(cpu)-statistics.mean(gpu), "bothPairsFaster": all(a > b for a, b in zip(cpu, gpu)),
                        "pathScalarsChecked": sum(row["pathScalarsChecked"] for row in rows)})
    cpu_sum = sum(row["cpuMs"] for row in pairs)
    gpu_sum = sum(row["gpuIncludingFallbackMs"] for row in pairs)
    fixed_cost = start["initializationWaitMs"]+end["shutdownMs"]
    mean_saved = (cpu_sum-gpu_sum)/len(pairs)
    # First measured request, including any deferred JIT cost, remains in both
    # the six-request total and conservative repeated-mix break-even estimate.
    break_even = math.ceil(fixed_cost/mean_saved) if mean_saved > 0 else None
    ticks_per_sec = controller["cpuClockTicksPerSecond"]
    driver_ticks = sum(controller["sampledDriverCpuTicks"].values())
    stages = [row for row in events if row.get("event") == "owner_stage"]
    gpu_metrics = []
    for line in (directory / "probe.stderr").read_text().splitlines():
        if line.startswith("gpu_owner completed "):
            gpu_metrics.append(json.loads(line.removeprefix("gpu_owner completed ")))
    return {"validScreeningRun": True, "scope": "one idle process, 3 formats x 2 paired samples; not active search or match evidence",
            "formats": formats, "initializationMs": start["initializationWaitMs"], "shutdownMs": end["shutdownMs"],
            "sixCpuOwnersMs": cpu_sum, "sixGpuOwnersAllCostsMs": gpu_sum, "gpuSixOwnersWithStartupAndShutdownMs": fixed_cost+gpu_sum,
            "netSavedAcrossSixOwnersMs": cpu_sum-gpu_sum-fixed_cost, "meanSavedPerOwnerIncludingFirstRequestMs": mean_saved,
            "repeatedMixBreakEvenOwnerRequests": break_even, "firstGpuOwner": next(row for row in stages if row["mode"] == "gpu-attempt"),
            "gpuRuntimeMetricsInRequestOrder": gpu_metrics, "entireProbeWallMsIncludingSetupAndExtraComparisons": end["wallMs"],
            "processCpuMs": end["processCpuTicks"]*1000/ticks_per_sec,
            "initializationProcessCpuMs": start["processCpuTicks"]*1000/ticks_per_sec,
            "shutdownProcessCpuMs": end["shutdownProcessCpuTicks"]*1000/ticks_per_sec,
            "sampledProducerDriverCpuMsLowerBound": driver_ticks*1000/ticks_per_sec,
            "stages": stages,
            "c008CompletedV2Jobs1Through83": {"requests": 2685, "builds": 1404, "owners": 483, "private": 921,
                "ownerFractionOfBuilds": 483/1404, "ownerFractionOfRequests": 483/2685,
                "conditionalAllOwnersSameMixNetSavedMs": 483*mean_saved-fixed_cost,
                "breakEvenFitsAll483Owners": break_even is not None and break_even <= 483,
                "projectionCaveat": "hypothetical all-owner admission with this equal-format cost mix; readiness, busy admission, deadlines and actual input distribution reduce applicability"},
            "limitations": ["CPU off is the exact CPU builder while initialized GPU actor is idle; process-level GPU off overhead is not separately measured",
                            "owner time includes input adaptation, queue/wait, allocation, upload, kernel, readback, full verification, CPU tree DP and Graph assembly",
                            "cache lookup/publication, active CPU search, first-request scheduling overlap and match score are outside this probe",
                            "driver CPU sample is a lower bound from observed thread lifetimes; process CPU includes main and driver and comparison overhead",
                            "two repetitions are a screening test; first request/JIT is retained, not silently discarded"]}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("results", type=Path)
    args = parser.parse_args()
    print(json.dumps(summarize(args.results), indent=2))
