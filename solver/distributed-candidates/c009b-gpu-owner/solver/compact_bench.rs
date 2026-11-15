use super::*;
use std::hint::black_box;

// Measurement-only copy of D0's Graph::new, independent of C007's tables.
// It preserves D0's allocations, shortest-path tie ordering, masks, fuel tables,
// potential and mobility work so construction time includes the same scope.
mod eager {
    use super::*;
    #[derive(Clone, Default)]
    pub(super) struct Path { pub(super) cells: Vec<usize>, pub(super) time: i32, pub(super) fuel: i32 }
    #[allow(dead_code)]
pub(super) struct Graph {
    pub(super) paths: Vec<Vec<Path>>, pub(super) positions: Vec<usize>, pub(super) alternatives: Vec<Vec<Vec<Path>>>, path_mode_mask:Vec<Vec<u8>>, durations:Vec<i32>, costs:Vec<i32>,
    spot_at: Vec<i32>, potential:Vec<Vec<(i32,i32)>>, mobility:Vec<i32>, fuel_to: RefCell<Vec<Vec<i32>>>, cell_node:Vec<usize>, pub(super) targets:usize, memo:RefCell<RouteMemo>, supply_cache:RefCell<Option<(SupplierKey,Rc<SupplierTrace>)>>,
}
impl Graph {
    pub(super) fn new(p: &Problem, obs: &Observation, extra:&[usize]) -> Self {
        let size=p.map.len();
        let mut durations=vec![2;size]; let mut costs=vec![1;size];
        for x in 0..size { match p.map.terrain(x) {1=>{durations[x]=1;costs[x]=2;},2=>{durations[x]=3;costs[x]=2;},_=>{}} }
        for t in &obs.traffics {durations[t.pos]=[1,2,4][t.status as usize];}
        let mut positions=p.spots.iter().map(|s|s.pos).collect::<Vec<_>>();
        positions.extend(obs.agents.iter().map(|a|a.pos));
        for &pos in extra {if !positions.contains(&pos){positions.push(pos);}}
        // A small spatial grid makes rendezvous possible between shops as well.
        for gy in 0..5 {for gx in 0..5 {
            let y0=gy*p.map.height/5;let y1=(gy+1)*p.map.height/5;
            let x0=gx*p.map.width/5;let x1=(gx+1)*p.map.width/5;
            let cy=(y0+y1)/2;let cx=(x0+x1)/2;
            let mut chosen=None;let mut best=i32::MAX;
            for y in y0..y1 {for x in x0..x1 {let cell=y*p.map.width+x;if p.map.terrain(cell)==3{continue;}
                let degree=p.map.neighbors(cell).filter(|&v|p.map.terrain(v)!=3).count()as i32;
                let score=3*(y.abs_diff(cy)+x.abs_diff(cx))as i32+(match p.map.terrain(cell){1=>1,2=>3,_=>2})*2+(6-degree)*3;
                if score<best{best=score;chosen=Some(cell);}
            }}
            if let Some(cell)=chosen {if !positions.contains(&cell){positions.push(cell);}}
        }}
        let mut spot_at=vec![-1;size]; for (i,s) in p.spots.iter().enumerate(){spot_at[s.pos]=i as i32;}
        // On larger maps, meet beside shops instead of snapping to a distant grid cell.
        // All six approach cells are useful because movement costs belong to the departure cell.
        if p.spots.len()>16 {
            let anchors=positions[..p.spots.len()+obs.agents.len()].to_vec();
            for anchor in anchors {
                for cell in p.map.neighbors(anchor) {
                    if p.map.terrain(cell)!=3&&!positions.contains(&cell){positions.push(cell);}
                }
            }
        }
        let sparse_targets=positions.len();
        let mut cell_node=vec![0;size];
        for pos in 0..size{if p.map.terrain(pos)!=3{cell_node[pos]=positions.iter().position(|&x|x==pos).unwrap_or_else(||{positions.push(pos);positions.len()-1});}}
        let sources=positions.len();
        // On small boards, keep an exact rendezvous cell instead of snapping it to the coarse grid.
        let targets=if p.spots.len()<=16&&size<=256{sources}else{sparse_targets};
        let mut variants=Vec::new();
        let mut fuel_to=vec![vec![];sources];
        for mode in 0..PATH_MODES {
            let mut paths=vec![vec![Path::default();targets];sources];
            for j in 0..targets {
                let target=positions[j];let mut d=vec![i32::MAX;size];let mut next=vec![0;size];
                let mut heap=BinaryHeap::new();d[target]=0;heap.push((Reverse(0),target));
                while let Some((Reverse(cost),x))=heap.pop(){if d[x]!=cost{continue;}
                    for dir in 0..6{if let Some(y)=p.map.neighbor(x,dir){if p.map.terrain(y)==3{continue;}
                        // Optional detours can collect intermediate shops, beyond equal-time ties.
                        // Keep every edge positive so shortest paths cannot contain reward cycles.
                        let reward=if spot_at[y]>=0{[0,0,0,20,80,160][mode]}else{0};
                        let edge=(durations[y]*[100,100,10,100,100,100][mode]+costs[y]*[3,100,100,3,3,3][mode]-reward).max(1);
                        let nd=cost+edge;
                        if nd<d[y]{d[y]=nd;next[y]=x;heap.push((Reverse(nd),y));}
                    }}
                }
                for (i,&source)in positions.iter().enumerate(){
                    let mut x=source;let mut path=Path::default();
                    while x!=target{let y=next[x];path.cells.push(y);path.time+=durations[x];path.fuel+=costs[x];x=y;}
                    paths[i][j]=path;
                }
                if mode==0 {
                    fuel_to[j]=vec![99999;size];
                    let mut heap=BinaryHeap::new();fuel_to[j][target]=0;heap.push((Reverse(0),target));
                    while let Some((Reverse(cost),x))=heap.pop(){if fuel_to[j][x]!=cost{continue;}
                        for y in p.map.neighbors(x){if p.map.terrain(y)==3{continue;}let nd=cost+costs[y];
                            if nd<fuel_to[j][y]{fuel_to[j][y]=nd;heap.push((Reverse(nd),y));}
                        }
                    }
                }
            }
            variants.push(paths);
        }
        let paths=variants.remove(0);
        let mut path_mode_mask=vec![vec![1u8;targets];sources];
        for source in 0..sources{for target in 0..targets{for mode in 1..PATH_MODES{
            let cells=&variants[mode-1][source][target].cells;
            let duplicate=(0..mode).any(|other|path_mode_mask[source][target]&(1<<other)!=0&&
                cells.as_slice()==if other==0{paths[source][target].cells.as_slice()}else{variants[other-1][source][target].cells.as_slice()});
            if !duplicate{path_mode_mask[source][target]|=1<<mode;}
        }}}
        let potential=paths.iter().map(|row|row[..p.spots.len()].iter().map(|path|(path.time,path.fuel)).collect()).collect();
        let mobility=positions.iter().map(|&pos|(0..p.spots.len()).filter(|&i|p.spots[i].pos!=pos).map(|i|fuel_to[i][pos]).min().unwrap_or(0)).collect();
        Self{paths,positions,alternatives:variants,path_mode_mask,durations,costs,spot_at,potential,mobility,fuel_to:RefCell::new(fuel_to),cell_node,targets,memo:RefCell::new(RouteMemo::default()),supply_cache:RefCell::new(None)}
    }
}
}

