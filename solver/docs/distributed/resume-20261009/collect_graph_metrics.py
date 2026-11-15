#!/usr/bin/env python3
"""Read a manifest-pinned remote log range and summarize existing planner logs.

Remote access is read-only. Full captured bytes stay in the supplied .server path.
No solver process, setting, source, or evaluation manifest is modified.
"""
import argparse
import base64
import collections
import datetime
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shlex
import sys

REMOTE = r'''
import base64,json,os,socket,sys
path,start,end=json.loads(sys.argv[1])
with open(path,'rb') as f:
    st=os.fstat(f.fileno())
    if st.st_size<end: raise RuntimeError('recorded range exceeds current log size')
    f.seek(start); data=f.read(end-start)
if len(data)!=end-start: raise RuntimeError('short read')
print('PROCON37_CAPTURE_META '+json.dumps({'hostname':socket.gethostname(),'sourceSizeAtOpen':st.st_size,'inode':st.st_ino,'device':st.st_dev,'mtimeNsAtOpen':st.st_mtime_ns}))
print('PROCON37_CAPTURE_DATA')
sys.stdout.flush()
sys.stdout.buffer.write(base64.b64encode(data)+b'\n')
'''


def sha(data):
    return hashlib.sha256(data).hexdigest()


def capture(node, record, known_hosts):
    import pexpect
    password = os.environ.get('PROCON37_SSH_PASSWORD')
    if not password:
        raise RuntimeError('PROCON37_SSH_PASSWORD is required for password authentication')
    spec = [record['path'], record['capturedStartByte'], record['capturedEndByte']]
    # Both local and remote small read/parse work are constrained to CPU 1.
    command = shlex.join(['taskset', '-c', '1', 'python3', '-c', REMOTE, json.dumps(spec)])
    child = pexpect.spawn('ssh', ['-F', '/dev/null', '-o', 'ConnectTimeout=8',
        '-o', 'StrictHostKeyChecking=accept-new', '-o', f'UserKnownHostsFile={known_hosts}',
        f'{node["user"]}@{node["address"]}', command], encoding='utf-8', timeout=30,
        maxread=65536, echo=False)
    pieces = []
    attempts = 0
    try:
        while True:
            event = child.expect([r'(?i)password:', pexpect.EOF])
            pieces.append(child.before)
            if event == 1:
                break
            attempts += 1
            if attempts > 2:
                raise RuntimeError('SSH authentication failed')
            child.sendline(password)
        child.close()
        if child.exitstatus != 0:
            raise RuntimeError(f'SSH read failed: exit={child.exitstatus}, signal={child.signalstatus}')
    finally:
        if child.isalive():
            child.close(force=True)
    output = ''.join(pieces).replace('\r\n', '\n')
    match = re.search(r'^PROCON37_CAPTURE_META (.+)\nPROCON37_CAPTURE_DATA\n([A-Za-z0-9+/=\n]+)$', output, re.M)
    if not match:
        raise RuntimeError('remote read returned malformed framing')
    return base64.b64decode(match[2], validate=False), json.loads(match[1])


def quantiles(values):
    values = sorted(values)
    if not values:
        return {'count': 0}
    def q(p):
        position = (len(values)-1)*p
        lo, hi = math.floor(position), math.ceil(position)
        return round(values[lo]+(values[hi]-values[lo])*(position-lo), 3)
    return {'count':len(values), 'min':values[0], 'p10':q(.1), 'p50':q(.5),
        'p90':q(.9), 'p95':q(.95), 'p99':q(.99), 'max':values[-1],
        'mean':round(sum(values)/len(values), 3), 'sum':sum(values)}


def summarize(rows):
    return {'samples':len(rows), 'graphMs':quantiles([r['graph_ms'] for r in rows]),
        'iterations':quantiles([r['iterations'] for r in rows]),
        'graphSources':quantiles([r['graph_sources'] for r in rows]),
        'graphTargets':quantiles([r['graph_targets'] for r in rows]),
        'zeroGraphMs':sum(r['graph_ms']==0 for r in rows),
        'zeroIterations':sum(r['iterations']==0 for r in rows)}


def grouped(rows, key):
    groups = collections.defaultdict(list)
    for row in rows:
        groups[key(row)].append(row)
    return {name:summarize(group) for name,group in sorted(groups.items())}


