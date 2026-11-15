//! Concurrency and cache identity tests. No TCP, remote worker or timing score tests.
use super::*;
use super::super::{Graph, Path, Solver, SupplierTrace, MAX_STEPS, STOPS, PATH_MODES};
use std::sync::{Barrier, mpsc};
use std::thread;
use tester::{engine::Game, model::{Problem, Map, Spot}};

fn problem() -> Problem {
    Problem { starts_at:0, day_seconds:vec![60;3], day_steps:vec![40;3],
        map:Map { height:3,width:5,cells:vec![vec![0;5];3] },
        spots:vec![Spot {brand:0,pos:2,stocks:2},Spot {brand:1,pos:12,stocks:2}],
        agents:vec![0,4,10,14],fuel_limits:20,players:4,busy_threshold:2,jammed_threshold:5 }
}
fn prepared() -> PreparedGraph {
    let p=problem();let obs=Game::new(p.clone(),&[1,0,0,0]).observation(0);
    PreparedGraph::new(&p,&obs,&[])
}
fn stat(cache:&GraphCache,key:&str)->u64 {cache.snapshot()[key].as_u64().unwrap()}
fn await_stat(cache:&GraphCache,key:&str,minimum:u64) {
    let deadline=Instant::now()+Duration::from_secs(3);
    while stat(cache,key)<minimum { assert!(Instant::now()<deadline,"missing {key}={minimum}: {}",cache.snapshot());thread::yield_now(); }
}
fn path(tables:&GraphTables,mode:usize,source:usize,target:usize)->&Path {
    if mode==0 {&tables.paths[source][target]}else{&tables.alternatives[mode-1][source][target]}
}
fn equivalent(left:&GraphTables,right:&GraphTables) {
    assert_eq!(left.positions,right.positions);assert_eq!(left.targets,right.targets);
    assert_eq!(left.durations,right.durations);assert_eq!(left.costs,right.costs);
    assert_eq!(left.spot_at,right.spot_at);assert_eq!(left.cell_node,right.cell_node);
    assert_eq!(left.path_mode_mask,right.path_mode_mask);assert_eq!(left.potential,right.potential);
    assert_eq!(left.mobility,right.mobility);assert_eq!(left.fuel_to,right.fuel_to);
    for mode in 0..PATH_MODES {for source in 0..left.positions.len(){for target in 0..left.targets{
        let (a,b)=(path(left,mode,source,target),path(right,mode,source,target));
        assert_eq!((a.time,a.fuel,a.len(),a.cells()),(b.time,b.fuel,b.len(),b.cells()));
    }}}
}

#[test]
fn identical_workers_share_one_table_and_one_lazy_route() {
    fn send_sync<T:Send+Sync>(){} send_sync::<GraphTables>();send_sync::<GraphCache>();
    let cache=Arc::new(GraphCache::default());let workers=31;let barrier=Arc::new(Barrier::new(workers));
    let handles=(0..workers).map(|_|{
        let cache=cache.clone();let barrier=barrier.clone();let key=prepared();
        thread::spawn(move||{barrier.wait();let tables=cache.get(key);tables.paths[2][0].cells();tables})
    }).collect::<Vec<_>>();
    let tables=handles.into_iter().map(|h|h.join().unwrap()).collect::<Vec<_>>();
    for table in &tables { assert!(Arc::ptr_eq(&tables[0],table));assert_eq!(tables[0].paths[2][0].cells().as_ptr(),table.paths[2][0].cells().as_ptr()); }
    assert_eq!(stat(&cache,"requests"),31);assert_eq!(stat(&cache,"builds"),1);assert_eq!(stat(&cache,"hits"),30);
    assert_eq!(stat(&cache,"materializedPaths"),1);assert_eq!(stat(&cache,"materializedBytes"),(tables[0].paths[2][0].len()*std::mem::size_of::<usize>())as u64);
    equivalent(&tables[0],&GraphTables::build(&prepared()));
}