trait BenchGraph {
    fn cost(&self, mode:usize, source:usize, target:usize)->u64;
    fn visits(&self, mode:usize, source:usize, target:usize)->u64;
    fn path_storage(&self)->(usize,usize,usize,usize);
}
fn visit_checksum(cells:&[usize])->u64{
    cells.iter().fold(0u64,|value,&cell|value.rotate_left(7).wrapping_add(cell as u64+1))
}
impl BenchGraph for Graph {
    fn cost(&self,mode:usize,source:usize,target:usize)->u64{
        let path=path_at(self,mode,source,target);((path.time as u64)<<32)|(path.fuel as u64)
    }
    fn visits(&self,mode:usize,source:usize,target:usize)->u64{
        visit_checksum(path_at(self,mode,source,target).cells())
    }
    fn path_storage(&self)->(usize,usize,usize,usize){
        let mut bytes=0;let mut expanded=0;let mut visit_bytes=0;
        for mode in 0..PATH_MODES {
            for source in 0..self.positions.len(){for target in 0..self.targets{
                let path=path_at(self,mode,source,target);bytes+=std::mem::size_of::<Path>();
                if let Some(cells)=path.cells.get(){visit_bytes+=cells.len()*std::mem::size_of::<usize>();expanded+=1;}
            }}
            for target in 0..self.targets {
                let tree=path_at(self,mode,0,target).tree.as_ref().unwrap();
                bytes+=std::mem::size_of::<RouteTree>()+tree.next.len()*std::mem::size_of::<u16>()+tree.fingerprints.len()*std::mem::size_of::<u64>();
            }
        }
        (bytes,bytes+visit_bytes,0,expanded)
    }
}
impl BenchGraph for eager::Graph {
    fn cost(&self,mode:usize,source:usize,target:usize)->u64{
        let path=if mode==0{&self.paths[source][target]}else{&self.alternatives[mode-1][source][target]};
        ((path.time as u64)<<32)|(path.fuel as u64)
    }
    fn visits(&self,mode:usize,source:usize,target:usize)->u64{
        let path=if mode==0{&self.paths[source][target]}else{&self.alternatives[mode-1][source][target]};visit_checksum(&path.cells)
    }
    fn path_storage(&self)->(usize,usize,usize,usize){
        let mut bytes=0;
        for mode in 0..PATH_MODES {for source in 0..self.positions.len(){for target in 0..self.targets{
            let path=if mode==0{&self.paths[source][target]}else{&self.alternatives[mode-1][source][target]};
            bytes+=std::mem::size_of::<eager::Path>()+path.cells.capacity()*std::mem::size_of::<usize>();
        }}}
        (bytes,bytes,PATH_MODES*self.positions.len()*self.targets,PATH_MODES*self.positions.len()*self.targets)
    }
}
#[derive(Clone,Copy)]
struct Query{mode:usize,source:usize,target:usize}
fn queries(sources:usize,targets:usize)->Vec<Query>{
    let mut state=0x47ac56925d497311u64;
    let mut next=||{state=state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);(state>>24)as usize};
    (0..4096).map(|index|Query{
        mode:if index%4!=0{0}else{1+next()%(PATH_MODES-1)},
        source:next()%if index%4!=0{targets}else{sources},
        target:next()%targets,
    }).collect()
}
fn query_phase(graph:&impl BenchGraph,queries:&[Query],visits:bool)->(f64,u64){
    let began=Instant::now();let mut checksum=0u64;
    for &query in black_box(queries){
        let graph=black_box(graph);
        let value=if visits{graph.visits(query.mode,query.source,query.target)}else{graph.cost(query.mode,query.source,query.target)};
        checksum=checksum.rotate_left(11).wrapping_add(black_box(value));
    }
    (began.elapsed().as_secs_f64()*1000.0,black_box(checksum))
}
fn measure<G:BenchGraph>(build:impl FnOnce()->G,queries:&[Query],variant:&str)->serde_json::Value{
    let began=Instant::now();let graph=black_box(build());let build_ms=began.elapsed().as_secs_f64()*1000.0;
    let (metadata_ms,metadata_checksum)=query_phase(&graph,queries,false);
    let (cold_ms,cold_checksum)=query_phase(&graph,queries,true);
    let (hot_ms,hot_checksum)=query_phase(&graph,queries,true);
    assert_eq!(cold_checksum,hot_checksum);
    // Inspect storage after all timed queries: a full path-table scan would
    // otherwise warm caches between construction and the cold query phase.
    let storage=graph.path_storage();
    serde_json::json!({"variant":variant,"buildMs":build_ms,"metadataQueryMs":metadata_ms,"coldRouteQueryMs":cold_ms,"hotRouteQueryMs":hot_ms,"totalMeasuredMs":build_ms+metadata_ms+cold_ms+hot_ms,
        "metadataChecksum":metadata_checksum,"routeChecksum":cold_checksum,"pathStorageEstimateBeforeBytes":storage.0,"pathStorageEstimateAfterBytes":storage.1,"materializedPathsBefore":storage.2,"materializedPathsAfter":storage.3,"storageEstimatesCollectedAfterTiming":true})
}