def summarize_shared(lines, requests):
    pattern=re.compile(r'^shared_graph context=(\S+) (\{.*\})$')
    snapshots=[]
    for number,line in enumerate(lines,1):
        match=pattern.match(line)
        if match:
            snapshots.append({'line':number,'context':match[1],'values':json.loads(match[2])})
    if not snapshots:
        return {'available':False,'reason':'No structured shared_graph counter line in this captured range.'}
    first,last=snapshots[0],snapshots[-1]
    cumulative=['requests','hits','builds','completedBuilds','failures','waits','timeouts','bypasses',
        'evictions','buildMs','waitMs','materializedPaths','materializedBytes','budgetLimitedRequests']
    decreases=[]
    intervals=[]
    def difference(a,b):
        return {key:b['values'].get(key,0)-a['values'].get(key,0) for key in cumulative}
    for a,b in zip(snapshots,snapshots[1:]):
        delta=difference(a,b)
        if any(v < -1e-6 for v in delta.values()):
            decreases.append({'from':a['context'],'to':b['context']})
        included=[r for r in requests if a['line']<r.get('endLine',0)<b['line']]
        intervals.append({'format':included[0]['format'] if len(included)==1 else 'mixed_or_unresolved',
            'phase':included[0]['phase'] if len(included)==1 else 'mixed_or_unresolved',
            'coveredCompleteRequests':len(included),'counters':delta})
    if decreases:
        return {'available':True,'snapshotCount':len(snapshots),'firstSnapshot':first,'lastSnapshot':last,
            'comparableWindow':False,'counterResetIntervals':decreases,
            'reason':'Cumulative counters decreased; no combined build reduction is inferred across process resets.'}
    delta=difference(first,last)
    def rates(values):
        count=values['requests'];builds=values['builds'];waits=values['waits']
        return {'constructionAvoidanceFraction':round(1-builds/count,6) if count else None,
            'privateBypassFraction':round(values['bypasses']/count,6) if count else None,
            'privateCapacityBypasses':values['bypasses']-values['timeouts'],
            'privateWaitTimeoutBypasses':values['timeouts'],
            'ownerBuildStarts':builds-values['bypasses'],
            'meanBuildMs':round(values['buildMs']/values['completedBuilds'],3) if values['completedBuilds'] else None,
            'meanWaitMsPerWaitingCall':round(values['waitMs']/waits,3) if waits else None}
    covered=[r for r in requests if first['line']<r.get('endLine',0)<last['line']]
    missing=[r for r in requests if r.get('endLine',0)>last['line']]
    grouped_intervals={}
    for category in ['format','phase']:
        groups={}
        for interval in intervals:
            item=groups.setdefault(interval[category],{'completedRequests':0,'counters':{key:0 for key in cumulative}})
            item['completedRequests']+=interval['coveredCompleteRequests']
            for key,value in interval['counters'].items():item['counters'][key]+=value
        for item in groups.values():item['rates']=rates(item['counters'])
        grouped_intervals[category]=groups
    sampled={}
    for key in ['liveTables','liveBaseBytesEstimate','liveCellBytes','retainedEntries','retainedTableBytesEstimate']:
        sampled[key]=quantiles([s['values'][key] for s in snapshots])
    return {'available':True,'comparableWindow':True,'snapshotCount':len(snapshots),
        'scope':{'firstContext':first['context'],'lastContext':last['context'],
            'firstLine':first['line'],'lastLine':last['line'],
            'completeRequestsCovered':len(covered),'totalCompleteRequestsInRaw':sum('endLine' in r for r in requests),
            'jobsCovered':[r['job'] for r in covered],
            'jobsCompletedAfterLastCounter':[r['job'] for r in missing],
            'plannerRowsInCoveredCompleteRequests':sum(len(r['rows']) for r in covered),
            'lastContextIsBeforeRequest':last['context'].startswith('before_request_'),
            'hasDropTotal':any(s['context']=='search_pool_drop' for s in snapshots),
            'firstCountersStartAtZero':all(first['values'].get(key,0)==0 for key in cumulative),
            'meaning':'Counter deltas cover only first through last checkpoint; a last before_request_N excludes request N and its work. Full graph_ms quantiles still cover the complete raw evaluation.'},
        'firstSnapshot':first,'lastSnapshot':last,'deltaCounters':delta,'rates':rates(delta),
        'byCompletedRequestFormat':grouped_intervals['format'],'byCompletedRequestPhase':grouped_intervals['phase'],
        'sampledLiveAndRetained':sampled,
        'lastProcessLifetimePeakTableBytesEstimate':last['values'].get('peakTableBytesEstimate'),
        'definitions':{
            'constructionAvoidance':'1-buildStarts/GraphCacheRequests, relative to one build per acquisition; not an observed paired C007 CPU-time saving.',
            'capacityFallback':'bypasses-timeouts; in this implementation this means all retained entry slots are Building when acquiring a distinct key. Does not include ordinary LRU eviction.',
            'scope':'Per-format/phase counter deltas group whole completed requests, including primary and its forecasts; counters cannot identify the internal stage responsible for a bypass.',
            'timing':'buildMs and waitMs sum wall durations across threads, not process wall time or CPU usage. Snapshots are concurrent atomic observations, not a transactional freeze.',
            'memory':'Live/retained tables are byte estimates sampled at request boundaries. peakTableBytesEstimate counts fully built attached tables and lazy cells, omits partially constructed tables, private memos, allocator overhead and RSS retention.',
            'peakScope':'Peak is process-lifetime as of the last snapshot; cannot subtract first peak to get a stage-only peak if a process carried over.',
            'counterNames':'Structured timeouts/failures values must be used; searching for the word timeout in a shared_graph JSON line counts the field name even when zero.'}}