#[test]
fn full_key_equality_rejects_hash_collisions_and_preserves_position_order() {
    let cache=GraphCache::with_limits(32,usize::MAX,Duration::from_secs(2));let a=prepared();let original=cache.get_with(a.clone(),7,GraphTables::build);
    let mut changed=a.clone();changed.durations[0]+=1;
    let different=cache.get_with(changed,7,GraphTables::build);assert!(!Arc::ptr_eq(&original,&different));
    let mut changed=a.clone();changed.positions.swap(2,3);changed.cell_node.fill(0);
    let reordered=cache.get_with(changed,7,GraphTables::build);assert!(!Arc::ptr_eq(&original,&reordered));
    let mut changed=a.clone();changed.positions.push(changed.positions[0]);
    let duplicated=cache.get_with(changed,7,GraphTables::build);assert!(!Arc::ptr_eq(&original,&duplicated));
    for field in 0..5 {
        let mut changed=a.clone();
        match field {
            0=>changed.costs[0]+=1,
            1=>changed.map.cells[0][0]=1,
            2=>changed.spot_at[0]=0,
            3=>changed.targets-=1,
            _=>changed.spot_positions.reverse(),
        }
        let different=cache.get_with(changed,7,GraphTables::build);assert!(!Arc::ptr_eq(&original,&different),"field {field}");
    }
    assert!(Arc::ptr_eq(&original,&cache.get_with(a,7,GraphTables::build)));
    let p=problem();let mut obs=Game::new(p.clone(),&[1,0,0,0]).observation(0);
    let before=PreparedGraph::new(&p,&obs,&[]);obs.day=2;obs.agents[0].fuel=1;
    let after=PreparedGraph::new(&p,&obs,&[]);assert!(before==after,"day and fuel are private solver inputs, not graph inputs");
    obs.agents.swap(0,1);assert!(before!=PreparedGraph::new(&p,&obs,&[]));obs.agents.swap(0,1);
    assert!(PreparedGraph::new(&p,&obs,&[5,6])!=PreparedGraph::new(&p,&obs,&[6,5]),"anchor order affects node meaning");
    obs.traffics=vec![tester::model::Traffic{pos:1,status:0},tester::model::Traffic{pos:3,status:2}];
    let before=PreparedGraph::new(&p,&obs,&[]);obs.traffics.reverse();assert!(before==PreparedGraph::new(&p,&obs,&[]));
    obs.traffics[0].status=1;assert!(before!=PreparedGraph::new(&p,&obs,&[]));
}

#[test]
fn different_keys_build_without_holding_the_index_lock() {
    let cache=Arc::new(GraphCache::with_limits(1,usize::MAX,Duration::from_secs(2)));
    let (started_tx,started_rx)=mpsc::channel();let (release_tx,release_rx)=mpsc::channel();
    let owner_cache=cache.clone();let owner=thread::spawn(move||owner_cache.get_with(prepared(),0,|key|{
        started_tx.send(()).unwrap();release_rx.recv_timeout(Duration::from_secs(3)).unwrap();GraphTables::build(key)
    }));
    started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    let mut other=prepared();other.durations[0]+=1;
    let table=cache.get(other);assert_eq!(table.durations[0],3);assert_eq!(stat(&cache,"bypasses"),1);
    release_tx.send(()).unwrap();owner.join().unwrap();assert_eq!(stat(&cache,"completedBuilds"),2);
}

#[test]
fn owner_panic_notifies_waiters_and_a_waiter_rebuilds() {
    let cache=Arc::new(GraphCache::default());let (started_tx,started_rx)=mpsc::channel();let (release_tx,release_rx)=mpsc::channel();
    let c=cache.clone();let key=prepared();let signature=key.signature();
    let owner=thread::spawn(move||c.get_with(key,signature,|_|{
        started_tx.send(()).unwrap();release_rx.recv_timeout(Duration::from_secs(3)).unwrap();panic!("injected builder failure")
    }));
    started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    let c=cache.clone();let waiter=thread::spawn(move||c.get(prepared()));await_stat(&cache,"waits",1);
    release_tx.send(()).unwrap();assert!(owner.join().is_err());let recovered=waiter.join().unwrap();
    assert!(Arc::ptr_eq(&recovered,&cache.get(prepared())));assert_eq!(stat(&cache,"failures"),1);
    assert_eq!(stat(&cache,"completedBuilds"),1);assert_eq!(stat(&cache,"timeouts"),0);
    equivalent(&recovered,&GraphTables::build(&prepared()));
}

