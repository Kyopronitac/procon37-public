//! Optional, owner-only GPU table producer. Nothing here runs a driver by default.
use super::{GraphTables, PreparedGraph, Path, RouteTree, SharedAllocation, PATH_MODES};
use gpu_tables::{Prepared, VerifiedTables, INF, NO_CELL};
use gpu_tables::producer::{Producer, Config, Confinement, Computed, Phase};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub(super) fn check_deadline(deadline:Option<Instant>)->Result<(),String> {
    if deadline.is_some_and(|d|Instant::now()>=d){Err("GPU assembly deadline elapsed".into())}else{Ok(())}
}
fn error(message:impl Into<String>)->gpu_tables::Error {gpu_tables::Error(message.into())}
fn nanos(duration:Duration)->u64 {duration.as_nanos().min(u128::from(u64::MAX))as u64}

pub(super) fn prepare(key:&PreparedGraph)->gpu_tables::Result<Arc<Prepared>> {
    let n=key.map.len();
    if n>gpu_tables::MAX_CELLS||key.targets>key.positions.len() {return Err(error("GPU graph shape outside limits"));}
    let neighbors=(0..n).map(|y|std::array::from_fn(|dir|key.map.neighbor(y,dir).map_or(NO_CELL,|x|x as u16))).collect();
    let input=Prepared{neighbors,passable:(0..n).map(|y|u8::from(key.map.terrain(y)!=3)).collect(),
        durations:key.durations.clone(),costs:key.costs.clone(),shops:key.spot_at.iter().map(|&x|u8::from(x>=0)).collect(),
        targets:key.positions[..key.targets].iter().map(|&x|u16::try_from(x).map_err(|_|error("target cell overflow"))).collect::<gpu_tables::Result<_>>()?};
    input.validate()?;Ok(Arc::new(input))
}

impl GraphTables {
    fn from_gpu(key:&PreparedGraph, expected:&Prepared, verified:VerifiedTables, deadline:Instant)->Result<Self,String> {
        check_deadline(Some(deadline))?;
        if verified.input()!=expected {return Err("GPU response belongs to a different graph".into());}
        let scalars=verified.scalars_until(Some(deadline)).map_err(|e|e.0)?;
        let n=key.map.len();let targets=key.targets;let sources=key.positions.len();
        let raw=verified.raw();let allocation=Arc::new(SharedAllocation::default());
        let mut variants=Vec::with_capacity(PATH_MODES);
        for mode in 0..PATH_MODES {
            check_deadline(Some(deadline))?;
            let mut paths=vec![vec![Path::default();targets];sources];
            for j in 0..targets {
                check_deadline(Some(deadline))?;
                let base=(mode*targets+j)*n;
                let tree=Arc::new(RouteTree{target:key.positions[j]as u16,
                    next:raw.next[base..base+n].to_vec().into_boxed_slice(),
                    fingerprints:scalars.fingerprints[base..base+n].to_vec().into_boxed_slice(),allocation:allocation.clone()});
                for (i,&source) in key.positions.iter().enumerate(){
                    if i%64==0{check_deadline(Some(deadline))?;}
                    if source>=n||raw.distances[base+source]==INF{return Err("unreachable GPU graph source".into());}
                    paths[i][j]=Path{tree:Some(tree.clone()),source:source as u16,
                        length:scalars.lengths[base+source],time:scalars.times[base+source],fuel:scalars.fuels[base+source],cells:OnceLock::new()};
                }
            }
            variants.push(paths);
        }
        let mut fuel_to=vec![vec![];sources];
        for (j,row) in fuel_to.iter_mut().enumerate().take(targets){
            check_deadline(Some(deadline))?;
            let base=(PATH_MODES*targets+j)*n;
            *row=raw.distances[base..base+n].iter().map(|&distance|if distance==INF{99999}else{distance}).collect();
        }
        // Large temporary scalar/distance arrays need not coexist with mode-mask construction.
        drop(scalars);drop(verified);
        Self::assemble(key,variants,fuel_to,allocation,Some(deadline))
    }
}