def parse(data, cases):
    lines = data.decode('utf-8', errors='strict').splitlines()
    begin = re.compile(r'^dist worker request=(\d+) job=(\d+) phase=(select|plan) .* started=(\d+) budget_ms=(\d+)$')
    end = re.compile(r'^dist worker request=(\d+) completed=(\d+)/(\d+) .* elapsed_ms=(\d+) ')
    metric = re.compile(r'^g007 day=(\d+) score=[^ ]+ iterations=(\d+).* graph_targets=(\d+) graph_sources=(\d+) graph_ms=(\d+)$')
    active = None
    requests, rows, orphan, malformed = [], [], [], []
    case_index = -1
    for line_number, line in enumerate(lines, 1):
        match = begin.match(line)
        if match:
            request_id, job, phase, started, budget = match.groups()
            if phase == 'select':
                case_index += 1
            case = cases[case_index] if 0 <= case_index < len(cases) else {}
            active = {'caseIndex':case_index, 'seed':case.get('seed'), 'format':case.get('format', 'unknown'),
                'requestId':int(request_id), 'job':int(job), 'phase':phase, 'started':int(started),
                'budgetMs':int(budget), 'rows':[], 'beginLine':line_number}
            requests.append(active)
            continue
        match = metric.match(line)
        if match:
            day, iterations, targets, sources, graph_ms = map(int, match.groups())
            row = {'day':day, 'iterations':iterations, 'graph_targets':targets, 'graph_sources':sources, 'graph_ms':graph_ms}
            if active is None:
                orphan.append(line_number)
                row.update(format='unattributed', seed=None, phase='unattributed', stage='unattributed')
            else:
                expected_day = 0 if active['phase']=='select' else active['requestId']-2
                stage = 'primary' if day==expected_day else ('forecast' if day>expected_day else 'earlier_or_stale')
                row.update(format=active['format'], seed=active['seed'], phase=active['phase'], stage=stage)
                active['rows'].append(row)
            rows.append(row)
            continue
        match = end.match(line)
        if match and active is not None:
            request_id, completed, started, elapsed = map(int, match.groups())
            if request_id != active['requestId']:
                raise RuntimeError(f'request completion mismatch at line {line_number}')
            active.update(completed=completed, expected=started, elapsedMs=elapsed, endLine=line_number)
            active = None
            continue
        if line.startswith('g007 day='):
            malformed.append(line_number)
    complete = [r for r in requests if 'completed' in r]
    matched_cases = case_index+1 == len(cases)
    warnings = []
    if not matched_cases:
        warnings.append('select request count differs from run case count; case/format mapping is provisional')
    if orphan:
        warnings.append('planner rows outside a live request are unattributed')
    if malformed:
        warnings.append('planner-shaped rows did not match full metric grammar')
    if len(complete)!=len(requests):
        warnings.append('not every started request completed within captured range')
    if any(r['completed']!=r['expected'] for r in complete):
        warnings.append('some requests have incomplete worker participation')
    fractions = []
    request_groups = collections.defaultdict(list)
    for request in complete:
        if request['started'] and request['elapsedMs']:
            fraction=sum(r['graph_ms'] for r in request['rows'])/(request['started']*request['elapsedMs'])
            fractions.append(fraction)
            request_groups[request['format']].append(fraction)
    return {'lineCount':len(lines), 'plannerRows':len(rows),
        'sharedGraph':summarize_shared(lines,requests),
        'allRows':summarize(rows), 'byFormat':grouped(rows, lambda r:r['format']),
        'byPhaseStage':grouped(rows, lambda r:f'{r["phase"]}.{r["stage"]}'),
        'byFormatPhaseStage':grouped(rows, lambda r:f'{r["format"]}.{r["phase"]}.{r["stage"]}'),
        'bySeed':grouped(rows, lambda r:str(r['seed'])),
        'requests':{'started':len(requests), 'completed':len(complete),
            'allExpectedWorkersCompleted':len(complete)==len(requests) and all(r['completed']==r['expected'] for r in complete),
            'selectCount':case_index+1, 'runCaseCount':len(cases), 'caseOrderMappingValidatedByCount':matched_cases,
            'workersStarted':quantiles([r['started'] for r in requests]),
            'budgetMs':quantiles([r['budgetMs'] for r in requests]),
            'elapsedMs':quantiles([r['elapsedMs'] for r in complete]),
            'loggedGraphMsOverStartedWorkerWallMs':quantiles(fractions),
            'graphFractionByFormat':{k:quantiles(v) for k,v in sorted(request_groups.items())}},
        'parseDiagnostics':{'unattributedPlannerRowCount':len(orphan), 'malformedPlannerRowCount':len(malformed),
            'unattributedLineExamples':orphan[:8], 'malformedLineExamples':malformed[:8], 'warnings':warnings}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--completion', type=Path, required=True)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--run', type=Path, required=True)
    parser.add_argument('--node', required=True, choices=['v2','v3'])
    parser.add_argument('--raw', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--known-hosts', default='/tmp/procon37-terminal-known-hosts')
    parser.add_argument('--reuse-raw', action='store_true')
    parser.add_argument('--planner', type=Path, help='evaluated target planner source used to interpret log metrics')
    parser.add_argument('--baseline', type=Path, help='same-node collector JSON for observational quantile comparison')
    args = parser.parse_args()
    if hasattr(os, 'sched_getaffinity') and os.sched_getaffinity(0)!={1}:
        raise RuntimeError('run this collector under taskset -c 1')
    completion = json.loads(args.completion.read_text())
    manifest = json.loads(args.manifest.read_text())
    run = json.loads(args.run.read_text())
    record = completion['log']
    if not 0 <= record['capturedStartByte'] <= record['capturedEndByte']:
        raise RuntimeError('invalid captured range')
    metadata_path = args.raw.with_suffix(args.raw.suffix+'.capture.json')
    if args.reuse_raw:
        data = args.raw.read_bytes()
        remote = json.loads(metadata_path.read_text())
    else:
        if args.raw.exists() or metadata_path.exists():
            raise RuntimeError('raw capture exists; use --reuse-raw to verify without overwriting')
        data, remote = capture(manifest['nodes'][args.node], record, args.known_hosts)
    actual = sha(data)
    if actual != record['sha256OfCapturedByteRange']:
        raise RuntimeError(f'captured range SHA mismatch: {actual}')
    if len(data)!=record['capturedEndByte']-record['capturedStartByte']:
        raise RuntimeError('captured range byte count mismatch')
    if remote['hostname'].lower()!=completion['hostname'].lower():
        raise RuntimeError('remote hostname mismatch')
    if not data.endswith(b'\n'):
        raise RuntimeError('captured byte range ends in a partial log line')
    if not args.reuse_raw:
        args.raw.parent.mkdir(parents=True, exist_ok=True)
        args.raw.write_bytes(data)
        metadata_path.write_text(json.dumps(remote, indent=2)+'\n')
    result = {'schemaVersion':2, 'createdAtUtc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'version':manifest['version'], 'runId':run['id'], 'label':run['config'].get('label'), 'node':args.node,
        'hostname':remote['hostname'], 'cpuAffinityLocalAndRemoteCollector':'1',
        'source':{'completion':str(args.completion), 'completionSha256':sha(args.completion.read_bytes()),
            'manifest':str(args.manifest), 'manifestSha256':sha(args.manifest.read_bytes()), 'run':str(args.run),
            'runSha256':sha(args.run.read_bytes()), 'rawLog':str(args.raw), 'remotePath':record['path'],
            'capturedStartByte':record['capturedStartByte'], 'capturedEndByte':record['capturedEndByte'],
            'capturedBytes':len(data), 'rangeSha256':actual, 'rangeSha256Verified':True,
            'currentRemoteSizeAtRead':remote['sourceSizeAtOpen'], 'recordedInode':record.get('inode'),
            'currentInode':remote['inode'], 'inodeUnchanged':remote['inode']==record.get('inode')},
        'definitions':{
            'quantiles':'Linear interpolation on sorted samples at (N-1)*p, not nearest-rank; integer ms logs are truncated by Rust as_millis().',
            'graphMs':'Wall duration around Graph acquisition in plan_inner only. Includes scheduling delay, cache reuse, and shared cache waiting if present; excludes initial feasible-plan construction, search, later forecast graph construction, and submission. Not CPU time.',
            'iterations':'Outer stochastic search-loop count for one logged plan_inner call. Includes different role weights, primary/forecast budgets, and different trajectories; not fixed-work throughput.',
            'sample':'One completed g007 plan_inner diagnostic across all worker threads, not one match or one official score.',
            'phaseStage':'Phase from preceding dist worker request. Primary day=0 for select, requestId-2 for plan; greater day is forecast. No worker ID in g007 line, so no per-worker pairing.',
            'caseMapping':'Select requests are associated in log order with serial server run cases. Count validation is necessary, not proof of identical input trajectories.',
            'graphFraction':'Sum of logged graph_ms per request / (workers started * request elapsed_ms). Scheduling-inclusive descriptive ratio, not measured CPU utilization or guaranteed recoverable wall time.',
            'score':'Internal g007 score is deliberately not aggregated as official score; worker weights and day vary.',
            'limits':'No GPU attribution, per-stage deadlines, route-query time, or full cache hit metric exists in D0/C007 logs. Format comparisons across versions are observational, not paired identical graph inputs.'},
        'metrics':parse(data,run['cases'])}
    result['processMemoryAtCompletion']=completion.get('processes',[])
    result['collector']={'path':str(Path(__file__)), 'sha256':sha(Path(__file__).read_bytes())}
    if args.planner:
        result['loggingSource']={'path':str(args.planner), 'sha256':sha(args.planner.read_bytes())}
    if args.baseline:
        baseline=json.loads(args.baseline.read_text())
        if baseline['node']!=result['node']:
            raise RuntimeError('baseline must be from the same host')
        comparison={'baseline':str(args.baseline), 'baselineSha256':sha(args.baseline.read_bytes()),
            'baselineRunId':baseline['runId'], 'kind':'observational quantiles, not paired identical input or strength evidence'}
        for category in ['byFormat','byPhaseStage','byFormatPhaseStage']:
            comparison[category]={}
            for name,current in result['metrics'][category].items():
                old=baseline['metrics'][category].get(name)
                if not old: continue
                changes={'baselineSamples':old['samples'],'candidateSamples':current['samples']}
                for metric in ['graphMs','iterations']:
                    changes[metric]={}
                    for q in ['p50','p90','mean']:
                        before,after=old[metric][q],current[metric][q]
                        changes[metric][q]={'baseline':before,'candidate':after,
                            'candidateOverBaseline':round(after/before,4) if before else None}
                comparison[category][name]=changes
        baseline_completion=Path(baseline['source']['completion'])
        old_completion=json.loads(baseline_completion.read_text())
        comparison['processMemoryAtCompletion']={
            'candidate':completion.get('processes',[]),'baseline':old_completion.get('processes',[]),
            'baselineCompletionSha256':sha(baseline_completion.read_bytes()),
            'meaning':'Process RSS/HWM from recorded completion only; check per-process lifetime scope. Counter table bytes are not RSS.'}
        result['comparison']=comparison
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2)+'\n')
    print(json.dumps({'node':args.node,'sha256':actual,'bytes':len(data),'samples':result['metrics']['plannerRows'],
        'byFormat':{k:{'samples':v['samples'],'graphP50Ms':v['graphMs'].get('p50'),'graphP90Ms':v['graphMs'].get('p90'),'iterationsP50':v['iterations'].get('p50')} for k,v in result['metrics']['byFormat'].items()},
        'warnings':result['metrics']['parseDiagnostics']['warnings']}, ensure_ascii=False))

if __name__=='__main__':
    main()