#[test]
fn slow_owner_allows_bounded_wait_fallback_without_replacing_owner() {
    let cache=Arc::new(GraphCache::with_limits(4,usize::MAX,Duration::from_millis(20)));
    let (started_tx,started_rx)=mpsc::channel();let (release_tx,release_rx)=mpsc::channel();
    let c=cache.clone();let key=prepared();let signature=key.signature();
    let owner=thread::spawn(move||c.get_with(key,signature,|key|{
        started_tx.send(()).unwrap();release_rx.recv_timeout(Duration::from_secs(3)).unwrap();GraphTables::build(key)
    }));
    started_rx.recv_timeout(Duration::from_secs(3)).unwrap();let fallback=cache.get(prepared());
    assert_eq!(stat(&cache,"timeouts"),1);assert_eq!(stat(&cache,"bypasses"),1);
    release_tx.send(()).unwrap();let original=owner.join().unwrap();assert!(!Arc::ptr_eq(&original,&fallback));
    assert!(Arc::ptr_eq(&original,&cache.get(prepared())));equivalent(&original,&fallback);
}

#[test]
fn lru_and_byte_eviction_preserve_active_tables_and_release_accounting() {
    let cache=GraphCache::with_limits(2,usize::MAX,Duration::from_secs(1));
    let a=prepared();let mut b=a.clone();b.durations[0]+=1;let mut c=a.clone();c.durations[0]+=2;
    let first=cache.get(a.clone());let second=cache.get(b.clone());
    assert!(Arc::ptr_eq(&first,&cache.get(a.clone())));let third=cache.get(c);
    assert_eq!(stat(&cache,"retainedEntries"),2);assert_eq!(stat(&cache,"liveTables"),3);
    assert!(!Arc::ptr_eq(&second,&cache.get(b)));assert_eq!(first.paths[2][0].cells().last(),Some(&first.positions[0]));
    drop((first,second,third));assert_eq!(stat(&cache,"liveTables"),2);
    let tiny=GraphCache::with_limits(4,1,Duration::from_secs(1));let active=tiny.get(a);
    assert_eq!(stat(&tiny,"retainedEntries"),0);assert_eq!(stat(&tiny,"liveTables"),1);
    active.paths[2][0].cells();assert!(stat(&tiny,"liveCellBytes")>0);drop(active);
    assert_eq!(stat(&tiny,"liveTables"),0);assert_eq!(stat(&tiny,"liveBaseBytesEstimate"),0);assert_eq!(stat(&tiny,"liveCellBytes"),0);
}

#[test]
fn mutable_worker_caches_and_snapshot_restore_remain_private() {
    let mut p=problem();p.map=Map{height:17,width:17,cells:vec![vec![0;17];17]};
    let obs=Game::new(p.clone(),&[1,0,0,0]).observation(0);let cache=Arc::new(GraphCache::default());
    let tables=cache.get(PreparedGraph::new(&p,&obs,&[]));let left=Graph::from_tables(tables.clone());let right=Graph::from_tables(tables);
    left.memo.borrow_mut().insert((1,2),(1,2,3,4,5,6.0,7));assert!(right.memo.borrow().is_empty());
    *left.supply_cache.borrow_mut()=Some(([0;3+STOPS*2+STOPS-1],std::rc::Rc::new(SupplierTrace{windows:[(0,0);STOPS],trace:[0;MAX_STEPS+1],endhub:0,value:0.0})));
    assert!(right.supply_cache.borrow().is_none());
    let target=left.targets;assert!(target<left.positions.len());
    let distance=left.fuel_distance(&p,0,target);assert!(!left.fuel_cache.borrow()[target].is_empty());assert!(right.fuel_cache.borrow()[target].is_empty());
    assert_eq!(distance,right.fuel_distance(&p,0,target));assert!(left.tables.fuel_to[target].is_empty());
    let mut solver=Solver::new(p,0,0.0,2.0,0.0);solver.graph_cache=Some(cache.clone());let state=solver.snapshot();solver.restore(state);
    assert!(Arc::ptr_eq(solver.graph_cache.as_ref().unwrap(),&cache));
}

#[test]
fn local_deadline_limits_wait_and_reserves_measured_private_build_cost() {
    let cache=GraphCache::default();let key=prepared();
    assert_eq!(cache.allowed_wait(&key,Duration::ZERO),Duration::ZERO);
    assert_eq!(cache.allowed_wait(&key,Duration::from_millis(10)),Duration::ZERO);
    assert!(cache.allowed_wait(&key,Duration::from_millis(100))<=Duration::from_millis(25));
    cache.counters.build_ns_per_work.store(100_000,Ordering::Relaxed);
    assert_eq!(cache.allowed_wait(&key,Duration::from_millis(100)),Duration::ZERO);
    let cache=Arc::new(GraphCache::default());let (started_tx,started_rx)=mpsc::channel();let (release_tx,release_rx)=mpsc::channel();
    let c=cache.clone();let key=prepared();let signature=key.signature();
    let owner=thread::spawn(move||c.get_with(key,signature,|key|{
        started_tx.send(()).unwrap();release_rx.recv_timeout(Duration::from_secs(3)).unwrap();GraphTables::build(key)
    }));
    started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    let fallback=cache.get_until(prepared(),Instant::now());
    assert_eq!(stat(&cache,"budgetLimitedRequests"),1);assert_eq!(stat(&cache,"timeouts"),1);
    assert!(cache.snapshot()["waitMs"].as_f64().unwrap()<100.0,"expired deadline must not wait for the two-second cap");
    release_tx.send(()).unwrap();equivalent(&fallback,&owner.join().unwrap());
}