#[derive(Clone)]
struct Policy { cap:Duration, max_temporary_bytes:usize }
impl Default for Policy {
    fn default()->Self {Self{cap:Duration::from_millis(250),max_temporary_bytes:64*1024*1024}}
}
impl Policy {
    fn budget(&self,remaining:Duration,cpu_estimate:Duration)->Duration {
        let reserve=cpu_estimate.saturating_add(cpu_estimate/2).saturating_add(Duration::from_millis(20));
        self.cap.min(remaining/8).min(remaining.saturating_sub(reserve))
    }
}
trait Backend:Send+Sync {
    fn ready(&self)->bool;
    fn compute(&self,input:Arc<Prepared>,deadline:Instant)->gpu_tables::Result<Computed>;
    fn shutdown(&self,deadline:Instant)->gpu_tables::Result<()>;
    fn diagnostic(&self)->String {String::new()}
}
impl Backend for Producer {
    fn ready(&self)->bool {self.status().phase==Phase::Ready}
    fn compute(&self,input:Arc<Prepared>,deadline:Instant)->gpu_tables::Result<Computed>{Producer::compute(self,input,deadline)}
    fn shutdown(&self,deadline:Instant)->gpu_tables::Result<()>{Producer::shutdown(self,deadline)}
    fn diagnostic(&self)->String {format!("{:?}",self.status())}
}
#[derive(Default)]
struct Counters {
    attempts:AtomicU64, completed:AtomicU64, not_ready:AtomicU64, budget_skips:AtomicU64,
    shape_skips:AtomicU64, errors:AtomicU64, assembly_errors:AtomicU64,
    prepare_ns:AtomicU64, wait_ns:AtomicU64, assembly_ns:AtomicU64,
}
pub(super) struct GraphGpu {backend:Box<dyn Backend>,policy:Policy,counters:Counters,disabled:AtomicBool}
impl GraphGpu {
    /// Explicitly opt in on v2 only. CPU-only unit tests never call this factory.
    pub(super) fn from_environment()->Option<Arc<Self>> {
        let requested=std::env::var("PROCON37_C009_GPU").unwrap_or_else(|_|"off".into());
        if requested=="off" {return None;}
        let start=(||->gpu_tables::Result<Self>{
            if requested!="opencl"{return Err(error("unknown C009 GPU mode"));}
            let hostname=std::fs::read_to_string("/proc/sys/kernel/hostname").map_err(|e|error(e.to_string()))?;
            if !hostname.trim().eq_ignore_ascii_case("VOIDLINUX-2"){return Err(error("C009 initial GPU policy permits VOIDLINUX-2 only"));}
            let group=PathBuf::from(std::env::var("PROCON37_C009_GPU_CPUSET").map_err(|_|error("C009 GPU cpuset is required"))?);
            let selector=std::env::var("PROCON37_C009_GPU_DEVICE").unwrap_or_else(|_|"RTX 5060 Laptop GPU".into());
            if selector.trim().is_empty(){return Err(error("empty GPU device selector"));}
            let mut policy=Policy::default();
            if let Ok(value)=std::env::var("PROCON37_C009_GPU_CAP_MS") {
                let ms=value.parse::<u64>().map_err(|_|error("invalid GPU cap"))?;
                if !(1..=1000).contains(&ms){return Err(error("GPU cap must be 1..1000 ms"));}
                policy.cap=Duration::from_millis(ms);
            }
            if let Ok(value)=std::env::var("PROCON37_C009_GPU_TEMP_MIB") {
                let mib=value.parse::<usize>().map_err(|_|error("invalid GPU temporary limit"))?;
                if !(1..=256).contains(&mib){return Err(error("GPU temporary limit must be 1..256 MiB"));}
                policy.max_temporary_bytes=mib*1024*1024;
            }
            let gate_group=group.clone();
            let backend=Producer::spawn(Config{device_contains:selector.clone(),confinement:Confinement{cpu:15,group:group.clone()}},move|tid|join_delegated_group(&gate_group,tid))?;
            eprintln!("gpu_owner startup cpu=15 device={selector:?} cpuset={} cap_ms={} temporary_limit={} state=asynchronous",group.display(),policy.cap.as_millis(),policy.max_temporary_bytes);
            Ok(Self{backend:Box::new(backend),policy,counters:Counters::default(),disabled:AtomicBool::new(false)})
        })();
        match start {Ok(service)=>Some(Arc::new(service)),Err(e)=>{eprintln!("gpu_owner disabled_before_start error={e:?}");None}}
    }
    pub(super) fn try_build(&self,key:&PreparedGraph,deadline:Instant,cpu_estimate:Duration)->Option<GraphTables> {
        if self.disabled.load(Ordering::Acquire)||!self.backend.ready(){self.counters.not_ready.fetch_add(1,Ordering::Relaxed);return None;}
        let started=Instant::now();let budget=self.policy.budget(deadline.saturating_duration_since(started),cpu_estimate);
        if budget<Duration::from_millis(1){self.counters.budget_skips.fetch_add(1,Ordering::Relaxed);return None;}
        let gpu_deadline=started+budget;
        let temporary=key.map.len().checked_mul(key.targets).and_then(|v|v.checked_mul(148)).unwrap_or(usize::MAX);
        if temporary>self.policy.max_temporary_bytes {self.counters.shape_skips.fetch_add(1,Ordering::Relaxed);return None;}
        let input=match prepare(key){Ok(input)=>input,Err(reason)=>{
            self.counters.shape_skips.fetch_add(1,Ordering::Relaxed);eprintln!("gpu_owner shape_skip error={reason:?}");return None;
        }};
        self.counters.prepare_ns.fetch_add(nanos(started.elapsed()),Ordering::Relaxed);
        if Instant::now()>=gpu_deadline{self.counters.budget_skips.fetch_add(1,Ordering::Relaxed);return None;}
        self.counters.attempts.fetch_add(1,Ordering::Relaxed);
        let waiting=Instant::now();
        // Producer uses atomic single-flight admission and try_send: never queue behind another owner.
        let computed=match self.backend.compute(input.clone(),gpu_deadline){Ok(output)=>output,Err(reason)=>{
            self.counters.wait_ns.fetch_add(nanos(waiting.elapsed()),Ordering::Relaxed);
            self.counters.errors.fetch_add(1,Ordering::Relaxed);eprintln!("gpu_owner cpu_fallback error={reason:?}");return None;
        }};
        let wait_ns=nanos(waiting.elapsed());self.counters.wait_ns.fetch_add(wait_ns,Ordering::Relaxed);
        let assembling=Instant::now();
        let result=GraphTables::from_gpu(key,&input,computed.tables,gpu_deadline);
        let assembly_ns=nanos(assembling.elapsed());self.counters.assembly_ns.fetch_add(assembly_ns,Ordering::Relaxed);
        match result {
            Ok(tables)=>{
                self.counters.completed.fetch_add(1,Ordering::Relaxed);
                let m=computed.metrics;
                eprintln!("gpu_owner completed {}",serde_json::json!({"cells":key.map.len(),"targets":key.targets,"temporaryBytesEstimate":temporary,
                    "waitMs":wait_ns as f64/1e6,"assemblyMs":assembly_ns as f64/1e6,"device":m.device_name,
                    "initializationMs":m.initialization_ms,"allocationMs":m.allocation_ms,"uploadMs":m.upload_ms,
                    "executionReadMs":m.execution_and_read_ms,"verificationMs":m.verification_ms,"sequence":m.request_sequence,"rounds":m.rounds_max}));
                Some(tables)
            }
            Err(reason)=>{
                self.disabled.store(true,Ordering::Release);self.counters.assembly_errors.fetch_add(1,Ordering::Relaxed);
                eprintln!("gpu_owner disabled_after_assembly error={reason:?}");None
            }
        }
    }
    pub(super) fn shutdown(&self,deadline:Instant) {
        self.disabled.store(true,Ordering::Release);
        let result=self.backend.shutdown(deadline);eprintln!("gpu_owner shutdown={result:?} counters={}",self.snapshot());
    }
    pub(super) fn snapshot(&self)->serde_json::Value {
        let c=&self.counters;
        serde_json::json!({"attempts":c.attempts.load(Ordering::Relaxed),"completed":c.completed.load(Ordering::Relaxed),
            "notReady":c.not_ready.load(Ordering::Relaxed),"budgetSkips":c.budget_skips.load(Ordering::Relaxed),"shapeSkips":c.shape_skips.load(Ordering::Relaxed),
            "producerErrors":c.errors.load(Ordering::Relaxed),"assemblyErrors":c.assembly_errors.load(Ordering::Relaxed),
            "prepareMs":c.prepare_ns.load(Ordering::Relaxed)as f64/1e6,"waitMs":c.wait_ns.load(Ordering::Relaxed)as f64/1e6,
            "assemblyMs":c.assembly_ns.load(Ordering::Relaxed)as f64/1e6,"disabled":self.disabled.load(Ordering::Acquire),"ready":self.backend.ready(),"producerStatus":self.backend.diagnostic()})
    }
}