#[test]
#[ignore = "finite performance microbench; pin to CPU 1 and run explicitly"]
fn compact_construction_and_queries_microbench(){
    let affinity=std::fs::read_to_string("/proc/self/status").unwrap().lines().find(|line|line.starts_with("Cpus_allowed_list:")).unwrap().to_owned();
    assert_eq!(affinity.split(':').nth(1).unwrap().trim(),"1","microbench must use CPU 1 only");
    for seed in 0..3 {
        let p=tester::generator::generate_for_players(seed,None,4).unwrap().problem;
        let mut kinds=vec![0;p.agents.len()];kinds[0]=1;let obs=Game::new(p.clone(),&kinds).observation(0);
        // Derive the query domain once; this setup is outside measured runs and
        // retained neither as a cache nor as a graph during measurements.
        let shape=Graph::new(&p,&obs,&[]);let sources=shape.positions.len();let targets=shape.targets;drop(shape);
        let queries=queries(sources,targets);
        for repeat in 0..3 {
            let (eager,compact,order)=if (seed+repeat)%2==0 {
                let eager=measure(||eager::Graph::new(&p,&obs,&[]),&queries,"D0-eager");
                let compact=measure(||Graph::new(&p,&obs,&[]),&queries,"C007-compact");(eager,compact,"eager-compact")
            }else{
                let compact=measure(||Graph::new(&p,&obs,&[]),&queries,"C007-compact");
                let eager=measure(||eager::Graph::new(&p,&obs,&[]),&queries,"D0-eager");(eager,compact,"compact-eager")
            };
            assert_eq!(eager["metadataChecksum"],compact["metadataChecksum"]);
            assert_eq!(eager["routeChecksum"],compact["routeChecksum"]);
            assert_eq!(compact["materializedPathsBefore"],0);
            eprintln!("compact_microbench {}",serde_json::json!({"seed":seed,"repeat":repeat,"order":order,"sources":sources,"targets":targets,"queriesPerPhase":queries.len(),"affinity":affinity,"eager":eager,"compact":compact}));
        }
    }
}
