#!/usr/bin/env python3
"""Finite plan-stage timing microbench; not a server match or adoption evaluation."""
import datetime,hashlib,json,os,pathlib,resource,selectors,signal,subprocess,time
ROOT=pathlib.Path(__file__).resolve().parent
RESULTS=ROOT/'results';RESULTS.mkdir(exist_ok=True)
CPUS=list(range(16))
ENV={k:v for k,v in os.environ.items() if not k.startswith('PROCON37_')}
ENV.update(PROCON37_G007_FUTURE_WEIGHT='0',PROCON37_G007_RESERVE_WEIGHT='15')
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def process_status(pid):
 try:
  fields=dict(line.split(':',1) for line in pathlib.Path(f'/proc/{pid}/status').read_text().splitlines() if ':' in line)
  return {k:fields[k].strip() for k in ['Threads','Cpus_allowed_list','VmRSS','VmHWM']}
 except (FileNotFoundError,ProcessLookupError):return {}
def cpu_settings():
 result={}
 for p in pathlib.Path('/sys/devices/system/cpu/cpufreq').glob('policy*'):
  result[p.name]={k:(p/k).read_text().strip() for k in ['scaling_governor','energy_performance_preference'] if (p/k).exists()}
 return result
all_results=[]
metadata={'purpose':'finite CPU timing microbench; not server short12, not formal, not adoption','host':os.uname().nodename,'cpus':CPUS,'environment':{k:ENV[k] for k in ['PROCON37_G007_FUTURE_WEIGHT','PROCON37_G007_RESERVE_WEIGHT']},'startedAt':datetime.datetime.now(datetime.timezone.utc).isoformat(),'cpuSettingsBefore':cpu_settings(),'preflightProcesses':subprocess.check_output(['ps','-eo','pid,comm,pcpu,nlwp','--sort=-pcpu'],text=True).splitlines()[:20], 'binaries':{v:sha(ROOT/'bin'/v) for v in ['d0','c006']}}
(ROOT/'metadata.json').write_text(json.dumps(metadata,indent=2)+'\n')
for pair,seed in enumerate([0,1,2]):
 for version in (['d0','c006'] if pair%2==0 else ['c006','d0']):
  label=f'pair{pair+1}-seed{seed}-{version}';out=RESULTS/label;out.mkdir(exist_ok=False)
  requests=(ROOT/'inputs'/f'seed-{seed}.jsonl').read_bytes().splitlines(keepends=True)
  cmd=['taskset','-c','0-15',str(ROOT/'bin'/version),'--mode','parallel','--threads','16','--seed','0']
  row={'label':label,'seed':seed,'version':version,'command':cmd,'inputSha256':sha(ROOT/'inputs'/f'seed-{seed}.jsonl'),'binarySha256':metadata['binaries'][version],'requests':[],'timedOut':False}
  usage0=resource.getrusage(resource.RUSAGE_CHILDREN);began=time.monotonic()
  proc=None
  with (out/'stderr.log').open('wb') as err,(out/'stdout.jsonl').open('wb') as output:
   try:
    proc=subprocess.Popen(cmd,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=err,env=ENV,start_new_session=True)
    row['pid']=proc.pid
    (out/'process.json').write_text(json.dumps({'pid':proc.pid,'command':cmd,'stopped':False},indent=2))
    select=selectors.DefaultSelector();select.register(proc.stdout,selectors.EVENT_READ)
    for raw in requests:
     request=json.loads(raw);start=time.monotonic();proc.stdin.write(raw);proc.stdin.flush()
     if not select.select(timeout=20):row['timedOut']=True;raise TimeoutError('no response in 20 seconds')
     line=proc.stdout.readline();elapsed=1000*(time.monotonic()-start)
     if not line:raise RuntimeError(f'early solver exit {proc.poll()}')
     answer=json.loads(line);assert answer['requestId']==request['requestId']
     output.write(line);output.flush()
     row['requests'].append({'phase':request['phase'],'limitMs':request['timeLimitMs'],'elapsedMs':elapsed,'processStatus':process_status(proc.pid)})
    select.close();proc.stdin.close();proc.wait(timeout=3)
    row['exitCode']=proc.returncode
   except Exception as exc:row['error']=str(exc)
   finally:
    if proc is not None:
     if proc.poll() is None:os.killpg(proc.pid,signal.SIGKILL);proc.wait(timeout=3)
     row['exitCode']=proc.returncode;row['stopped']=True
     (out/'process.json').write_text(json.dumps({'pid':proc.pid,'command':cmd,'stopped':True,'exitCode':proc.returncode},indent=2)+'\n')
  usage=resource.getrusage(resource.RUSAGE_CHILDREN)
  row.update(wallMs=1000*(time.monotonic()-began),userCpuSeconds=usage.ru_utime-usage0.ru_utime,systemCpuSeconds=usage.ru_stime-usage0.ru_stime)
  (out/'measurement.json').write_text(json.dumps(row,indent=2)+'\n');all_results.append(row)
  (ROOT/'measurements.json').write_text(json.dumps(all_results,indent=2)+'\n')
  print(json.dumps({'label':label,'planElapsedMs':row['requests'][-1]['elapsedMs'] if row['requests'] else None,'exitCode':row.get('exitCode'),'error':row.get('error')}),flush=True)
  if row.get('error') or row['exitCode']!=0:raise SystemExit(1)
metadata['finishedAt']=datetime.datetime.now(datetime.timezone.utc).isoformat();metadata['cpuSettingsAfter']=cpu_settings();metadata['allStopped']=True
(ROOT/'metadata.json').write_text(json.dumps(metadata,indent=2)+'\n')
