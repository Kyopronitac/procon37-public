//! Compare the compact representation against D0's eager path reconstruction.
//! This deliberately keeps a separate implementation of the original tree walk.
use super::*;

fn reference_tree(p: &Problem, obs: &Observation, target: usize, mode: usize) -> (Vec<usize>, Vec<i32>, Vec<i32>) {
    let size=p.map.len();
    let mut durations=vec![2;size];let mut costs=vec![1;size];
    for x in 0..size {match p.map.terrain(x){1=>{durations[x]=1;costs[x]=2;},2=>{durations[x]=3;costs[x]=2;},_=>{}}}
    for traffic in &obs.traffics {durations[traffic.pos]=[1,2,4][traffic.status as usize];}
    let mut spot_at=vec![false;size];for spot in &p.spots {spot_at[spot.pos]=true;}
    let mut distance=vec![i32::MAX;size];let mut next=vec![0usize;size];
    let mut heap=BinaryHeap::new();distance[target]=0;heap.push((Reverse(0),target));
    while let Some((Reverse(cost),x))=heap.pop(){
        if distance[x]!=cost {continue;}
        for dir in 0..6 {if let Some(y)=p.map.neighbor(x,dir){
            if p.map.terrain(y)==3 {continue;}
            let reward=if spot_at[y]{[0,0,0,20,80,160][mode]}else{0};
            let edge=(durations[y]*[100,100,10,100,100,100][mode]+costs[y]*[3,100,100,3,3,3][mode]-reward).max(1);
            let candidate=cost+edge;
            if candidate<distance[y]{distance[y]=candidate;next[y]=x;heap.push((Reverse(candidate),y));}
        }}
    }
    (next,durations,costs)
}
fn path_at(g:&Graph,mode:usize,source:usize,target:usize)->&Path{
    if mode==0 {&g.paths[source][target]}else{&g.alternatives[mode-1][source][target]}
}
fn cached_paths(g:&Graph)->usize{
    (0..PATH_MODES).map(|mode|(0..g.positions.len()).map(|source|(0..g.targets).filter(|&target|path_at(g,mode,source,target).cells.get().is_some()).count()).sum::<usize>()).sum()
}

#[test]
fn compact_paths_match_d0_eager_routes_costs_and_mode_masks() {
    let mut checked=0usize;
    for seed in 0..3 {
        let p=tester::generator::generate_for_players(seed,None,4).unwrap().problem;
        let mut kinds=vec![0;p.agents.len()];kinds[0]=1;
        let mut game=Game::new(p.clone(),&kinds);
        if seed%2==1 {game.play_day(game.waiting_plan()).unwrap();}
        let mut obs=game.observation(0);
        // Include every observed congestion state and departure terrain type.
        for (i,traffic) in obs.traffics.iter_mut().enumerate(){traffic.status=((i+seed as usize)%3)as u8;}
        for agent in &mut obs.agents {
            if let Some(pos)=p.map.neighbors(agent.pos).find(|&pos|p.map.terrain(pos)!=3){agent.pos=pos;}
        }
        let extra=(0..p.map.len()).filter(|&pos|p.map.terrain(pos)!=3).step_by(37).take(5).collect::<Vec<_>>();
        let g=Graph::new(&p,&obs,&extra);
        assert_eq!(cached_paths(&g),0,"graph and mode-mask construction must not expand paths");
        for mode in 0..PATH_MODES {
            for target in 0..g.targets {
                let target_cell=g.positions[target];
                let (next,durations,costs)=reference_tree(&p,&obs,target_cell,mode);
                for (source,&source_cell) in g.positions.iter().enumerate(){
                    let mut expected=Vec::new();let mut time=0;let mut fuel=0;let mut cell=source_cell;
                    while cell!=target_cell {
                        time+=durations[cell];fuel+=costs[cell];cell=next[cell];expected.push(cell);
                        assert!(expected.len()<=p.map.len(),"reference path must not cycle");
                    }
                    let actual=path_at(&g,mode,source,target);
                    assert_eq!(actual.len(),expected.len(),"seed={seed} mode={mode} source={source} target={target}");
                    assert_eq!((actual.time,actual.fuel),(time,fuel),"seed={seed} mode={mode} source={source} target={target}");
                    assert_eq!(actual.cells(),expected.as_slice(),"seed={seed} mode={mode} source={source} target={target}");
                    let collected=|cells:&[usize]|cells.iter().filter_map(|&pos|p.spots.iter().position(|spot|spot.pos==pos)).collect::<Vec<_>>();
                    assert_eq!(collected(actual.cells()),collected(&expected));
                    assert_eq!(actual.cells().as_ptr(),actual.cells().as_ptr(),"subsequent access must reuse its visit list");
                    checked+=1;
                }
            }
        }
        for source in 0..g.positions.len(){for target in 0..g.targets{
            let mut expected_mask=1u8;
            for mode in 1..PATH_MODES {
                let cells=path_at(&g,mode,source,target).cells();
                let duplicate=(0..mode).any(|other|expected_mask&(1<<other)!=0&&cells==path_at(&g,other,source,target).cells());
                if !duplicate {expected_mask|=1<<mode;}
            }
            assert_eq!(g.path_mode_mask[source][target],expected_mask,"seed={seed} source={source} target={target}");
        }}
        eprintln!("compact equivalence seed={seed} sources={} targets={} paths={}",g.positions.len(),g.targets,PATH_MODES*g.positions.len()*g.targets);
    }
    assert!(checked>1_000_000);
    eprintln!("compact exact route comparisons={checked}");
}

#[test]
fn compact_path_cache_is_lazy_and_exact_comparison_survives_hash_collision(){
    let tree=|next:Vec<u16>|Arc::new(RouteTree{target:3,next:next.into_boxed_slice(),fingerprints:vec![0;4].into_boxed_slice()});
    let left=Path{tree:Some(tree(vec![1,3,3,3])),source:0,length:2,time:2,fuel:2,cells:OnceCell::new()};
    let right=Path{tree:Some(tree(vec![2,3,3,3])),source:0,length:2,time:2,fuel:2,cells:OnceCell::new()};
    assert!(!left.same_route(&right),"equal fingerprints cannot merge different visit lists");
    assert!(left.same_route(&left));
    assert!(left.cells.get().is_none()&&right.cells.get().is_none(),"comparisons must stay compact");
    assert_eq!(left.cells(),&[1,3]);
    assert_eq!(right.cells(),&[2,3]);
    let pointer=left.cells().as_ptr();assert_eq!(pointer,left.cells().as_ptr());
    let empty=Path{tree:left.tree.clone(),source:3,length:0,time:0,fuel:0,cells:OnceCell::new()};
    assert!(empty.is_empty());assert!(empty.cells().is_empty());assert!(empty.same_route(&Path::default()));
}

#[path = "compact_bench.rs"]
mod benchmark;