#[cfg(test)]
pub(super) enum MockBehavior {Exact, Error, NotReady, WrongInput, Late, Panic}
#[cfg(test)]
impl GraphGpu {
    pub(super) fn testing(behavior:MockBehavior)->Arc<Self> {
        struct Mock(MockBehavior);
        impl Backend for Mock {
            fn ready(&self)->bool {!matches!(self.0,MockBehavior::NotReady)}
            fn compute(&self,mut input:Arc<Prepared>,_deadline:Instant)->gpu_tables::Result<Computed> {
                if matches!(self.0,MockBehavior::Panic){panic!("injected GPU owner panic");}
                if matches!(self.0,MockBehavior::Error){return Err(error("injected unavailable/busy/driver error"));}
                if matches!(self.0,MockBehavior::Late){std::thread::sleep(Duration::from_millis(30));}
                if matches!(self.0,MockBehavior::WrongInput){let mut wrong=(*input).clone();wrong.costs[0]=3-wrong.costs[0];input=Arc::new(wrong);}
                let tables=gpu_tables::cpu_reference(&input)?.verify(input)?;
                Ok(Computed{tables,metrics:gpu_tables::producer::Metrics{device_name:"CPU-only mock".into(),initialization_ms:0.0,allocation_ms:0.0,upload_ms:0.0,
                    execution_and_read_ms:0.0,verification_ms:0.0,rounds_max:1,request_sequence:1,device_allocated_bytes:0}})
            }
            fn shutdown(&self,_deadline:Instant)->gpu_tables::Result<()>{Ok(())}
        }
        Arc::new(Self{backend:Box::new(Mock(behavior)),policy:Policy::default(),counters:Counters::default(),disabled:AtomicBool::new(false)})
    }
}

