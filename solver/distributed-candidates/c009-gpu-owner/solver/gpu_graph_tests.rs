//! Adapter and budget correctness without loading OpenCL or using a GPU.
use super::*;
use super::super::{Graph, PATH_MODES};
use tester::{engine::Game,model::{Problem,Map,Spot,Traffic}};

fn problem(side:usize)->Problem {
    let mut cells=vec![vec![0;side];side];
    for (y,row) in cells.iter_mut().enumerate(){for (x,cell) in row.iter_mut().enumerate(){*cell=((x+2*y)%3)as u8;}}
    cells[1][1]=3;
    Problem{starts_at:0,day_seconds:vec![60;3],day_steps:vec![40;3],map:Map{width:side,height:side,cells},
        spots:vec![Spot{brand:0,pos:2,stocks:2},Spot{brand:1,pos:side*side-2,stocks:2}],
        agents:vec![2,0,side-1,side*side-1],fuel_limits:20,players:4,busy_threshold:2,jammed_threshold:5}
}
fn path(t:&GraphTables,mode:usize,source:usize,target:usize)->&Path {
    if mode==0{&t.paths[source][target]}else{&t.alternatives[mode-1][source][target]}
}
fn same(a:&GraphTables,b:&GraphTables)->usize {
    assert_eq!(a.positions,b.positions);assert_eq!(a.targets,b.targets);
    assert_eq!(a.durations,b.durations);assert_eq!(a.costs,b.costs);assert_eq!(a.spot_at,b.spot_at);assert_eq!(a.cell_node,b.cell_node);
    assert_eq!(a.path_mode_mask,b.path_mode_mask);assert_eq!(a.potential,b.potential);assert_eq!(a.mobility,b.mobility);assert_eq!(a.fuel_to,b.fuel_to);
    let mut checked=0;
    for mode in 0..PATH_MODES{for source in 0..a.positions.len(){for target in 0..a.targets {
        let (x,y)=(path(a,mode,source,target),path(b,mode,source,target));
        assert_eq!((x.len(),x.time,x.fuel,x.cells()),(y.len(),y.time,y.fuel,y.cells()));
        let cell=usize::from(x.source);
        assert_eq!(x.tree.as_ref().unwrap().fingerprints[cell],y.tree.as_ref().unwrap().fingerprints[cell]);checked+=1;
    }}}
    checked
}

#[test]
fn gpu_adapter_matches_cpu_with_duplicate_positions_traffic_and_sparse_fuel_cache() {
    let mut checked=0;
    for side in [5,17] {
        let p=problem(side);let mut obs=Game::new(p.clone(),&[1,0,0,0]).observation(0);
        obs.traffics=vec![Traffic{pos:0,status:2},Traffic{pos:side+2,status:0}];
        let key=PreparedGraph::new(&p,&obs,&[side+2,side+3]);
        assert_eq!(key.positions[0],key.positions[2]);
        let input=prepare(&key).unwrap();let verified=gpu_tables::cpu_reference(&input).unwrap().verify(input.clone()).unwrap();
        let gpu=GraphTables::from_gpu(&key,&input,verified,Instant::now()+Duration::from_secs(30)).unwrap();
        let cpu=GraphTables::build(&key);checked+=same(&cpu,&gpu);
        assert_eq!(gpu.fuel_to[0][side+1],99999,"GPU INF maps to CPU sentinel");
        if side==17 {
            assert!(gpu.targets<gpu.positions.len());assert!(gpu.fuel_to[gpu.targets].is_empty());
            let target=gpu.targets;let left=Graph::from_tables(Arc::new(gpu));let right=Graph::from_tables(Arc::new(cpu));
            assert_eq!(left.fuel_distance(&p,0,target),right.fuel_distance(&p,0,target));
        }
    }
    assert!(checked>10_000);
    eprintln!("C009 CPU-only adapter path checks={checked}");
}

#[test]
fn adapter_rejects_wrong_input_and_expired_deadline() {
    let p=problem(5);let obs=Game::new(p.clone(),&[1,0,0,0]).observation(0);let key=PreparedGraph::new(&p,&obs,&[]);
    let input=prepare(&key).unwrap();let raw=gpu_tables::cpu_reference(&input).unwrap();
    assert!(GraphTables::from_gpu(&key,&input,raw.clone().verify(input.clone()).unwrap(),Instant::now()).is_err());
    let mut different=(*input).clone();different.targets.reverse();
    let different=Arc::new(different);let verified=gpu_tables::cpu_reference(&different).unwrap().verify(different).unwrap();
    assert!(GraphTables::from_gpu(&key,&input,verified,Instant::now()+Duration::from_secs(2)).is_err());
    assert!(raw.verify(input).unwrap().scalars_until(Some(Instant::now())).is_err());
}

#[test]
fn gpu_adapter_matches_full_generated_format_tables_without_driver() {
    let mut checked=0usize;
    for seed in 0..3 {
        let p=tester::generator::generate_for_players(seed,None,4).unwrap().problem;
        let mut kinds=vec![0;p.agents.len()];kinds[0]=1;
        let game=Game::new(p.clone(),&kinds);let mut obs=game.observation(0);
        for (i,traffic) in obs.traffics.iter_mut().enumerate(){traffic.status=((i+seed as usize)%3)as u8;}
        let key=PreparedGraph::new(&p,&obs,&[]);let input=prepare(&key).unwrap();
        let verified=gpu_tables::cpu_reference(&input).unwrap().verify(input.clone()).unwrap();
        let gpu=GraphTables::from_gpu(&key,&input,verified,Instant::now()+Duration::from_secs(30)).unwrap();
        let cpu=GraphTables::build(&key);checked+=same(&gpu,&cpu);
        eprintln!("C009 adapter seed={seed} cells={} sources={} targets={}",key.map.len(),key.positions.len(),key.targets);
    }
    assert!(checked>1_000_000);eprintln!("C009 generated full adapter route checks={checked}");
}

#[test]
fn local_gpu_budget_preserves_cpu_reserve_and_rejects_tiny_slack() {
    let policy=Policy::default();
    assert_eq!(policy.budget(Duration::from_secs(10),Duration::from_millis(100)),Duration::from_millis(250));
    assert_eq!(policy.budget(Duration::from_millis(100),Duration::from_millis(100)),Duration::ZERO);
    assert_eq!(policy.budget(Duration::from_millis(160),Duration::ZERO),Duration::from_millis(20));
    let p=problem(5);let obs=Game::new(p.clone(),&[1,0,0,0]).observation(0);let key=PreparedGraph::new(&p,&obs,&[]);
    let service=GraphGpu::testing(MockBehavior::Exact);
    assert!(service.try_build(&key,Instant::now(),Duration::ZERO).is_none());
    assert_eq!(service.counters.attempts.load(Ordering::Relaxed),0);
}

#[test]
fn wrong_and_late_gpu_replies_disable_gpu_without_returning_partial_tables() {
    let p=problem(5);let obs=Game::new(p.clone(),&[1,0,0,0]).observation(0);let key=PreparedGraph::new(&p,&obs,&[]);
    for behavior in [MockBehavior::WrongInput,MockBehavior::Late] {
        let service=GraphGpu::testing(behavior);
        assert!(service.try_build(&key,Instant::now()+Duration::from_millis(160),Duration::ZERO).is_none());
        assert!(service.disabled.load(Ordering::Acquire));
        assert_eq!(service.counters.completed.load(Ordering::Relaxed),0);
    }
}
