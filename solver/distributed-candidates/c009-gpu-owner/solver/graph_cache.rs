//! Pool-scoped, bounded sharing of immutable shortest-path tables.
//! No graph construction or large table destruction occurs under the map lock.
use super::{GraphTables, PreparedGraph};
use super::gpu_graph::GraphGpu;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
const BUILDING:u8=0;
const READY:u8=1;
const FAILED:u8=2;
fn lock<T>(mutex:&Mutex<T>)->MutexGuard<'_,T>{mutex.lock().unwrap_or_else(|poison|poison.into_inner())}

#[derive(Default)]
struct Counters {
    requests:AtomicU64, hits:AtomicU64, builds:AtomicU64, completed:AtomicU64,
    failures:AtomicU64, waits:AtomicU64, timeouts:AtomicU64, bypasses:AtomicU64, evictions:AtomicU64,
    build_ns:AtomicU64, wait_ns:AtomicU64, build_ns_per_work:AtomicU64, budget_limited:AtomicU64,
    cpu_builds:AtomicU64, cpu_build_ns:AtomicU64, gpu_builds:AtomicU64,
    live_tables:AtomicUsize, live_base_bytes:AtomicUsize, live_cell_bytes:AtomicUsize,
    peak_bytes:AtomicUsize, materialized_paths:AtomicU64, materialized_bytes:AtomicU64,
}
impl Counters {
    fn peak(&self){self.peak_bytes.fetch_max(self.live_base_bytes.load(Ordering::Relaxed)+self.live_cell_bytes.load(Ordering::Relaxed),Ordering::Relaxed);}
}
#[derive(Default)]
pub(super) struct SharedAllocation {
    base_bytes:AtomicUsize, cell_bytes:AtomicUsize, paths:AtomicUsize,
    counters:OnceLock<Arc<Counters>>,
}
impl SharedAllocation {
    pub(super) fn set_base_bytes(&self,bytes:usize){self.base_bytes.store(bytes,Ordering::Relaxed);}
    fn attach(&self,counters:Arc<Counters>){
        assert!(self.counters.set(counters.clone()).is_ok(),"table registered twice");
        counters.live_tables.fetch_add(1,Ordering::Relaxed);
        counters.live_base_bytes.fetch_add(self.base_bytes.load(Ordering::Relaxed),Ordering::Relaxed);
        counters.live_cell_bytes.fetch_add(self.cell_bytes.load(Ordering::Relaxed),Ordering::Relaxed);
        counters.peak();
    }
    pub(super) fn materialize(&self,cells:usize){
        let bytes=cells*std::mem::size_of::<usize>();
        self.cell_bytes.fetch_add(bytes,Ordering::Relaxed);self.paths.fetch_add(1,Ordering::Relaxed);
        if let Some(counters)=self.counters.get(){
            counters.live_cell_bytes.fetch_add(bytes,Ordering::Relaxed);
            counters.materialized_paths.fetch_add(1,Ordering::Relaxed);
            counters.materialized_bytes.fetch_add(bytes as u64,Ordering::Relaxed);counters.peak();
        }
    }
    fn bytes(&self)->usize{self.base_bytes.load(Ordering::Relaxed)+self.cell_bytes.load(Ordering::Relaxed)}
}
impl Drop for SharedAllocation {
    fn drop(&mut self){if let Some(counters)=self.counters.get(){
        counters.live_tables.fetch_sub(1,Ordering::Relaxed);
        counters.live_base_bytes.fetch_sub(self.base_bytes.load(Ordering::Relaxed),Ordering::Relaxed);
        counters.live_cell_bytes.fetch_sub(self.cell_bytes.load(Ordering::Relaxed),Ordering::Relaxed);
    }}
}
impl PreparedGraph {
    fn signature(&self)->u64{
        let mut hash=DefaultHasher::new();
        self.map.width.hash(&mut hash);self.map.height.hash(&mut hash);self.map.cells.hash(&mut hash);
        self.durations.hash(&mut hash);self.costs.hash(&mut hash);self.positions.hash(&mut hash);
        self.spot_at.hash(&mut hash);self.cell_node.hash(&mut hash);self.targets.hash(&mut hash);self.spot_positions.hash(&mut hash);
        hash.finish()
    }
}
enum State { Building, Ready(Arc<GraphTables>), Failed }
struct Entry {
    state:Mutex<State>, signal:Condvar, phase:AtomicU8,
    allocation:OnceLock<Arc<SharedAllocation>>,
}
impl Entry {
    fn new()->Self{Self{state:Mutex::new(State::Building),signal:Condvar::new(),phase:AtomicU8::new(BUILDING),allocation:OnceLock::new()}}
    fn bytes(&self)->usize{self.allocation.get().map_or(0,|allocation|allocation.bytes())}
}
struct Slot { signature:u64, key:Arc<PreparedGraph>, entry:Arc<Entry>, used:u64 }
#[derive(Default)]
struct Index { slots:Vec<Slot>, clock:u64 }
pub(super) struct GraphCache {
    index:Mutex<Index>, counters:Arc<Counters>, max_entries:usize, max_bytes:usize, max_wait:Duration,
    gpu:Option<Arc<GraphGpu>>,
}
impl Default for GraphCache {
    fn default()->Self{Self::with_limits(4,128*1024*1024,Duration::from_secs(2))}
}
enum Acquisition { Owner(Arc<Entry>), Existing(Arc<Entry>), Bypass }
struct BuildingGuard {
    entry:Option<Arc<Entry>>, counters:Arc<Counters>, began:Instant, finished:bool,
}
impl BuildingGuard {
    fn new(entry:Option<Arc<Entry>>,counters:Arc<Counters>)->Self{
        counters.builds.fetch_add(1,Ordering::Relaxed);Self{entry,counters,began:Instant::now(),finished:false}
    }
    fn finish(mut self,tables:Arc<GraphTables>)->Arc<GraphTables>{
        if let Some(entry)=&self.entry {
            assert!(entry.allocation.set(tables.allocation.clone()).is_ok());
            *lock(&entry.state)=State::Ready(tables.clone());
            entry.phase.store(READY,Ordering::Release);entry.signal.notify_all();
        }
        self.counters.completed.fetch_add(1,Ordering::Relaxed);self.finished=true;tables
    }
}
impl Drop for BuildingGuard {
    fn drop(&mut self){
        self.counters.build_ns.fetch_add(self.began.elapsed().as_nanos().min(u128::from(u64::MAX))as u64,Ordering::Relaxed);
        if !self.finished {
            self.counters.failures.fetch_add(1,Ordering::Relaxed);
            if let Some(entry)=&self.entry {
                *lock(&entry.state)=State::Failed;entry.phase.store(FAILED,Ordering::Release);entry.signal.notify_all();
            }
        }
    }
}
impl GraphCache {
    fn with_limits(max_entries:usize,max_bytes:usize,max_wait:Duration)->Self{
        assert!(max_entries>0);Self{index:Mutex::new(Index::default()),counters:Arc::new(Counters::default()),max_entries,max_bytes,max_wait,gpu:None}
    }
    pub(super) fn for_search_pool()->Self {
        let mut cache=Self::default();cache.gpu=GraphGpu::from_environment();cache
    }
    pub(super) fn shutdown_gpu(&self,deadline:Instant) {
        if let Some(gpu)=&self.gpu {gpu.shutdown(deadline);}
    }
    fn evict_one(index:&mut Index)->Option<Slot>{
        let oldest=index.slots.iter().enumerate().filter(|(_,slot)|slot.entry.phase.load(Ordering::Acquire)!=BUILDING).min_by_key(|(_,slot)|slot.used).map(|(i,_)|i)?;
        Some(index.slots.remove(oldest))
    }
    fn acquire(&self,key:Arc<PreparedGraph>,signature:u64)->Acquisition{
        let mut retired=Vec::new();
        let result={
            let mut index=lock(&self.index);index.clock=index.clock.saturating_add(1);let now=index.clock;
            if let Some(slot)=index.slots.iter_mut().find(|slot|slot.signature==signature&&*slot.key==*key){
                slot.used=now;self.counters.hits.fetch_add(1,Ordering::Relaxed);Acquisition::Existing(slot.entry.clone())
            }else{
                while index.slots.len()>=self.max_entries {
                    let Some(slot)=Self::evict_one(&mut index)else{break;};retired.push(slot);
                }
                if index.slots.len()>=self.max_entries{Acquisition::Bypass}else{
                    let entry=Arc::new(Entry::new());index.slots.push(Slot{signature,key,entry:entry.clone(),used:now});Acquisition::Owner(entry)
                }
            }
        };
        self.counters.evictions.fetch_add(retired.len()as u64,Ordering::Relaxed);drop(retired);result
    }
    fn trim(&self){
        let mut retired=Vec::new();{
            let mut index=lock(&self.index);
            while index.slots.iter().map(|slot|slot.entry.bytes()).sum::<usize>()>self.max_bytes {
                let Some(slot)=Self::evict_one(&mut index)else{break;};retired.push(slot);
            }
        }
        self.counters.evictions.fetch_add(retired.len()as u64,Ordering::Relaxed);drop(retired);
    }
    fn remove_failed(&self,entry:&Arc<Entry>){
        let retired={let mut index=lock(&self.index);index.slots.iter().position(|slot|Arc::ptr_eq(&slot.entry,entry)).map(|pos|index.slots.remove(pos))};
        if retired.is_some(){self.counters.evictions.fetch_add(1,Ordering::Relaxed);}drop(retired);
    }
    fn build_work(key:&PreparedGraph)->u64 {
        (key.map.len() as u64).saturating_mul(key.targets as u64).saturating_mul(super::PATH_MODES as u64).max(1)
    }
    fn allowed_wait(&self,key:&PreparedGraph,remaining:Duration)->Duration {
        // Keep room to rebuild privately if the owner fails or is descheduled.
        // First-use estimate is conservative; subsequent requests use the pool's
        // maximum measured ns per cell/target/mode, with 50% build headroom.
        let measured=self.counters.build_ns_per_work.load(Ordering::Relaxed);
        let ns_per_work=if measured==0{1000}else{measured};
        let estimate=Duration::from_nanos(Self::build_work(key).saturating_mul(ns_per_work));
        let reserve=estimate.saturating_add(estimate/2).saturating_add(Duration::from_millis(20));
        self.max_wait.min(remaining/4).min(remaining.saturating_sub(reserve))
    }
    pub(super) fn get_until(&self,key:PreparedGraph,deadline:Instant)->Arc<GraphTables>{
        let signature=key.signature();
        let max_wait=self.allowed_wait(&key,deadline.saturating_duration_since(Instant::now()));
        if max_wait<self.max_wait {self.counters.budget_limited.fetch_add(1,Ordering::Relaxed);}
        self.get_with_wait_backend(key,signature,max_wait,GraphTables::build,Some(deadline))
    }
    #[cfg(test)]
    fn get(&self,key:PreparedGraph)->Arc<GraphTables>{
        let signature=key.signature();self.get_with(key,signature,GraphTables::build)
    }
    fn build(&self,key:&PreparedGraph,entry:Option<Arc<Entry>>,builder:impl FnOnce(&PreparedGraph)->GraphTables,gpu_deadline:Option<Instant>)->Arc<GraphTables>{
        let owner=entry.is_some();
        let guard=BuildingGuard::new(entry,self.counters.clone());
        let gpu_tables=if owner {
            self.gpu.as_ref().zip(gpu_deadline).and_then(|(gpu,deadline)|{
                let measured=self.counters.build_ns_per_work.load(Ordering::Relaxed);
                let estimate=Duration::from_nanos(Self::build_work(key).saturating_mul(if measured==0{1000}else{measured}));
                gpu.try_build(key,deadline,estimate)
            })
        }else{None};
        let tables=Arc::new(match gpu_tables {
            Some(tables)=>{self.counters.gpu_builds.fetch_add(1,Ordering::Relaxed);tables}
            None=>{
                let cpu_began=Instant::now();let tables=builder(key);
                let elapsed=cpu_began.elapsed().as_nanos().min(u128::from(u64::MAX))as u64;
                // CPU-only high water mark: GPU success/wait never lowers fallback reserve.
                self.counters.build_ns_per_work.fetch_max(elapsed.div_ceil(Self::build_work(key)),Ordering::Relaxed);
                self.counters.cpu_builds.fetch_add(1,Ordering::Relaxed);self.counters.cpu_build_ns.fetch_add(elapsed,Ordering::Relaxed);
                tables
            }
        });
        tables.allocation.attach(self.counters.clone());
        guard.finish(tables)
    }
    #[cfg(test)]
    fn get_with(&self,key:PreparedGraph,signature:u64,builder:impl FnOnce(&PreparedGraph)->GraphTables)->Arc<GraphTables>{
        self.get_with_wait(key,signature,self.max_wait,builder)
    }
    fn get_with_wait(&self,key:PreparedGraph,signature:u64,max_wait:Duration,builder:impl FnOnce(&PreparedGraph)->GraphTables)->Arc<GraphTables>{
        self.get_with_wait_backend(key,signature,max_wait,builder,None)
    }
    fn get_with_wait_backend(&self,key:PreparedGraph,signature:u64,max_wait:Duration,builder:impl FnOnce(&PreparedGraph)->GraphTables,gpu_deadline:Option<Instant>)->Arc<GraphTables>{
        let wait_deadline=Instant::now()+max_wait;
        self.counters.requests.fetch_add(1,Ordering::Relaxed);
        let key=Arc::new(key);let mut builder=Some(builder);
        loop {
            match self.acquire(key.clone(),signature){
                Acquisition::Owner(entry)=>{
                    let tables=self.build(&key,Some(entry),builder.take().unwrap(),gpu_deadline);self.trim();return tables;
                }
                Acquisition::Bypass=>{
                    self.counters.bypasses.fetch_add(1,Ordering::Relaxed);return self.build(&key,None,builder.take().unwrap(),gpu_deadline);
                }
                Acquisition::Existing(entry)=>{
                    let mut state=lock(&entry.state);let began=Instant::now();let mut waited=false;
                    loop {match &*state {
                        State::Ready(tables)=>{
                            let tables=tables.clone();drop(state);
                            if waited{self.counters.wait_ns.fetch_add(began.elapsed().as_nanos()as u64,Ordering::Relaxed);}
                            self.trim();return tables;
                        }
                        State::Failed=>{
                            drop(state);if waited{self.counters.wait_ns.fetch_add(began.elapsed().as_nanos()as u64,Ordering::Relaxed);}
                            self.remove_failed(&entry);break;
                        }
                        State::Building=>{
                            if !waited{self.counters.waits.fetch_add(1,Ordering::Relaxed);waited=true;}
                            let left=wait_deadline.saturating_duration_since(Instant::now());
                            if left.is_zero(){
                                drop(state);self.counters.wait_ns.fetch_add(began.elapsed().as_nanos()as u64,Ordering::Relaxed);
                                self.counters.timeouts.fetch_add(1,Ordering::Relaxed);self.counters.bypasses.fetch_add(1,Ordering::Relaxed);
                                return self.build(&key,None,builder.take().unwrap(),gpu_deadline);
                            }
                            state=entry.signal.wait_timeout(state,left).unwrap_or_else(|poison|poison.into_inner()).0;
                        }
                    }}
                }
            }
        }
    }
    pub(super) fn snapshot(&self)->serde_json::Value{
        let index=lock(&self.index);let counters=&self.counters;
        serde_json::json!({"requests":counters.requests.load(Ordering::Relaxed),"hits":counters.hits.load(Ordering::Relaxed),
            "builds":counters.builds.load(Ordering::Relaxed),"completedBuilds":counters.completed.load(Ordering::Relaxed),"failures":counters.failures.load(Ordering::Relaxed),
            "waits":counters.waits.load(Ordering::Relaxed),"timeouts":counters.timeouts.load(Ordering::Relaxed),"bypasses":counters.bypasses.load(Ordering::Relaxed),"evictions":counters.evictions.load(Ordering::Relaxed),
            "buildMs":counters.build_ns.load(Ordering::Relaxed)as f64/1e6,"waitMs":counters.wait_ns.load(Ordering::Relaxed)as f64/1e6,
            "retainedEntries":index.slots.len(),"retainedTableBytesEstimate":index.slots.iter().map(|slot|slot.entry.bytes()).sum::<usize>(),
            "liveTables":counters.live_tables.load(Ordering::Relaxed),"liveBaseBytesEstimate":counters.live_base_bytes.load(Ordering::Relaxed),"liveCellBytes":counters.live_cell_bytes.load(Ordering::Relaxed),"peakTableBytesEstimate":counters.peak_bytes.load(Ordering::Relaxed),
            "materializedPaths":counters.materialized_paths.load(Ordering::Relaxed),"materializedBytes":counters.materialized_bytes.load(Ordering::Relaxed),
            "measuredMaxBuildNsPerWork":counters.build_ns_per_work.load(Ordering::Relaxed),"budgetLimitedRequests":counters.budget_limited.load(Ordering::Relaxed),
            "cpuBuilds":counters.cpu_builds.load(Ordering::Relaxed),"gpuBuilds":counters.gpu_builds.load(Ordering::Relaxed),
            "cpuBuildMs":counters.cpu_build_ns.load(Ordering::Relaxed)as f64/1e6,"gpu":self.gpu.as_ref().map(|gpu|gpu.snapshot()),
            "maxEntries":self.max_entries,"targetRetainedBytes":self.max_bytes,"maxWaitMs":self.max_wait.as_millis()})
    }
    pub(super) fn report(&self,context:&str){eprintln!("shared_graph context={context} {}",self.snapshot());}
}

#[cfg(test)]
#[path="graph_cache_tests.rs"]
mod tests;