#[test]
fn gpu_owner_publication_attaches_once_and_does_not_pollute_cpu_estimate() {
    use super::super::gpu_graph::{GraphGpu,MockBehavior};
    let mut cache=GraphCache::default();cache.gpu=Some(GraphGpu::testing(MockBehavior::Exact));
    cache.counters.build_ns_per_work.store(12345,Ordering::Relaxed);
    let key=prepared();let tables=cache.get_until(key.clone(),Instant::now()+Duration::from_secs(5));
    assert_eq!(stat(&cache,"builds"),1);assert_eq!(stat(&cache,"gpuBuilds"),1);assert_eq!(stat(&cache,"cpuBuilds"),0);
    assert_eq!(stat(&cache,"liveTables"),1);assert_eq!(stat(&cache,"measuredMaxBuildNsPerWork"),12345);
    assert!(Arc::ptr_eq(&tables,&cache.get(key.clone())));
    equivalent(&tables,&GraphTables::build(&key));
    assert_eq!(stat(&cache,"failures"),0);
}

#[test]
fn unavailable_gpu_and_wrong_reply_use_cpu_in_the_same_owner() {
    use super::super::gpu_graph::{GraphGpu,MockBehavior};
    for behavior in [MockBehavior::NotReady,MockBehavior::Error,MockBehavior::WrongInput] {
        let mut cache=GraphCache::default();cache.gpu=Some(GraphGpu::testing(behavior));
        let key=prepared();let tables=cache.get_until(key.clone(),Instant::now()+Duration::from_secs(5));
        assert_eq!(stat(&cache,"builds"),1);assert_eq!(stat(&cache,"cpuBuilds"),1);assert_eq!(stat(&cache,"gpuBuilds"),0);
        assert_eq!(stat(&cache,"completedBuilds"),1);assert_eq!(stat(&cache,"failures"),0);assert_eq!(stat(&cache,"liveTables"),1);
        assert!(stat(&cache,"measuredMaxBuildNsPerWork")>0);
        assert!(Arc::ptr_eq(&tables,&cache.get(key.clone())));equivalent(&tables,&GraphTables::build(&key));
    }
}

#[test]
fn private_bypass_never_dispatches_gpu() {
    use super::super::gpu_graph::{GraphGpu,MockBehavior};
    let mut cache=GraphCache::with_limits(1,usize::MAX,Duration::from_secs(2));
    let gpu=GraphGpu::testing(MockBehavior::Exact);cache.gpu=Some(gpu.clone());
    let occupied=prepared();let signature=occupied.signature();
    assert!(matches!(cache.acquire(Arc::new(occupied),signature),Acquisition::Owner(_)));
    let mut key=prepared();key.durations[0]+=1;
    let tables=cache.get_until(key.clone(),Instant::now()+Duration::from_secs(5));
    assert_eq!(stat(&cache,"bypasses"),1);assert_eq!(stat(&cache,"cpuBuilds"),1);assert_eq!(stat(&cache,"gpuBuilds"),0);
    assert_eq!(gpu.snapshot()["attempts"].as_u64(),Some(0));equivalent(&tables,&GraphTables::build(&key));
}

#[test]
fn gpu_owner_panic_notifies_failed_entry_and_allows_cpu_recovery() {
    use super::super::gpu_graph::{GraphGpu,MockBehavior};
    let mut cache=GraphCache::default();cache.gpu=Some(GraphGpu::testing(MockBehavior::Panic));
    let failed=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||cache.get_until(prepared(),Instant::now()+Duration::from_secs(5))));
    assert!(failed.is_err());assert_eq!(stat(&cache,"failures"),1);
    // Test-only get uses the ordinary CPU builder, so recovery does not retry the panic mock.
    let recovered=cache.get(prepared());assert_eq!(stat(&cache,"completedBuilds"),1);
    equivalent(&recovered,&GraphTables::build(&prepared()));
}