/// Root delegates only this group's tasks file. No cgroup.procs/TGID migration.
/// A cgroup command must be one write, including its trailing newline. Do not
/// use write_fmt/write_all: a partial write must fail rather than send a second
/// command containing the remaining bytes.
fn write_delegated_tid_once(writer:&mut impl std::io::Write,tid:u32)->gpu_tables::Result<()> {
    let command=format!("{tid}\n");
    let written=writer.write(command.as_bytes()).map_err(|e|error(format!("write delegated cpuset tasks tid={tid}: {e}")))?;
    if written!=command.len(){return Err(error(format!("short write delegated cpuset tasks tid={tid}: {written}/{} bytes",command.len())));}
    Ok(())
}

fn join_delegated_group(group:&std::path::Path,tid:u32)->gpu_tables::Result<()> {
    use std::os::unix::fs::MetadataExt;
    if gpu_tables::producer::current_tid()!=Ok(tid){return Err(error("gate TID does not identify current thread"));}
    let actual=group.canonicalize().map_err(|e|error(e.to_string()))?;
    if actual!=group||actual.parent()!=Some(std::path::Path::new("/sys/fs/cgroup/cpuset"))
        || !actual.file_name().and_then(|s|s.to_str()).is_some_and(|s|s.starts_with("procon37-c009-v2-")){
        return Err(error("unexpected delegated cpuset path"));
    }
    for path in [actual.clone(),actual.join("cpuset.cpus"),actual.join("cpuset.mems")] {
        let meta=std::fs::metadata(path).map_err(|e|error(e.to_string()))?;
        if meta.uid()!=0||meta.mode()&0o022!=0{return Err(error("cpuset controls are not exclusively root managed"));}
    }
    let status=std::fs::read_to_string("/proc/thread-self/status").map_err(|e|error(e.to_string()))?;
    let uid=status.lines().find_map(|line|line.strip_prefix("Uid:")).and_then(|s|s.split_whitespace().nth(1)).and_then(|s|s.parse::<u32>().ok()).ok_or_else(||error("effective UID missing"))?;
    let tasks=actual.join("tasks");let meta=std::fs::metadata(&tasks).map_err(|e|error(e.to_string()))?;
    if uid==0||meta.uid()!=uid||meta.mode()&0o777!=0o600{return Err(error("tasks must be delegated 0600 to this non-root UID"));}
    let mut file=std::fs::OpenOptions::new().write(true).open(&tasks).map_err(|e|error(format!("open delegated cpuset tasks {}: {e}",tasks.display())))?;
    write_delegated_tid_once(&mut file,tid)?;
    Confinement{cpu:15,group:actual}.verify_current()?;
    eprintln!("gpu_owner gate_joined tid={tid} cpu=15");Ok(())
}

#[cfg(test)]
#[path="gpu_graph_tests.rs"]
mod tests;
