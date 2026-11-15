use std::cmp::Reverse;
use std::collections::{BinaryHeap,HashMap};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::hash::{BuildHasherDefault, Hasher};
use std::time::{Duration, Instant};
use tester::model::{Problem, Observation, Plan, Event, Traffic};
use tester::engine::Game;
const STOPS:usize=4;
const PATH_MODES:usize=6;
const MAX_AGENTS:usize=8;
const MAX_STEPS:usize=256;

// Memo keys are already two independently mixed 64-bit route fingerprints.
// HashMap still compares both words, so bucket collisions never alias evaluations.
#[derive(Default)]
struct MemoHasher(u64);
impl Hasher for MemoHasher {
    fn finish(&self)->u64{self.0}
    fn write_u64(&mut self,value:u64){self.0=self.0.rotate_left(27)^value.wrapping_mul(0x9e3779b185ebca87);}
    fn write(&mut self,bytes:&[u8]){for &byte in bytes{self.write_u64(u64::from(byte));}}
}
type RouteMemo=HashMap<(u64,u64),(u32,i32,usize,usize,i32,f64,i32),BuildHasherDefault<MemoHasher>>;

#[derive(Clone, Default)]
struct Path { cells: Vec<usize>, time: i32, fuel: i32 }
#[derive(Clone,Copy)]
struct CarPrefix {mask:u32,node:usize,time:i32,fuel:i32,penalty:f64,complete:bool}
struct SupplierTrace { windows:[(i32,i32);STOPS], trace:[usize;MAX_STEPS+1], endhub:usize, value:f64 }
type SupplierKey=[usize;3+STOPS*2+STOPS-1];
struct Graph {
    paths: Vec<Vec<Path>>, positions: Vec<usize>, alternatives: Vec<Vec<Vec<Path>>>, path_mode_mask:Vec<Vec<u8>>, durations:Vec<i32>, costs:Vec<i32>,
    spot_at: Vec<i32>, potential:Vec<Vec<(i32,i32)>>, mobility:Vec<i32>, fuel_to: RefCell<Vec<Vec<i32>>>, cell_node:Vec<usize>, targets:usize, memo:RefCell<RouteMemo>, supply_cache:RefCell<Option<(SupplierKey,Rc<SupplierTrace>)>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tester::model::{Map, Spot};

    fn rejected_selection_problem()->Problem{
        Problem{starts_at:0,day_seconds:vec![60,60,60],day_steps:vec![40,40,40],
            map:Map{height:3,width:5,cells:vec![vec![0;5];3]},
            spots:vec![Spot{brand:0,pos:2,stocks:2},Spot{brand:1,pos:12,stocks:2}],
            agents:vec![0,4,10,14],fuel_limits:20,players:5,busy_threshold:2,jammed_threshold:5}
    }

    #[test]
    fn rejected_selection_rebinds_roles_and_invalidates_warm_start(){
        let p=rejected_selection_problem();let mut solver=Solver::new(p.clone(),0,0.0,2.0,0.0);
        let old=Game::new(p.clone(),&[1,0,0,0]);let obs=old.observation(0);
        let graph=Graph::new(&p,&obs,&[]);let warm=solver.initial(&graph,&obs,40,None);
        solver.cache=Some([warm.clone(),warm]);solver.cache_positions=graph.positions.clone();
        solver.selection_graph=Some(Rc::new(graph));solver.game=Some(old);
        let actual=Game::new(p,&[0,0,1,0]).observation(0);
        let plan=solver.plan(&actual,40,1,false);
        assert_eq!(solver.supplier,2);assert!(solver.cache.is_none());assert!(solver.selection_graph.is_none());
        assert_eq!(solver.game.as_ref().unwrap().agents,actual.agents);
        assert_eq!(plan,vec![vec![-40];4]);
    }

    #[test]
    fn rejected_selection_with_no_supplier_respects_observed_fuel(){
        let p=rejected_selection_problem();let mut solver=Solver::new(p.clone(),0,0.0,2.0,0.0);
        solver.game=Some(Game::new(p.clone(),&[1,0,0,0]));
        let mut actual=Game::new(p,&[0,0,0,0]);actual.agents[0].fuel=1;
        let plan=solver.plan(&actual.observation(0),40,1000,false);
        assert_eq!(solver.supplier,usize::MAX);assert!(solver.cache.is_none());
        actual.play_day(plan).unwrap();
        assert!(actual.score.udon>0);assert_eq!(actual.agents[0].pos,0);assert_eq!(actual.agents[0].fuel,1);
        let next=solver.plan(&actual.observation(0),40,1000,true);
        actual.play_day(next).unwrap();assert_eq!(actual.agents[0].fuel,1);
    }

    #[test]
    fn changed_supplier_generates_a_valid_plan_for_actual_roles(){
        let p=rejected_selection_problem();let mut solver=Solver::new(p.clone(),0,0.0,2.0,0.0);
        solver.game=Some(Game::new(p.clone(),&[1,0,0,0]));
        let mut actual=Game::new(p,&[0,0,1,0]);
        let plan=solver.plan(&actual.observation(0),40,300,false);
        assert_eq!(solver.supplier,2);actual.play_day(plan).unwrap();assert!(actual.score.udon>0);
    }

    #[test]
    fn tiny_catch_up_requests_synchronize_without_search(){
        let p=tester::generator::generate(2,None).unwrap().problem;
        let mut solver=Solver::new(p.clone(),0,0.0,2.0,0.0);let kinds=solver.select(1);
        let mut actual=Game::new(p,&kinds);let rng=solver.rng.0;
        let first=solver.plan(&actual.observation(0),actual.problem.day_steps[0],1,false);
        actual.play_day(first).unwrap();
        let second=solver.plan(&actual.observation(0),actual.problem.day_steps[1],1,true);
        assert_eq!(solver.game.as_ref().unwrap().day,actual.day);
        assert_eq!(solver.game.as_ref().unwrap().agents,actual.agents);
        assert_eq!(second,actual.waiting_plan());assert_eq!(solver.rng.0,rng);
        assert!(solver.cache.is_none());assert!(solver.selection_graph.is_none());assert!(solver.opponent_stays.is_none());
    }

    #[test]
    fn opponent_tree_conserves_travel_budget_and_skips_blocked_cells(){
        use tester::model::Map;
        let p=Problem{starts_at:0,day_seconds:vec![60,60],day_steps:vec![40,40],
            map:Map{height:2,width:4,cells:vec![vec![1,1,1,3],vec![0,1,1,1]]},
            spots:vec![Spot{brand:0,pos:2,stocks:2},Spot{brand:1,pos:7,stocks:2}],
            agents:vec![0,4],fuel_limits:20,players:5,busy_threshold:2,jammed_threshold:5};
        let mut obs=Game::new(p.clone(),&[1,0]).observation(0);
        obs.others=vec![serde_json::json!({"id":1,"agents":[{"pos":0},{"pos":4}]})];
        let solver=Solver::new(p,0,0.0,2.0,0.0);
        let tree=solver.opponent_tree(&obs,40);
        assert!((tree.iter().sum::<f64>()-80.0).abs()<1e-8);
        assert_eq!(tree[3],0.0);assert!(tree.iter().all(|v|v.is_finite()&&*v>=0.0));
        assert_eq!(tree,solver.opponent_tree(&obs,40));
        obs.others.clear();assert!(solver.opponent_tree(&obs,40).is_empty());
    }

    #[test]
    fn forecast_graph_preserves_reused_selection_numbering(){
        let p=Problem{starts_at:0,day_seconds:vec![60,60],day_steps:vec![40,40],
            map:Map{height:5,width:7,cells:vec![vec![0;7];5]},
            spots:vec![Spot{brand:0,pos:2,stocks:2},Spot{brand:1,pos:31,stocks:2}],
            agents:vec![0,34],fuel_limits:20,players:5,busy_threshold:2,jammed_threshold:5};
        let obs=Game::new(p.clone(),&[1,0]).observation(0);
        let g=Graph::new(&p,&obs,&[]);let offset=p.spots.len()+obs.agents.len();
        // A cached hub can already be a target but appear in another insertion order.
        let reordered=Graph::new(&p,&obs,&[g.positions[g.targets-1]]);
        assert_ne!(g.positions,reordered.positions);
        let matching=Graph::new(&p,&obs,&g.positions[offset..g.targets]);
        assert_eq!(g.positions,matching.positions);
    }

    #[test]
    fn supplier_path_modes_match_public_engine_and_cache() {
        for spot_positions in [vec![2,8,15,21,27,33],(0..21).collect::<Vec<_>>()] {
        let p=Problem {
            starts_at:0, day_seconds:vec![60,60], day_steps:vec![40,40],
            map:Map {height:5,width:7,cells:(0..5).map(|y|(0..7).map(|x|((x+2*y)%3)as u8).collect()).collect()},
            spots:spot_positions.into_iter().enumerate().map(|(i,pos)|Spot{brand:i/2,pos,stocks:2}).collect(),
            agents:vec![0,10,24,34],fuel_limits:16,players:5,busy_threshold:2,jammed_threshold:5,
        };
        let mut game=Game::new(p.clone(),&[1,0,0,0]);
        game.agents[1].fuel=3;game.agents[2].fuel=7;
        let mut obs=game.observation(0);
        for (i,t) in obs.traffics.iter_mut().enumerate(){t.status=(i%3)as u8;}
        game.set_observed_traffic(obs.traffics.clone());
        let graph=Graph::new(&p,&obs,&[]);
        let mut solver=Solver::new(p,0,0.0,2.0,0.0);
        solver.supplier=0;
        let mut rng=Rng(1234567);
        for case in 0..240 {
            let mut solution=Solution {
                hubs:std::array::from_fn(|_|rng.n(graph.targets)),
                hub_modes:std::array::from_fn(|_|rng.n(PATH_MODES)),
                departures:std::array::from_fn(|_|rng.n(41)as i32),
                routes:vec![vec![];4],
            };
            solution.departures.sort();
            for route in &mut solution.routes[1..]{
                for _ in 0..8{route.push(rng.n(solver.p.spots.len()+2*STOPS)+rng.n(PATH_MODES)*(solver.p.spots.len()+2*STOPS));}
            }
            // Prime the cache and alter only supplier paths, keeping collector tokens fixed.
            let _=solver.evaluate(&graph,&obs,40,&solution,false,None);
            for mode in &mut solution.hub_modes{*mode=(*mode+1)%PATH_MODES;}
            let cached=solver.evaluate(&graph,&obs,40,&solution,false,None);
            let again=solver.evaluate(&graph,&obs,40,&solution,false,None);
            let out=solver.evaluate(&graph,&obs,40,&solution,true,None);
            assert!((cached.value-out.value).abs()<1e-8);
            assert_eq!(again.value,cached.value);
            assert_eq!(cached.masks,out.masks);
            let supply=graph.supply_cache.borrow().as_ref().unwrap().1.clone();
            for car in 1..4{
                let pos=obs.agents[car].pos;
                let mut prefix=CarPrefix{mask:if graph.spot_at[pos]>=0{1<<graph.spot_at[pos]}else{0},node:solver.p.spots.len()+car,time:0,fuel:obs.agents[car].fuel as i32,penalty:0.0,complete:true};
                for &token in &solution.routes[car]{prefix=solver.extend_prefix(&graph,&supply,&solution,40,prefix,token);if !prefix.complete{break;}}
                let (value,fuel)=solver.prefix_value(&graph,&obs,&supply,40,&prefix);
                assert_eq!(prefix.mask,out.masks[car]);assert_eq!(prefix.node,out.endnodes[car]);
                assert_eq!(prefix.time,out.endtime[car]);assert_eq!(prefix.fuel,out.arrivefuel[car]);assert_eq!(fuel,out.endfuel[car]);
                assert!((value-out.route_values[car]).abs()<1e-6,"prefix {case} car={car} value={value} expected={}",out.route_values[car]);
            }

            for a in 0..4{assert!((cached.route_values[a]-out.route_values[a]).abs()<1e-6);}
            let mut last_obs=obs.clone();last_obs.day=solver.p.day_steps.len()-1;
            graph.memo.borrow_mut().clear();let final_out=solver.evaluate(&graph,&last_obs,40,&solution,false,None);
            for car in 1..4{
                let pos=obs.agents[car].pos;
                let mut prefix=CarPrefix{mask:if graph.spot_at[pos]>=0{1<<graph.spot_at[pos]}else{0},node:solver.p.spots.len()+car,time:0,fuel:obs.agents[car].fuel as i32,penalty:0.0,complete:true};
                for &token in &solution.routes[car]{prefix=solver.extend_prefix(&graph,&supply,&solution,40,prefix,token);if !prefix.complete{break;}}
                assert!((solver.prefix_value(&graph,&last_obs,&supply,40,&prefix).0-final_out.route_values[car]).abs()<1e-6);
            }
            graph.memo.borrow_mut().clear();
            assert_eq!(cached.endfuel,out.endfuel);
            assert_eq!(cached.endnodes,out.endnodes);
            let mut actual=game.clone();actual.play_day(out.plans).unwrap();
            for a in 0..4 {
                assert_eq!(actual.agents[a].pos,graph.positions[out.endnodes[a]]);
                assert_eq!(actual.agents[a].fuel as i32,out.endfuel[a]);
            }
            let mut brands=0u64;let mut balls=0;
            for (i,spot) in solver.p.spots.iter().enumerate(){
                let count=out.masks.iter().filter(|&&mask|mask&(1<<i)!=0).count().min(spot.stocks as usize);
                balls+=count;if count>0{brands|=1u64<<spot.brand;}
            }
            assert_eq!(actual.score.daily_brands,brands.count_ones()as usize);
            assert_eq!(actual.score.udon,balls as u32);
            if case==0{
                let mut pooled=solution.clone();let before=solver.evaluate(&graph,&obs,40,&pooled,false,None).value;
                solver.fleet_pool(&graph,&obs,40,&mut pooled,Instant::now()+Duration::from_millis(100));
                let outcome=solver.evaluate(&graph,&obs,40,&pooled,true,None);
                assert!(outcome.value>=before-1e-6);
                let mut check=game.clone();check.play_day(outcome.plans).unwrap();
                for a in 0..4{assert_eq!(check.agents[a].pos,graph.positions[outcome.endnodes[a]]);assert_eq!(check.agents[a].fuel as i32,outcome.endfuel[a]);}
            }
        }
        }
    }
}
impl Graph {
    fn new(p: &Problem, obs: &Observation, extra:&[usize]) -> Self {
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
    fn fuel_distance(&self,p:&Problem,source:usize,target:usize)->i32{
        let missing=self.fuel_to.borrow()[target].is_empty();
        if missing {
            let pos=self.positions[target];let mut distance=vec![99999;p.map.len()];
            let mut heap=BinaryHeap::new();distance[pos]=0;heap.push((Reverse(0),pos));
            while let Some((Reverse(cost),x))=heap.pop(){if distance[x]!=cost{continue;}
                for y in p.map.neighbors(x){if p.map.terrain(y)==3{continue;}let next=cost+self.costs[y];
                    if next<distance[y]{distance[y]=next;heap.push((Reverse(next),y));}
                }
            }
            self.fuel_to.borrow_mut()[target]=distance;
        }
        self.fuel_to.borrow()[target][source]
    }
}
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Solution { hubs:[usize;STOPS], hub_modes:[usize;STOPS], departures:[i32;STOPS-1], routes:Vec<Vec<usize>> }
impl Solution {
    fn overwrite(&mut self,other:&Self){
        self.hubs=other.hubs;self.hub_modes=other.hub_modes;self.departures=other.departures;
        for (route,source) in self.routes.iter_mut().zip(&other.routes){route.clear();route.extend_from_slice(source);}
    }
}
#[derive(Clone,Default)]
struct Outcome {value:f64, route_values:[f64;MAX_AGENTS], masks:[u32;MAX_AGENTS], plans:Plan, endfuel:[i32;MAX_AGENTS], arrivefuel:[i32;MAX_AGENTS], endtime:[i32;MAX_AGENTS], endnodes:[usize;MAX_AGENTS], actual:[usize;MAX_AGENTS]}
struct Rng(u64);
impl Rng {fn u(&mut self)->u64{self.0^=self.0<<13;self.0^=self.0>>7;self.0^=self.0<<17;self.0}fn n(&mut self,n:usize)->usize{(self.u() as usize)%n.max(1)}fn f(&mut self)->f64{(self.u()>>11) as f64/(1u64<<53)as f64}}

pub struct Solver {p:Problem, selection_graph:Option<Rc<Graph>>, opponent_stays:Option<(usize,Vec<f64>)>, forecast_other:Vec<f64>, supplier:usize, game:Option<Game>, last:Option<Plan>, rng:Rng, cache:Option<[Solution;2]>, cache_positions:Vec<usize>, cache_day:usize, cache_complete:bool, selecting:bool, seen_brands:u64, coverage_weight:f64, beam_time_weight:f64, archive_weight:f64, future_weight:f64, reserve_weight:f64,last_value:f64, choice_value:f64, raw_value:f64, canonical_value:f64, temperature_scale:f64, exchange:Option<Arc<Mutex<Elite>>>,cross:Option<(Arc<CrossBus>,usize)>}
impl Solver {
    pub fn new(p:Problem,exploration_seed:u64,coverage_weight:f64,beam_time_weight:f64,archive_weight:f64)->Self{let seed=p.agents.iter().fold(9183479123,|a,&b|(a^b as u64).wrapping_mul(673493))^exploration_seed.wrapping_mul(0x9e3779b185ebca87);Self{p,selection_graph:None,opponent_stays:None,forecast_other:vec![],supplier:0,game:None,last:None,rng:Rng(seed|1),cache:None,cache_positions:vec![],cache_day:0,cache_complete:false,selecting:false,seen_brands:0,coverage_weight,beam_time_weight,archive_weight,future_weight:std::env::var("PROCON37_G007_FUTURE_WEIGHT").ok().and_then(|x|x.parse::<f64>().ok()).filter(|x|x.is_finite()&&*x>=0.0&&*x<=1.5).unwrap_or(0.0),reserve_weight:std::env::var("PROCON37_G007_RESERVE_WEIGHT").ok().and_then(|x|x.parse::<f64>().ok()).filter(|x|x.is_finite()&&*x>=0.0&&*x<=100.0).unwrap_or(15.0),last_value:f64::NEG_INFINITY,choice_value:f64::NEG_INFINITY,raw_value:f64::NEG_INFINITY,canonical_value:f64::NEG_INFINITY,temperature_scale:1.0,exchange:None,cross:None}}
    pub fn select(&mut self,limit:u64)->Vec<u8>{
        let began=Instant::now();let n=self.p.agents.len();
        if limit<=110 {
            let mut kinds=vec![0;n];kinds[0]=1;self.supplier=0;
            self.game=Some(Game::new(self.p.clone(),&kinds));self.last=None;
            self.cache=None;self.cache_positions.clear();self.selection_graph=None;
            return kinds;
        }
        let obs=Game::new(self.p.clone(),&vec![0;n]).observation(0);
        let g=Rc::new(Graph::new(&self.p,&obs,&[]));self.selection_graph=Some(g.clone());let steps=self.p.day_steps[0]as i32;
        let graph_millis=began.elapsed().as_millis()as u64;
        let mut initial_millis=0u64;
        let mut bestv=f64::NEG_INFINITY;let mut best_supplier=0;let mut warm_all=Vec::new();
        for a in 0..n {
            if a>0&&began.elapsed().as_millis()>u128::from(limit*70/100){break;}
            self.supplier=a;
            let initial_started=Instant::now();
            let candidate=self.initial(&g,&obs,steps,None);let out=self.evaluate(&g,&obs,steps,&candidate,false,None);
            initial_millis=initial_millis.max(initial_started.elapsed().as_millis()as u64);
            if out.value>bestv {bestv=out.value;best_supplier=a;}
            warm_all.push((a,candidate));
        }
        let positions=g.positions.clone();drop(g);
        self.supplier=best_supplier;
        let initial=warm_all.iter().find(|(a,_)|*a==best_supplier).unwrap().1.clone();
        self.cache=Some([initial.clone(),initial]);self.cache_positions=positions.clone();self.cache_complete=false;self.cache_day=0;
        let mut kinds=vec![0;n];kinds[self.supplier]=1;self.game=Some(Game::new(self.p.clone(),&kinds));
        // Rebuilding paths and constructing a feasible solution cannot be interrupted.
        // Reserve their measured cost before starting another role or forecast probe.
        let minimum_plan=(3*graph_millis+2*initial_millis+300).max(700);
        let remaining=limit.saturating_sub(began.elapsed().as_millis()as u64+200);
        if remaining>=minimum_plan*warm_all.len()as u64 {
            // Optimize each role independently: changing the supplier need not preserve an old route assignment.
            let slot=remaining/warm_all.len()as u64;
            warm_all.sort_by_key(|(a,_)|usize::from(*a!=best_supplier));
            let mut selected:Option<(usize,[Solution;2],Vec<usize>,f64)>=None;
            for (a,initial) in warm_all {
                let available=limit.saturating_sub(began.elapsed().as_millis()as u64+150);if available<minimum_plan{break;}
                self.supplier=a;self.last=None;self.selecting=false;
                let mut kinds=vec![0;n];kinds[a]=1;self.game=Some(Game::new(self.p.clone(),&kinds));
                self.cache=Some([initial.clone(),initial]);self.cache_positions=positions.clone();self.cache_day=0;self.cache_complete=false;
                let observation=self.game.as_ref().unwrap().observation(0);
                let role_started=Instant::now();let role_budget=slot.min(available);
                let rollout=self.future_weight==0.0&&self.p.day_steps.len()>1&&role_budget>=minimum_plan*3;
                let forecast_count=if rollout{if role_budget>=minimum_plan*5&&self.p.day_steps.len()>2{2}else{1}}else{0};
                let first_limit=if forecast_count==2{role_budget/2}else if rollout{role_budget*60/100}else{role_budget};
                let first_plan=self.plan_inner(&observation,steps as u32,first_limit,true,false);self.last=None;
                let first_value=self.last_value;let first_cache=self.cache.as_ref().unwrap().clone();let first_coords=self.cache_positions.clone();
                let mut forecast_values=[0.0;2];let mut forecast_done=0;
                if rollout {
                    let mut previous_plan=first_plan;let mut previous_observation=observation.clone();
                    for day in 1..=forecast_count {
                        let forecast_limit=role_budget.saturating_sub(role_started.elapsed().as_millis()as u64+50)/(forecast_count-day+1)as u64;
                        if forecast_limit<minimum_plan{break;}
                        let mut next_game=self.game.as_ref().unwrap().clone();
                        if next_game.play_day(previous_plan).is_err(){break;}
                        let mut next_observation=next_game.observation(0);
                        next_observation.traffics=self.forecast_traffic(self.game.as_ref().unwrap().last_stays(),next_game.last_stays(),&previous_observation.traffics);
                        self.game=Some(next_game);self.seen_brands=u64::MAX;
                        let next_steps=self.p.day_steps[day];
                        previous_plan=self.plan_inner(&next_observation,next_steps,forecast_limit,true,false);
                        forecast_values[day-1]=self.last_value;forecast_done+=1;self.last=None;
                        previous_observation=next_observation;
                    }
                    self.seen_brands=0;
                }
                let role_value=first_value+0.5*forecast_values[0]+0.25*forecast_values[1];
                eprintln!("g007 selection supplier={} score={:.1} forecast={:.1} forecast2={:.1} forecast_days={} total={:.1} millis={}",a,first_value,forecast_values[0],forecast_values[1],forecast_done,role_value,role_started.elapsed().as_millis());
                if selected.as_ref().is_none_or(|x|role_value>x.3){
                    selected=Some((a,first_cache,first_coords,role_value));
                }
            }
            if let Some((a,cache,coords,_))=selected{self.supplier=a;self.cache=Some(cache);self.cache_positions=coords;self.cache_complete=true;self.cache_day=0;}
            let mut kinds=vec![0;n];kinds[self.supplier]=1;self.game=Some(Game::new(self.p.clone(),&kinds));self.last=None;
            let extra=limit.saturating_sub(began.elapsed().as_millis()as u64+200);
            if extra>minimum_plan{let observation=self.game.as_ref().unwrap().observation(0);let _=self.plan_inner(&observation,steps as u32,extra,true,false);self.last=None;}
        }else if remaining>minimum_plan {
            let observation=self.game.as_ref().unwrap().observation(0);self.selecting=true;
            let _=self.plan_inner(&observation,steps as u32,remaining,true,false);self.selecting=false;self.last=None;
        }
        self.selection_graph=None;
        let mut kinds=vec![0;n];kinds[self.supplier]=1;kinds
    }
    fn terminal_value(&self,g:&Graph,node:usize,fuel:i32,supply_end:usize)->f64{
        let cap=self.p.fuel_limits as i32;
        let needed=g.fuel_distance(&self.p,g.positions[node],supply_end);
        let mut value=-(needed-fuel).max(0)as f64*self.reserve_weight+0.20*fuel.min(cap/2)as f64;
        let required=g.mobility[node];
        if fuel<required{value-=1600.0+20.0*(required-fuel)as f64;}
        value-2.0*(cap/4-fuel).max(0)as f64
    }
    fn forecast_traffic(&self,previous:&[u32],current:&[u32],observed:&[Traffic])->Vec<Traffic>{
        if self.forecast_other.len()!=self.p.map.len(){
            return self.forecast_traffic_ratio(previous,current,observed,1,2);
        }
        observed.iter().map(|road|{
            let own=previous.get(road.pos).copied().unwrap_or(0)+current.get(road.pos).copied().unwrap_or(0);
            let busy=f64::from(self.p.busy_threshold);let jam=f64::from(self.p.jammed_threshold);
            let prior=match road.status{0=>0.25*busy,1=>0.5*(busy+jam),_=>1.3*jam};
            // Endpoint trees estimate only a fraction of a full day's travel;
            // own simulated stays and the observed category carry most weight.
            let count=0.5*f64::from(own)+0.05*self.forecast_other[road.pos]+0.25*prior;
            Traffic{pos:road.pos,status:if count>=jam{2}else if count>=busy{1}else{0}}
        }).collect()
    }
    fn forecast_traffic_ratio(&self,previous:&[u32],current:&[u32],observed:&[Traffic],numerator:u64,denominator:u64)->Vec<Traffic>{
        observed.iter().map(|road|{
            let count=u64::from(previous.get(road.pos).copied().unwrap_or(0))+u64::from(current.get(road.pos).copied().unwrap_or(0));
            let status=if count*numerator>=denominator*u64::from(self.p.jammed_threshold){2}else if count*numerator>=denominator*u64::from(self.p.busy_threshold){1}else{0};
            Traffic{pos:road.pos,status:status.max(road.status.min(1))}
        }).collect()
    }
    fn archive_push(&self,archive:&mut Vec<(Solution,Outcome,Vec<u64>)>,s:&Solution,out:Outcome){
        let mut brands=0u64;
        for (i,spot) in self.p.spots.iter().enumerate(){if out.masks.iter().any(|&mask|mask&(1<<i)!=0){brands|=1u64<<spot.brand;}}
        let mut key=vec![brands&!self.seen_brands,out.endnodes[self.supplier]as u64];
        let mut ends=(0..self.p.agents.len()).filter(|&a|a!=self.supplier).map(|a|(out.endnodes[a],out.endfuel[a]/4)).collect::<Vec<_>>();
        ends.sort_unstable();for (node,fuel) in ends{key.push(node as u64);key.push(fuel as u64);}
        if let Some(entry)=archive.iter_mut().find(|entry|entry.2==key){
            if out.value>entry.1.value{*entry=(s.clone(),out,key);}
        }else{archive.push((s.clone(),out,key));}
        archive.sort_unstable_by(|a,b|b.1.value.total_cmp(&a.1.value));archive.truncate(24);
    }
    fn check_prediction(&self,game:&Game,g:&Graph,predicted:&Outcome,label:&str)->Option<Game>{
        let mut test=game.clone();
        if let Err(e)=test.play_day(predicted.plans.clone()){eprintln!("validation fallback {}: {}",label,e.message);return None;}
        let mut brands=0u64;let mut balls=0;
        for (i,spot) in self.p.spots.iter().enumerate(){let count=predicted.masks.iter().filter(|&&v|v&(1<<i)!=0).count().min(spot.stocks as usize);if count>0{brands|=1u64<<spot.brand;}balls+=count;}
        if test.score.daily_brands-game.score.daily_brands!=brands.count_ones()as usize||test.score.udon-game.score.udon!=balls as u32{eprintln!("score mismatch {} expected={}/{} actual={}/{}",label,brands.count_ones(),balls,test.score.daily_brands-game.score.daily_brands,test.score.udon-game.score.udon);}
        for a in 0..self.p.agents.len(){if test.agents[a].pos!=g.positions[predicted.endnodes[a]]||test.agents[a].fuel as i32!=predicted.endfuel[a]{eprintln!("model mismatch {} car={} predicted={}/{} actual={}/{}",label,a,g.positions[predicted.endnodes[a]],predicted.endfuel[a],test.agents[a].pos,test.agents[a].fuel);}}
        Some(test)
    }
    fn evaluate(&self,g:&Graph,obs:&Observation,steps:i32,s:&Solution,output:bool,prev:Option<&Outcome>)->Outcome{
        let p=&self.p;let m=p.spots.len();let n=p.agents.len();let cap=p.fuel_limits as i32;
        let mut out=Outcome{plans:if output{vec![vec![];n]}else{vec![]},..Outcome::default()};
        let sup=self.supplier;
        // Collector-only changes share the exact same supplier timeline.
        // An exact key and a single retained entry avoid both hash aliasing and growth.
        let mut supply_key=[0usize;3+STOPS*2+STOPS-1];
        supply_key[0]=sup;supply_key[1]=steps as usize;supply_key[2]=prev.map_or(m+sup,|v|v.endnodes[sup]);
        for k in 0..STOPS{supply_key[3+k]=s.hubs[k];supply_key[3+STOPS+k]=s.hub_modes[k];}
        for k in 0..STOPS-1{supply_key[3+2*STOPS+k]=s.departures[k]as usize;}
        let cached=if output{None}else{g.supply_cache.borrow().as_ref().filter(|(key,_)|*key==supply_key).map(|(_,value)|value.clone())};
        let supply=if let Some(cached)=cached{cached}else{
            let mut value=0.0;
        let mut windows=[(steps+1,steps+1);STOPS];
        let mut st=0;let mut snode=prev.map_or(m+sup,|v|v.endnodes[sup]);
        let mut endhub=snode;let mut trace=[g.positions[snode];MAX_STEPS+1];
        for k in 0..STOPS {
            let h=s.hubs[k];let mode=s.hub_modes[k];let path=if mode==0{&g.paths[snode][h]}else{&g.alternatives[mode-1][snode][h]};let arrival=st+path.time;
            let mut q=g.positions[snode];let mut clock=st;let mut moved=0;
            for &r in &path.cells {let finish=clock+g.durations[q];if finish>steps{break;}
                if output{out.plans[sup].push((0..6).find(|&d|p.map.neighbor(q,d)==Some(r)).expect("shortest path edge")as i64);}
                for v in clock+1..finish{trace[v as usize]=q;}trace[finish as usize]=r;clock=finish;q=r;moved+=1;
            }
            st=clock;snode=g.cell_node[q];endhub=snode;
            if moved<path.cells.len(){value-=0.002*(arrival-steps).max(0)as f64;break;}
            endhub=h;snode=h;
            let depart=if k<STOPS-1{s.departures[k].max(st).min(steps)}else{steps};
            windows[k]=(st,depart);for v in st+1..=depart{trace[v as usize]=g.positions[h];}
            if output&&depart>st{out.plans[sup].push(-i64::from(depart-st));}
            st=depart;
        }
        for v in st+1..=steps{trace[v as usize]=g.positions[endhub];}
        if output&&st<steps{out.plans[sup].push(-i64::from(steps-st));}
            let result=Rc::new(SupplierTrace{windows,trace,endhub,value});
            if !output{*g.supply_cache.borrow_mut()=Some((supply_key,result.clone()));}
            result
        };
        let windows=&supply.windows;let trace=&supply.trace;let endhub=supply.endhub;
        out.value+=supply.value;
        out.endnodes[sup]=endhub;out.endfuel[sup]=cap;out.arrivefuel[sup]=cap;
        let mut base_hash=(17389853233441u64,672594024112321u64);
        let mix=|h:&mut(u64,u64),v:usize|{h.0=(h.0^v as u64).wrapping_mul(0x9e3779b185ebca87);h.1=(h.1.wrapping_add(v as u64+17)).rotate_left(17).wrapping_mul(0xc2b2ae3d27d4eb4f);};
        for v in [sup,steps as usize,prev.is_some()as usize,prev.map_or(m+sup,|v|v.endnodes[sup])]{mix(&mut base_hash,v);}
        for &v in &s.hubs{mix(&mut base_hash,v);}for &v in &s.hub_modes{mix(&mut base_hash,v);}for &v in &s.departures{mix(&mut base_hash,v as usize);}
        if g.memo.borrow().len()>100000{g.memo.borrow_mut().clear();}
        for a in 0..n {if a==sup{continue;}
            let mut node=prev.map_or(m+a,|v|v.endnodes[a]);let mut pos=g.positions[node];let mut fuel=prev.map_or(obs.agents[a].fuel as i32,|v|v.endfuel[a]);let mut t=0;let mut mask=0u32;
            let mut hash=base_hash;mix(&mut hash,node);mix(&mut hash,fuel as usize);mix(&mut hash,s.routes[a].len());
            for &v in &s.routes[a]{mix(&mut hash,v);}
            if !output {if let Some(&(cm,cf,cn,ca,ct,cv,caf))=g.memo.borrow().get(&hash){out.masks[a]=cm;out.endfuel[a]=cf;out.arrivefuel[a]=caf;out.endnodes[a]=cn;out.actual[a]=ca;out.endtime[a]=ct;out.value+=cv;out.route_values[a]=cv;continue;}}
            let before=out.value;
            if g.spot_at[pos]>=0 {mask|=1<<g.spot_at[pos];}
            for (idx,&encoded)in s.routes[a].iter().enumerate(){
                let token=encoded%(m+2*STOPS);let mode=encoded/(m+2*STOPS);
                let meet=if token>=m{Some((token-m)%STOPS)}else{None};
                let next=meet.map_or(token,|k|s.hubs[k]);let path=if mode==0{&g.paths[node][next]}else{&g.alternatives[mode-1][node][next]};
                let mut projected_fuel=fuel;let mut q=pos;let mut clock=t;let mut moved=0;let mut collected=0u32;
                for &r in &path.cells {
                    let mut finish=clock+g.durations[q];if finish>steps{break;}
                    let mut next_fuel=projected_fuel;
                    for v in clock+1..finish {if trace[v as usize]==q{next_fuel=cap;}}
                    if next_fuel<g.costs[q]{
                        if let Some(resume)=(clock+1..=steps-g.durations[q]).find(|&v|trace[v as usize]==q){
                            if output{out.plans[a].push(-i64::from(resume-clock));}
                            clock=resume;finish=clock+g.durations[q];next_fuel=cap;
                        }else{break;}
                    }
                    if output{out.plans[a].push((0..6).find(|&d|p.map.neighbor(q,d)==Some(r)).expect("shortest path edge")as i64);}
                    next_fuel-=g.costs[q];if trace[finish as usize]==r{next_fuel=cap;}
                    projected_fuel=next_fuel;clock=finish;q=r;moved+=1;
                    if g.spot_at[r]>=0{collected|=1<<g.spot_at[r];}
                }
                t=clock;fuel=projected_fuel;pos=q;mask|=collected;
                if moved<path.cells.len(){
                    node=g.cell_node[pos];if moved>0{out.actual[a]=idx+1;}
                    out.value-=0.002*((t+path.time-steps).max(0)+(path.fuel-fuel).max(0))as f64;
                    break;
                }
                node=next;out.actual[a]=idx+1;
                if let Some(k)=meet {let (arrival,depart)=windows[k];let target=t.max(if token>=m+STOPS{depart}else{arrival}).max(1);
                    if target<=depart && target<=steps {
                        if output&&target>t{out.plans[a].push(-i64::from(target-t));}t=target;fuel=cap;
                    }else{out.value-=2.0;}
                }
            }
            out.value-=0.12*t as f64;out.arrivefuel[a]=fuel;
            for v in t+1..=steps{if trace[v as usize]==pos{fuel=cap;break;}}
            if output&&t<steps{out.plans[a].push(-i64::from(steps-t));}
            // A car ending with a reachable supplier can restart reliably on the following day.
            if (prev.is_some()||self.future_weight==0.0) && obs.day+1+usize::from(prev.is_some())<p.day_steps.len(){
                out.value+=self.terminal_value(g,node,fuel,endhub);
            }
            out.masks[a]=mask;out.endtime[a]=t;out.endfuel[a]=fuel;out.endnodes[a]=node;
            out.route_values[a]=out.value-before;
            if !output {g.memo.borrow_mut().insert(hash,(mask,fuel,node,out.actual[a],t,out.route_values[a],out.arrivefuel[a]));}
        }
        let mut brands=0u64;let mut balls=0;
        for i in 0..m{let count=out.masks.iter().filter(|&&v|v&(1<<i)!=0).count().min(p.spots[i].stocks as usize);if count>0{brands|=1u64<<p.spots[i].brand;}balls+=count;}
        if self.coverage_weight>0.0&&(prev.is_some()||self.future_weight==0.0){
            if let Some(&next_steps)=p.day_steps.get(obs.day+1+usize::from(prev.is_some())){
                let mut nearest=[99999i32;64];
                for a in 0..n{if a==sup{continue;}
                    for (i,&(time,fuel))in g.potential[out.endnodes[a]].iter().enumerate(){
                        let estimate=time+(fuel-out.endfuel[a]).max(0);
                        let brand=p.spots[i].brand;nearest[brand]=nearest[brand].min(estimate);
                    }
                }
                let threshold=next_steps as i32/4;
                for time in nearest{if time<99999{out.value-=self.coverage_weight*(time-threshold).max(0)as f64;}}
            }
        }
        // With no remaining day, fuel and route completion time are tie-breakers.
        if obs.day+1+usize::from(prev.is_some())>=p.day_steps.len(){
            out.value*=0.01;
            for value in &mut out.route_values{*value*=0.01;}
        }
        let mut already=self.seen_brands;
        if let Some(previous)=prev {for i in 0..m{if previous.masks.iter().any(|&mask|mask&(1<<i)!=0){already|=1u64<<p.spots[i].brand;}}}
        if obs.day>0 {out.value+=(brands&!already).count_ones()as f64*100000.0;}
        out.value+=brands.count_ones()as f64*1500.0+balls as f64*if prev.is_some(){12.0}else{20.0};
        out
    }
    fn initial(&mut self,g:&Graph,obs:&Observation,steps:i32,prev:Option<&Outcome>)->Solution{
        let m=self.p.spots.len();let n=self.p.agents.len();let mut best=(i32::MAX,0);
        for h in 0..m{let mut score=0;for a in 0..n{if a==self.supplier{continue;}score+=g.paths[prev.map_or(m+a,|v|v.endnodes[a])][h].time;}
            score+=g.paths[prev.map_or(m+self.supplier,|v|v.endnodes[self.supplier])][h].time*2;if score<best.0{best=(score,h);}}
        let mut s=Solution{hubs:[best.1;STOPS],hub_modes:[0;STOPS],departures:std::array::from_fn(|i|steps*(i+1)as i32/STOPS as i32),routes:vec![vec![];n]};
        for a in 0..n{if a!=self.supplier&&prev.map_or(obs.agents[a].fuel as i32,|v|v.endfuel[a])<(self.p.fuel_limits/2)as i32{s.routes[a].push(m);}}
        let mut used=vec![false;m];
        for _ in 0..m {let mut choose=None;let mut best_gain=f64::NEG_INFINITY;let base=self.evaluate(g,obs,steps,&s,false,prev).value;
            for a in 0..n{if a==self.supplier{continue;}for x in 0..m{if used[x]{continue;}
                for idx in 0..=s.routes[a].len(){
                    s.routes[a].insert(idx,x);let val=self.evaluate(g,obs,steps,&s,false,prev).value-base;s.routes[a].remove(idx);
                    if val>best_gain{best_gain=val;choose=Some((a,x,idx));}
                }
            }}
            if let Some((a,x,idx))=choose{if best_gain<0.1{break;}used[x]=true;s.routes[a].insert(idx,x);}
        }
        s
    }
    fn extend_prefix(&self,g:&Graph,supply:&SupplierTrace,s:&Solution,steps:i32,prefix:CarPrefix,encoded:usize)->CarPrefix{
        let m=self.p.spots.len();let cap=self.p.fuel_limits as i32;
        let token=encoded%(m+2*STOPS);let mode=encoded/(m+2*STOPS);
        let meet=if token>=m{Some((token-m)%STOPS)}else{None};let target=meet.map_or(token,|k|s.hubs[k]);
        let path=if mode==0{&g.paths[prefix.node][target]}else{&g.alternatives[mode-1][prefix.node][target]};
        let mut c=prefix;let mut pos=g.positions[c.node];let mut moved=0;
        for &next in &path.cells{
            let mut finish=c.time+g.durations[pos];if finish>steps{break;}
            let mut fuel=c.fuel;
            for t in c.time+1..finish{if supply.trace[t as usize]==pos{fuel=cap;}}
            if fuel<g.costs[pos]{
                if let Some(resume)=(c.time+1..=steps-g.durations[pos]).find(|&t|supply.trace[t as usize]==pos){c.time=resume;finish=resume+g.durations[pos];fuel=cap;}else{break;}
            }
            fuel-=g.costs[pos];if supply.trace[finish as usize]==next{fuel=cap;}
            c.fuel=fuel;c.time=finish;pos=next;moved+=1;
            if g.spot_at[pos]>=0{c.mask|=1<<g.spot_at[pos];}
        }
        if moved<path.cells.len(){
            c.node=g.cell_node[pos];c.complete=false;
            c.penalty-=0.002*((c.time+path.time-steps).max(0)+(path.fuel-c.fuel).max(0))as f64;
            return c;
        }
        c.node=target;
        if let Some(k)=meet{
            let (arrival,depart)=supply.windows[k];let time=c.time.max(if token>=m+STOPS{depart}else{arrival}).max(1);
            if time<=depart&&time<=steps{c.time=time;c.fuel=cap;}else{c.penalty-=2.0;}
        }
        c
    }
    fn prefix_value(&self,g:&Graph,obs:&Observation,supply:&SupplierTrace,steps:i32,c:&CarPrefix)->(f64,i32){
        let mut fuel=c.fuel;let pos=g.positions[c.node];
        if (c.time+1..=steps).any(|t|supply.trace[t as usize]==pos){fuel=self.p.fuel_limits as i32;}
        let mut value=c.penalty-0.12*c.time as f64;
        if self.future_weight==0.0&&obs.day+1<self.p.day_steps.len(){value+=self.terminal_value(g,c.node,fuel,supply.endhub);}
        if obs.day+1>=self.p.day_steps.len(){value*=0.01;}
        (value,fuel)
    }
    fn beam_car(&self,g:&Graph,obs:&Observation,steps:i32,s:&mut Solution,car:usize,deadline:Instant,prev:Option<&Outcome>)->usize{
        if prev.is_some()||self.coverage_weight>0.0||self.future_weight>0.0{return self.beam_car_legacy(g,obs,steps,s,car,deadline,prev);}
        let m=self.p.spots.len();let base=m+2*STOPS;
        let original=self.evaluate(g,obs,steps,s,false,None).value;
        let mut best=s.routes[car].clone();let mut best_value=original;
        let mut trial=s.clone();trial.routes[car].clear();
        let zero=self.evaluate(g,obs,steps,&trial,false,None);
        let supply=g.supply_cache.borrow().as_ref().unwrap().1.clone();
        let mut available=0u32;let mut other_brands=0u64;
        for i in 0..m{
            let visits=(0..self.p.agents.len()).filter(|&a|a!=car&&zero.masks[a]&(1<<i)!=0).count();
            if visits<self.p.spots[i].stocks as usize{available|=1<<i;}
            if visits>0&&self.p.spots[i].stocks>0{other_brands|=1u64<<self.p.spots[i].brand;}
        }
        let marginal=|mask:u32|{
            let mut brands=0u64;let mut remaining=mask;
            while remaining!=0{let i=remaining.trailing_zeros()as usize;remaining&=remaining-1;if self.p.spots[i].stocks>0{brands|=1u64<<self.p.spots[i].brand;}}
            brands&=!other_brands;
            20.0*(mask&available).count_ones()as f64+1500.0*brands.count_ones()as f64
                +if obs.day>0{100000.0*(brands&!self.seen_brands).count_ones()as f64}else{0.0}
        };
        let fixed=zero.value-zero.route_values[car]-marginal(zero.masks[car]);
        let start=CarPrefix{mask:zero.masks[car],node:m+car,time:0,fuel:obs.agents[car].fuel as i32,penalty:0.0,complete:true};
        let time_weight=self.beam_time_weight*(m as f64/32.0).powi(2);
        let mut beam=vec![(Vec::<usize>::new(),start)];let mut layers=0;
        'search: for depth in 0..20{
            let mut expanded=Vec::<(f64,Vec<usize>,CarPrefix)>::new();
            let mut labels=HashMap::<(u32,usize),Vec<usize>>::new();
            for (route,prefix) in &beam{
                if Instant::now()>=deadline{break 'search;}
                for token in 0..base{
                    if token<m&&prefix.mask&(1<<token)!=0{continue;}
                    if route.last().is_some_and(|v|v%base==token){continue;}
                    let target=if token<m{token}else{s.hubs[(token-m)%STOPS]};
                    for mode in 0..PATH_MODES{
                        if g.path_mode_mask[prefix.node][target]&(1<<mode)==0{continue;}
                        let encoded=token+mode*base;
                        let next=self.extend_prefix(g,&supply,s,steps,*prefix,encoded);
                        let value=fixed+self.prefix_value(g,obs,&supply,steps,&next).0+marginal(next.mask);
                        if value>best_value+0.00001{best_value=value;best=route.clone();best.push(encoded);}
                        if !next.complete||next.time>=steps{continue;}
                        let group=labels.entry((next.mask,next.node)).or_default();
                        if group.iter().any(|&i|{let old=&expanded[i];old.0>=value-0.00001&&old.2.time<=next.time&&old.2.fuel>=next.fuel}){continue;}
                        for &i in group.iter(){let old=&mut expanded[i];if value>=old.0-0.00001&&next.time<=old.2.time&&next.fuel>=old.2.fuel{old.0=f64::NEG_INFINITY;}}
                        let mut new_route=route.clone();new_route.push(encoded);group.push(expanded.len());expanded.push((value,new_route,next));
                    }
                }
            }
            expanded.retain(|entry|entry.0.is_finite());if expanded.is_empty(){break;}layers=depth+1;
            expanded.sort_unstable_by(|a,b|(b.0-time_weight*b.2.time as f64).total_cmp(&(a.0-time_weight*a.2.time as f64)));expanded.truncate(32);
            beam=expanded.into_iter().map(|(_,route,prefix)|(route,prefix)).collect();
        }
        trial.routes[car]=best;
        // Keep the full simulator as the final authority at this optimization boundary.
        if self.evaluate(g,obs,steps,&trial,false,None).value>original+0.00001{s.routes[car]=trial.routes[car].clone();}
        layers
    }
    fn beam_car_legacy(&self,g:&Graph,obs:&Observation,steps:i32,s:&mut Solution,car:usize,deadline:Instant,prev:Option<&Outcome>)->usize{
        let m=self.p.spots.len();let base=m+2*STOPS;
        let time_weight=self.beam_time_weight*(m as f64/32.0).powi(2);
        let mut best=s.routes[car].clone();let mut best_value=self.evaluate(g,obs,steps,s,false,prev).value;
        let mut trial=s.clone();trial.routes[car].clear();
        let zero=self.evaluate(g,obs,steps,&trial,false,prev);
        let mut beam=vec![(Vec::<usize>::new(),zero.masks[car],zero.endnodes[car])];
        let mut layers=0;
        for depth in 0..20 {
            let mut expanded=Vec::<(f64,Vec<usize>,u32,usize,i32,i32)>::new();
            let mut seen=HashMap::<(u32,usize),Vec<usize>>::new();
            for (route,mask,node) in &beam {
                if Instant::now()>=deadline {s.routes[car]=best;return layers;}
                for token in 0..base {
                    if token<m&&mask&(1<<token)!=0{continue;}
                    if route.last().is_some_and(|v|v%base==token){continue;}
                    let target=if token<m{token}else{s.hubs[(token-m)%STOPS]};
                    for mode in 0..PATH_MODES {
                        if g.path_mode_mask[*node][target]&(1<<mode)==0{continue;}
                        trial.routes[car].clear();trial.routes[car].extend_from_slice(route);trial.routes[car].push(token+mode*base);
                        let out=self.evaluate(g,obs,steps,&trial,false,prev);
                        if out.value>best_value+0.00001{best_value=out.value;best=trial.routes[car].clone();}
                        if out.endtime[car]>=steps||out.actual[car]<trial.routes[car].len(){continue;}
                        let key=(out.masks[car],out.endnodes[car]);
                        let time=out.endtime[car];let fuel=out.arrivefuel[car];
                        let group=seen.entry(key).or_default();
                        // Prefixes continue with their arrival fuel, before the implicit rest-of-day waiting.
                        if group.iter().any(|&idx|{let old=&expanded[idx];old.0>=out.value-0.00001&&old.4<=time&&old.5>=fuel}){continue;}
                        for &idx in group.iter(){let old=&mut expanded[idx];if out.value>=old.0-0.00001&&time<=old.4&&fuel>=old.5{old.0=f64::NEG_INFINITY;}}
                        group.push(expanded.len());expanded.push((out.value,trial.routes[car].clone(),out.masks[car],out.endnodes[car],time,fuel));
                    }
                }
            }
            expanded.retain(|entry|entry.0.is_finite());
            if expanded.is_empty(){break;}
            layers=depth+1;
            expanded.sort_unstable_by(|a,b|(b.0-time_weight*b.4 as f64).total_cmp(&(a.0-time_weight*a.4 as f64)));expanded.truncate(32);
            beam=expanded.into_iter().map(|(_,r,m,node,_,_)|(r,m,node)).collect();
        }
        s.routes[car]=best;layers
    }
    fn fleet_pool(&self,g:&Graph,obs:&Observation,steps:i32,s:&mut Solution,deadline:Instant)->(usize,bool){
        let m=self.p.spots.len();let base=m+2*STOPS;
        let cars=(0..self.p.agents.len()).filter(|&a|a!=self.supplier).collect::<Vec<_>>();
        let generation_deadline=Instant::now()+deadline.saturating_duration_since(Instant::now())*3/4;
        let mut trial=s.clone();for &car in &cars{trial.routes[car].clear();}
        let mut pools=Vec::<Vec<(Vec<usize>,u32,f64)>>::new();
        let mut expanded_count=0;
        for (index,&car) in cars.iter().enumerate(){
            let car_deadline=Instant::now()+generation_deadline.saturating_duration_since(Instant::now())/((cars.len()-index)as u32);
            let mut combined_routes=Vec::<(Vec<usize>,u32,f64)>::new();
            for context in 0..2 {
            // Keep routes that work alone and routes that complement the current fleet.
            // Their own masks and costs can be combined under the same supplier trace.
            for &other in &cars {
                if context==1&&other!=car{trial.routes[other]=s.routes[other].clone();}
                else{trial.routes[other].clear();}
            }
            let phase_deadline=Instant::now()+car_deadline.saturating_duration_since(Instant::now())/((2-context)as u32);
            let mut pool=HashMap::<u32,(Vec<usize>,f64)>::new();
            trial.routes[car]=s.routes[car].clone();let seed=self.evaluate(g,obs,steps,&trial,false,None);
            pool.insert(seed.masks[car],(trial.routes[car].clone(),seed.route_values[car]));
            trial.routes[car].clear();let empty=self.evaluate(g,obs,steps,&trial,false,None);
            pool.entry(empty.masks[car]).or_insert((vec![],empty.route_values[car]));
            let supply=g.supply_cache.borrow().as_ref().unwrap().1.clone();
            let mut available=0u32;let mut base_brands=0u64;
            for i in 0..m{
                let visits=(0..self.p.agents.len()).filter(|&a|a!=car&&empty.masks[a]&(1<<i)!=0).count();
                if visits<self.p.spots[i].stocks as usize{available|=1<<i;}
                if visits>0&&self.p.spots[i].stocks>0{base_brands|=1u64<<self.p.spots[i].brand;}
            }
            let marginal=|mask:u32|{
                let mut brands=0u64;let mut remaining=mask;
                while remaining!=0{let i=remaining.trailing_zeros()as usize;remaining&=remaining-1;if self.p.spots[i].stocks>0{brands|=1u64<<self.p.spots[i].brand;}}
                brands&=!base_brands;
                20.0*(mask&available).count_ones()as f64+1500.0*brands.count_ones()as f64
                    +if obs.day>0{100000.0*(brands&!self.seen_brands).count_ones()as f64}else{0.0}
            };
            let start=CarPrefix{mask:empty.masks[car],node:m+car,time:0,fuel:obs.agents[car].fuel as i32,penalty:0.0,complete:true};
            let mut beam=vec![(Vec::<usize>::new(),start)];
            'layers: for _ in 0..18{
                let mut expanded=Vec::<(f64,Vec<usize>,CarPrefix)>::new();
                let mut labels=HashMap::<(u32,usize),Vec<usize>>::new();
                for (route,prefix) in &beam{
                    for token in 0..base{
                        if Instant::now()>=phase_deadline{break 'layers;}
                        if token<m&&prefix.mask&(1<<token)!=0{continue;}
                        if route.last().is_some_and(|v|v%base==token){continue;}
                        let target=if token<m{token}else{s.hubs[(token-m)%STOPS]};
                        for mode in 0..PATH_MODES{
                            if g.path_mode_mask[prefix.node][target]&(1<<mode)==0{continue;}
                            let encoded=token+mode*base;let next=self.extend_prefix(g,&supply,s,steps,*prefix,encoded);expanded_count+=1;
                            let cost=self.prefix_value(g,obs,&supply,steps,&next).0;let value=cost+marginal(next.mask);
                            if pool.get(&next.mask).is_none_or(|entry|cost>entry.1){let mut r=route.clone();r.push(encoded);pool.insert(next.mask,(r,cost));}
                            if !next.complete||next.time>=steps{continue;}
                            let group=labels.entry((next.mask,next.node)).or_default();
                            if group.iter().any(|&i|{let old=&expanded[i];old.0>=value-0.00001&&old.2.time<=next.time&&old.2.fuel>=next.fuel}){continue;}
                            for &i in group.iter(){let old=&mut expanded[i];if value>=old.0-0.00001&&next.time<=old.2.time&&next.fuel>=old.2.fuel{old.0=f64::NEG_INFINITY;}}
                            let mut r=route.clone();r.push(encoded);group.push(expanded.len());expanded.push((value,r,next));
                        }
                    }
                }
                expanded.retain(|x|x.0.is_finite());
                expanded.sort_unstable_by(|a,b|(b.0-2.0*b.2.time as f64).total_cmp(&(a.0-2.0*a.2.time as f64)));expanded.truncate(96);
                if expanded.is_empty(){break;}beam=expanded.into_iter().map(|(_,route,prefix)|(route,prefix)).collect();
            }
            let mut routes=pool.into_iter().map(|(mask,(route,cost))|(route,mask,cost)).collect::<Vec<_>>();
            let mut occupied=[0usize;32];let mut other_brands=0u64;
            if context==1 {
                for spot in 0..m {
                    occupied[spot]=(0..self.p.agents.len()).filter(|&a|a!=car&&empty.masks[a]&(1<<spot)!=0).count();
                    if occupied[spot]>0&&self.p.spots[spot].stocks>0{other_brands|=1u64<<self.p.spots[spot].brand;}
                }
            }
            let route_score=|entry:&(Vec<usize>,u32,f64)|{
                let brands=(0..m).filter(|&i|entry.1&(1<<i)!=0).fold(0u64,|a,i|a|(1u64<<self.p.spots[i].brand))&!other_brands;
                let balls=(0..m).filter(|&i|entry.1&(1<<i)!=0&&occupied[i]<self.p.spots[i].stocks as usize).count();
                entry.2+1500.0*brands.count_ones()as f64+20.0*balls as f64
                    +if obs.day>0{100000.0*(brands&!self.seen_brands).count_ones()as f64}else{0.0}
            };
            routes.sort_unstable_by(|a,b|route_score(b).total_cmp(&route_score(a)).then_with(||a.1.cmp(&b.1)));routes.truncate(48);
            if !routes.iter().any(|x|x.1==seed.masks[car]){routes.push((s.routes[car].clone(),seed.masks[car],seed.route_values[car]));}
            for route in routes {
                if let Some(old)=combined_routes.iter_mut().find(|old|old.1==route.1){if route.2>old.2{*old=route;}}
                else{combined_routes.push(route);}
            }
            }
            pools.push(combined_routes);for &other in &cars{trial.routes[other].clear();}
        }
        // For a fixed supplier trajectory, collector routes are independent.
        // Combine whole routes while accounting for shared shop stocks and brands.
        #[derive(Clone)]
        struct Combination{counts:u128,brands:u64,value:f64,bound:f64,choices:[usize;MAX_AGENTS]}
        let mut remaining=vec![0u64;cars.len()+1];
        let mut remaining_visits=vec![[0u8;32];cars.len()+1];
        for i in (0..cars.len()).rev(){
            remaining[i]=remaining[i+1];remaining_visits[i]=remaining_visits[i+1];
            let possible=pools[i].iter().fold(0u32,|mask,entry|mask|entry.1);
            for spot in 0..m{if possible&(1<<spot)!=0&&self.p.spots[spot].stocks>0{
                remaining[i]|=1u64<<self.p.spots[spot].brand;remaining_visits[i][spot]+=1;
            }}
        }
        let route_bounds=pools.iter().map(|pool|pool.iter().map(|entry|{
            let brands=(0..m).filter(|&spot|entry.1&(1<<spot)!=0&&self.p.spots[spot].stocks>0)
                .fold(0u64,|mask,spot|mask|(1u64<<self.p.spots[spot].brand));
            (entry.1,brands,entry.2)
        }).collect::<Vec<_>>()).collect::<Vec<_>>();
        let mut remaining_local=vec![0.0;cars.len()+1];
        for i in (0..cars.len()).rev(){remaining_local[i]=remaining_local[i+1]+pools[i].iter().map(|entry|entry.2).fold(f64::NEG_INFINITY,f64::max);}
        let mut combinations=vec![Combination{counts:0,brands:0,value:0.0,bound:0.0,choices:[0;MAX_AGENTS]}];
        for (i,pool) in pools.iter().enumerate(){
            if Instant::now()>=deadline{return (expanded_count,false);}
            let mut next=Vec::<Combination>::new();let mut best_counts=HashMap::<u128,usize>::new();
            for combination in &combinations{
                if Instant::now()>=deadline{return (expanded_count,false);}
                for (choice,entry) in pool.iter().enumerate(){
                    let mut c=combination.clone();c.choices[i]=choice;c.value+=entry.2;
                    let mut mask=entry.1;
                    while mask!=0{
                        let spot=mask.trailing_zeros()as usize;mask&=mask-1;let shift=3*spot;
                        let count=((c.counts>>shift)&7)as usize;
                        if count<(self.p.spots[spot].stocks as usize).min(7){c.counts+=1u128<<shift;c.value+=20.0;}
                        let brand=1u64<<self.p.spots[spot].brand;
                        if self.p.spots[spot].stocks>0&&c.brands&brand==0{c.brands|=brand;c.value+=1500.0;if obs.day>0&&self.seen_brands&brand==0{c.value+=100000.0;}}
                    }
                    let future=remaining[i+1]&!c.brands;
                    c.bound=c.value+remaining_local[i+1]+1500.0*future.count_ones()as f64;
                    let mut available=0u32;
                    for spot in 0..m{
                        let count=((c.counts>>(3*spot))&7)as usize;
                        let stock=(self.p.spots[spot].stocks as usize).saturating_sub(count);
                        if stock>0{available|=1<<spot;}
                        c.bound+=20.0*stock.min(remaining_visits[i+1][spot]as usize)as f64;
                    }
                    if obs.day>0{c.bound+=100000.0*(future&!self.seen_brands).count_ones()as f64;}
                    // A remaining car must choose one route; it cannot realize the
                    // union of every route in its pool. Independent marginal maxima
                    // give another upper bound, including each route's fuel/time cost.
                    if cars.len()-i-1<=3 {
                        let mut marginal_bound=c.value;
                        for pool in &route_bounds[i+1..]{
                            marginal_bound+=pool.iter().map(|&(mask,brands,cost)|{
                                let new=brands&!c.brands;
                                cost+20.0*(mask&available).count_ones()as f64+1500.0*new.count_ones()as f64
                                    +if obs.day>0{100000.0*(new&!self.seen_brands).count_ones()as f64}else{0.0}
                            }).fold(f64::NEG_INFINITY,f64::max);
                        }
                        c.bound=c.bound.min(marginal_bound);
                    }
                    if let Some(&old)=best_counts.get(&c.counts){if c.value>next[old].value{next[old]=c;}}
                    else{best_counts.insert(c.counts,next.len());next.push(c);}
                }
            }
            next.sort_unstable_by(|a,b|b.bound.total_cmp(&a.bound).then_with(||b.value.total_cmp(&a.value)).then_with(||a.counts.cmp(&b.counts)));next.truncate(192);combinations=next;
        }
        let mut best=self.evaluate(g,obs,steps,s,false,None).value;
        for combination in combinations.iter().take(8){
            for (i,&car) in cars.iter().enumerate(){trial.routes[car]=pools[i][combination.choices[i]].0.clone();}
            let value=self.evaluate(g,obs,steps,&trial,false,None).value;
            if value>best{best=value;s.overwrite(&trial);}
        }
        (expanded_count,true)
    }
    fn horizon(&self,g:&Graph,ng:&Graph,obs:&Observation,steps:i32,next_steps:i32,s:&[Solution;2])->f64{
        let first=self.evaluate(g,obs,steps,&s[0],false,None);
        if next_steps<=0{return first.value;}
        let second=self.evaluate(ng,obs,next_steps,&s[1],false,Some(&first));
        first.value+self.future_weight*second.value
    }
    fn opponent_tree(&self,obs:&Observation,steps:u32)->Vec<f64>{
        let size=self.p.map.len();
        if obs.others.is_empty(){return vec![];}
        let mut durations=(0..size).map(|x|if self.p.map.terrain(x)==2{3}else{2}).collect::<Vec<i32>>();
        for road in &obs.traffics{durations[road.pos]=[1,2,4][road.status as usize];}
        let mut tree=vec![0.0;size];
        for other in &obs.others{
            let Some(agents)=other.get("agents").and_then(|v|v.as_array())else{continue;};
            for agent in agents{
                let Some(root)=agent.get("pos").and_then(|v|v.as_u64()).map(|v|v as usize)else{continue;};
                if root>=size||self.p.map.terrain(root)==3{continue;}
                let mut distance=vec![i32::MAX;size];let mut parent=vec![usize::MAX;size];
                let mut order=Vec::with_capacity(size);let mut heap=BinaryHeap::new();
                distance[root]=0;heap.push(Reverse((0,root)));
                while let Some(Reverse((cost,x)))=heap.pop(){
                    if cost!=distance[x]{continue;}order.push(x);
                    for y in self.p.map.neighbors(x){
                        if self.p.map.terrain(y)==3{continue;}
                        let nd=cost+durations[x];
                        if nd<distance[y]{distance[y]=nd;parent[y]=x;heap.push(Reverse((nd,y)));}
                    }
                }
                let mut flow=vec![0.0;size];let mut total=0.0;
                for spot in &self.p.spots{
                    let d=distance[spot.pos];
                    if spot.pos!=root&&d<=steps as i32{
                        let weight=1.0/(f64::from(d)+4.0).powi(2);
                        flow[spot.pos]+=weight;total+=weight*f64::from(d);
                    }
                }
                if total<=0.0{continue;}
                let scale=f64::from(steps)/total;
                for &x in order.iter().rev(){
                    let q=parent[x];if q==usize::MAX{continue;}
                    tree[q]+=flow[x]*f64::from(durations[q])*scale;flow[q]+=flow[x];
                }
            }
        }
        tree
    }
    pub fn plan(&mut self,obs:&Observation,steps:u32,limit:u64,accepted:bool)->Plan{
        if limit<=110 {
            self.forecast_other.clear();self.opponent_stays=None;
            return self.plan_inner(obs,steps,limit,accepted,true);
        }
        let tree=self.opponent_tree(obs,steps);
        self.forecast_other=tree.clone();
        if let Some((day,previous))=self.opponent_stays.take(){
            if day+1==obs.day{for (sum,value) in self.forecast_other.iter_mut().zip(previous){*sum+=value;}}
        }
        self.opponent_stays=Some((obs.day,tree));
        self.plan_inner(obs,steps,limit,accepted,false)
    }
    // Rejected selection and reconnects can change the actual roles. The regular
    // evaluator assumes exactly one supplier, so unsupported role sets use a
    // conservative collector plan with the observed fuel and no assumed refill.
    fn collector_plan(&mut self,obs:&Observation,steps:u32,deadline:Instant)->Plan{
        let p=&self.p;let size=p.map.len();let mut durations=vec![2u32;size];let mut costs=vec![1u32;size];
        for x in 0..size{match p.map.terrain(x){1=>{durations[x]=1;costs[x]=2;},2=>{durations[x]=3;costs[x]=2;},_=>{}}}
        for traffic in &obs.traffics{durations[traffic.pos]=[1,2,4][traffic.status as usize];}
        let mut stocks=p.spots.iter().map(|s|s.stocks).collect::<Vec<_>>();let mut brands=self.seen_brands;
        let mut plan=vec![vec![];obs.agents.len()];
        for (car,agent) in obs.agents.iter().enumerate(){
            let mut pos=agent.pos;let mut fuel=agent.fuel;let mut time=0u32;let mut visited=vec![false;p.spots.len()];
            if agent.kind==0{while Instant::now()<deadline{
                let mut distances=vec![(u32::MAX,u32::MAX);size];let mut parent=vec![usize::MAX;size];let mut heap=BinaryHeap::new();
                distances[pos]=(0,0);heap.push(Reverse((0u32,0u32,pos)));
                while let Some(Reverse((f,t,x)))=heap.pop(){
                    if Instant::now()>=deadline{break;}
                    if distances[x]!=(f,t){continue;}
                    for y in p.map.neighbors(x){if p.map.terrain(y)==3{continue;}
                        let next=(f+costs[x],t+durations[x]);
                        if next.0<=fuel&&next.1<=steps-time&&next<distances[y]{distances[y]=next;parent[y]=x;heap.push(Reverse((next.0,next.1,y)));}
                    }
                }
                if Instant::now()>=deadline{break;}
                let target=p.spots.iter().enumerate().filter(|(i,s)|!visited[*i]&&stocks[*i]>0&&s.pos!=pos&&distances[s.pos].0<=fuel)
                    .max_by_key(|(_,s)|(usize::from(brands&(1u64<<s.brand)==0),Reverse(distances[s.pos].1)));
                let Some((_,target))=target else{break;};
                let mut cells=vec![];let mut x=target.pos;
                while x!=pos{cells.push(x);x=parent[x];}
                cells.reverse();
                for next in cells{
                    plan[car].push((0..6).find(|&d|p.map.neighbor(pos,d)==Some(next)).unwrap()as i64);
                    fuel-=costs[pos];time+=durations[pos];pos=next;
                    for (i,spot) in p.spots.iter().enumerate(){if spot.pos==pos&&!visited[i]&&stocks[i]>0{visited[i]=true;stocks[i]-=1;brands|=1u64<<spot.brand;}}
                }
            }}
            if time<steps{plan[car].push(-i64::from(steps-time));}
        }
        if let Some(game)=&self.game{if game.clone().play_day(plan.clone()).is_err(){plan=game.waiting_plan();}}
        self.last=Some(plan.clone());self.last_value=f64::NEG_INFINITY;plan
    }
    fn plan_inner(&mut self,obs:&Observation,steps:u32,limit:u64,accepted:bool,allow_archive:bool)->Plan{
        let began=Instant::now();
        let coverage_guard=allow_archive&&self.archive_weight==0.0&&self.future_weight==0.0&&self.p.spots.len()<=16&&limit>=20000&&obs.day+1<self.p.day_steps.len();
        let use_archive=allow_archive&&self.future_weight==0.0&&obs.day+1<self.p.day_steps.len()&&((self.archive_weight>0.0&&limit>=5000)||coverage_guard);
        if let Some(game)=self.game.as_mut(){
            if let Some(last)=self.last.take(){let plan=if accepted{last}else{game.waiting_plan()};if let Ok(day)=game.play_day(plan){for frame in day.frames{for event in frame.events{if let Event::Collect{brand,..}=event{self.seen_brands|=1u64<<brand;}}}}}
            game.agents=obs.agents.clone();game.day=obs.day;game.set_observed_traffic(obs.traffics.clone());
        }
        if !self.selecting {
            let supplier=obs.agents.iter().position(|a|a.kind==1).unwrap_or(usize::MAX);
            if supplier!=self.supplier {
                self.supplier=supplier;self.cache=None;self.cache_positions.clear();
                self.cache_complete=false;self.selection_graph=None;
            }
        }
        if limit<=110 {
            self.cache=None;self.cache_positions.clear();self.cache_complete=false;self.selection_graph=None;
            let plan=vec![vec![-i64::from(steps)];obs.agents.len()];
            self.last=Some(plan.clone());self.last_value=f64::NEG_INFINITY;
            return plan;
        }
        if !self.selecting&&obs.agents.iter().filter(|a|a.kind==1).count()!=1 {
            self.cache=None;self.cache_positions.clear();self.cache_complete=false;self.selection_graph=None;
            return self.collector_plan(obs,steps,began+Duration::from_millis(limit.saturating_sub(110).min(limit*91/100)));
        }
        let extra=self.cache.as_ref().map(|old|old.iter().flat_map(|s|s.hubs.iter().map(|&h|self.cache_positions[h])).collect::<Vec<_>>()).unwrap_or_default();
        let graph_started=Instant::now();
        let g=if obs.day==0 {
            self.selection_graph.as_ref().filter(|graph|extra.iter().all(|&pos|graph.cell_node[pos]<graph.targets)).cloned()
                .unwrap_or_else(||Rc::new(Graph::new(&self.p,obs,&extra)))
        }else{Rc::new(Graph::new(&self.p,obs,&extra))};
        let graph_millis=graph_started.elapsed().as_millis();
        let mut forecast=obs.clone();
        for t in &mut forecast.traffics {t.status=t.status.max(1);}
        let mut next_graph=if self.future_weight>0.0{Some(Graph::new(&self.p,&forecast,&g.positions[self.p.spots.len()+obs.agents.len()..g.targets]))}else{None};
        let ng=next_graph.as_ref().unwrap_or(&g);
        let steps=steps as i32;let m=self.p.spots.len();let n=self.p.agents.len();
        let next_steps=if self.future_weight>0.0{self.p.day_steps.get(obs.day+1).copied().unwrap_or(0)as i32}else{0};
        let first=self.initial(&g,obs,steps,None);
        let out=self.evaluate(&g,obs,steps,&first,false,None);
        let second=if next_steps>0{self.initial(&ng,obs,next_steps,Some(&out))}else{first.clone()};
        let mut current=[first,second];
        let mut val=self.horizon(&g,&ng,obs,steps,next_steps,&current);
        if let Some(mut old)=self.cache.take(){
            for solution in &mut old{for h in &mut solution.hubs{*h=g.cell_node[self.cache_positions[*h]];}}
            let same_day=obs.day==self.cache_day;
            if !same_day {
                // Yesterday's endpoint is a natural starting point for its reverse tour.
                // Re-evaluate all travel and refuelling under today's actual traffic.
                let previous_steps=self.p.day_steps[self.cache_day]as i32;
                let mut arrivals=[0i32;STOPS];let mut clock=0;
                let mut node=g.cell_node[self.cache_positions[m+self.supplier]];
                for k in 0..STOPS{
                    clock+=g.paths[node][old[0].hubs[k]].time;arrivals[k]=clock;
                    if k<STOPS-1{clock=clock.max(old[0].departures[k]);}node=old[0].hubs[k];
                }
                let mut reversed=old[0].clone();reversed.hubs.reverse();
                reversed.hub_modes=std::array::from_fn(|k|if k==0{0}else{old[0].hub_modes[STOPS-k]});
                reversed.departures=std::array::from_fn(|k|((previous_steps-arrivals[STOPS-1-k]).max(0)*steps/previous_steps.max(1)).clamp(0,steps));
                reversed.departures.sort();
                for route in &mut reversed.routes{
                    route.reverse();
                    for encoded in route{
                        let base=m+2*STOPS;let mode=*encoded/base;let token=*encoded%base;
                        if token>=m{let old_stop=(token-m)%STOPS;let kind=usize::from(token<m+STOPS);*encoded=mode*base+m+STOPS-1-old_stop+kind*STOPS;}
                    }
                }
                let reverse_plan=[reversed,current[1].clone()];
                let score=self.horizon(&g,&ng,obs,steps,next_steps,&reverse_plan);
                if score>val{current=reverse_plan;val=score;}
                eprintln!("g007 reverse_warm day={} score={:.3}",obs.day,score);
            }
            let alternative=if same_day&&self.cache_complete{old}else{[old[if same_day{0}else{1}].clone(),current[1].clone()]};
            let v=self.horizon(&g,&ng,obs,steps,next_steps,&alternative);if v>val{current=alternative;val=v;}
        }
        let mut best=current.clone();let mut bestv=val;let mut best_supplier=self.supplier;
        let total_budget=Duration::from_millis(limit.saturating_sub(110).min(limit*91/100).max(1));
        let budget=if coverage_guard{total_budget*4/5}else if use_archive{total_budget*3/4}else{total_budget};
        let mut cross=self.cross.as_ref().and_then(|(bus,worker)|cross_elite::Binding::from_solver(bus,*worker,self,&g,obs,steps,began+budget));
        let mut archive=Vec::<(Solution,Outcome,Vec<u64>)>::new();
        let mut it=0u64;let mut phase=0.0;let mut beam_calls=0usize;let mut beam_layers=0usize;
        let mut fleet_calls=0usize;let mut fleet_accepted=0usize;let mut fleet_improved=0usize;let mut fleet_rescues=0usize;
        let mut triple_calls=0usize;let mut triple_accepted=0usize;let mut triple_improved=0usize;
        let mut restart_cycle=0u32;let mut restart_calls=0usize;
        let mut candidate=current.clone();
        while began.elapsed()<budget {
            it+=1;if it%512==0{phase=began.elapsed().as_secs_f64()/budget.as_secs_f64();}
            candidate[0].overwrite(&current[0]);
            if next_steps>0{candidate[1].overwrite(&current[1]);}
            let day=if next_steps>0&&self.rng.n(100)<46{1}else{0};let next=&mut candidate[day];let ds=if day==0{steps}else{next_steps};let mut a=self.rng.n(n);while a==self.supplier{a=self.rng.n(n);}
            let typ=self.rng.n(if n>=4{124}else{116});
            if typ<6 {let len=next.routes[a].len();if len>0{let i=self.rng.n(len);next.routes[a][i]=next.routes[a][i]%(m+2*STOPS)+(m+2*STOPS)*self.rng.n(PATH_MODES);}}
            else if typ<30 {let len=next.routes[a].len();if len<24{let idx=self.rng.n(len+1);let token=if self.rng.n(6)==0{m+self.rng.n(2*STOPS)}else{self.rng.n(m)};next.routes[a].insert(idx,token);}}
            else if typ<47 {let len=next.routes[a].len();if len>0{let idx=self.rng.n(len);next.routes[a].remove(idx);}}
            else if typ<66 {let len=next.routes[a].len();if len>0{let i=self.rng.n(len);let j=self.rng.n(len);if self.rng.n(2)==0{next.routes[a].swap(i,j);}else{next.routes[a][i.min(j)..=i.max(j)].reverse();}}}
            else if typ<79 {let mut b=self.rng.n(n);while b==self.supplier{b=self.rng.n(n);}let len=next.routes[a].len();if len>0{let i=self.rng.n(len);let token=next.routes[a].remove(i);let j=self.rng.n(next.routes[b].len()+1);next.routes[b].insert(j,token);}}
            else if typ<91 {let len=next.routes[a].len();if len>0{let i=self.rng.n(len);next.routes[a][i]=if self.rng.n(6)==0{m+self.rng.n(2*STOPS)}else{self.rng.n(m)};}}
            else if typ<97 {
                let k=self.rng.n(STOPS);
                if self.rng.n(3)==0{next.hub_modes[k]=self.rng.n(PATH_MODES);}
                else if self.rng.n(2)==0{
                    // Move a rendezvous locally, retaining nearby meetings when possible.
                    let pos=g.positions[next.hubs[k]];
                    if let Some(cell)=self.p.map.neighbor(pos,self.rng.n(6)){
                        if self.p.map.terrain(cell)!=3&&g.cell_node[cell]<g.targets{next.hubs[k]=g.cell_node[cell];}
                    }
                }else{
                    next.hubs[k]=if self.rng.n(4)==0{next.hubs[(k+1)%STOPS]}else{self.rng.n(g.targets)};
                    if self.rng.n(4)==0{let j=self.rng.n(STOPS);next.hubs[j]=next.hubs[k];}
                }
            }
            else if typ<100{let k=self.rng.n(STOPS-1);next.departures[k]=(next.departures[k]+self.rng.n(21)as i32-10).clamp(0,ds);next.departures.sort();}
            else if typ<116 {
                // Move related visits together instead of forcing every intermediate assignment through the objective.
                let mut b=a;while b==a||b==self.supplier{b=self.rng.n(n);}
                if self.rng.n(2)==0 {
                    let ia=self.rng.n(next.routes[a].len()+1);let ib=self.rng.n(next.routes[b].len()+1);
                    if ia+next.routes[b].len()-ib<=24&&ib+next.routes[a].len()-ia<=24{
                        let tail_a=next.routes[a].split_off(ia);let tail_b=next.routes[b].split_off(ib);
                        next.routes[a].extend(tail_b);next.routes[b].extend(tail_a);
                    }
                }else if !next.routes[a].is_empty(){
                    let begin=self.rng.n(next.routes[a].len());let end=begin+1+self.rng.n(next.routes[a].len()-begin);
                    if next.routes[b].len()+end-begin<=24{
                        let mut segment=next.routes[a].drain(begin..end).collect::<Vec<_>>();
                        if self.rng.n(4)==0{segment.reverse();}
                        let index=self.rng.n(next.routes[b].len()+1);next.routes[b].splice(index..index,segment);
                    }
                }
            }else{
                // Reassign visits among three collectors without scoring a partially completed cycle.
                triple_calls+=1;
                let mut b=a;while b==a||b==self.supplier{b=self.rng.n(n);}
                let mut c=a;while c==a||c==b||c==self.supplier{c=self.rng.n(n);}
                if self.rng.n(2)==0{
                    let ia=self.rng.n(next.routes[a].len()+1);let ib=self.rng.n(next.routes[b].len()+1);let ic=self.rng.n(next.routes[c].len()+1);
                    if ia+next.routes[c].len()-ic<=24&&ib+next.routes[a].len()-ia<=24&&ic+next.routes[b].len()-ib<=24{
                        let tail_a=next.routes[a].split_off(ia);let tail_b=next.routes[b].split_off(ib);let tail_c=next.routes[c].split_off(ic);
                        next.routes[a].extend(tail_c);next.routes[b].extend(tail_a);next.routes[c].extend(tail_b);
                    }
                }else if !next.routes[a].is_empty()&&!next.routes[b].is_empty()&&!next.routes[c].is_empty(){
                    let ia=self.rng.n(next.routes[a].len());let ib=self.rng.n(next.routes[b].len());let ic=self.rng.n(next.routes[c].len());
                    let token=next.routes[a][ia];next.routes[a][ia]=next.routes[c][ic];next.routes[c][ic]=next.routes[b][ib];next.routes[b][ib]=token;
                }
            }
            if next_steps>0 && self.rng.n(12)==0 {
                if self.rng.n(2)==0 {
                    let from=self.rng.n(2);let to=1-from;
                    if !candidate[from].routes[a].is_empty(){
                        let len=candidate[from].routes[a].len();let idx=if from==0{len-1}else{0};
                        let token=candidate[from].routes[a].remove(idx);
                        let idx=if to==1{0}else{candidate[to].routes[a].len()};candidate[to].routes[a].insert(idx,token);
                    }
                }else{
                    let mut b=self.rng.n(n);while b==self.supplier{b=self.rng.n(n);}
                    for d in 0..2{candidate[d].routes.swap(a,b);}
                }
            }
            if self.rng.n(16)==0 {
                let d=if next_steps>0{self.rng.n(2)}else{0};let graph=if d==0{g.as_ref()}else{ng};let ds=if d==0{steps}else{next_steps};
                let first=self.evaluate(&g,obs,steps,&candidate[0],false,None);
                let start=if d==0{m+a}else{first.endnodes[a]};
                let supplier_start=if d==0{m+self.supplier}else{first.endnodes[self.supplier]};
                let c=&mut candidate[d];let idx=self.rng.n(c.routes[a].len()+1);
                let mut node=start;let mut t=0;
                for &v in &c.routes[a][..idx]{let token=v%(m+2*STOPS);let next=if token<m{token}else{c.hubs[(token-m)%STOPS]};t+=graph.paths[node][next].time;node=next;}
                if idx<c.routes[a].len()&&self.rng.n(2)==0 {
                    let token=c.routes[a][idx]%(m+2*STOPS);let target=if token<m{token}else{c.hubs[(token-m)%STOPS]};
                    let path=&graph.paths[node][target];
                    if !path.cells.is_empty(){let take=self.rng.n(path.cells.len())+1;let mut pos=graph.positions[node];for &cell in &path.cells[..take]{t+=graph.durations[pos];pos=cell;}node=graph.cell_node[pos];}
                }
                let k=(t*STOPS as i32/ds.max(1)).clamp(0,STOPS as i32-1)as usize;
                if node>=graph.targets {
                    node=(0..graph.targets).min_by_key(|&h|graph.paths[node][h].time).unwrap();
                }
                c.hubs[k]=node;
                if k<STOPS-1 {let earliest=graph.paths[supplier_start][node].time;c.departures[k]=t.max(earliest).min(ds);c.departures.sort();}
                if c.routes[a].len()<24{c.routes[a].insert(idx,m+k+if self.rng.n(2)==0{STOPS}else{0});}
            }
            let old_supplier=self.supplier;
            if self.selecting&&self.rng.n(32)==0{
                let proposed=self.rng.n(n);
                for d in 0..2{candidate[d].routes.swap(old_supplier,proposed);}
                self.supplier=proposed;
            }
            let nv=self.horizon(&g,&ng,obs,steps,next_steps,&candidate);
            let cycle=(phase*12.0).fract();let temperature=(if phase<0.8{(350.0f64).powf(1.0-cycle)*0.8}else{(1.0-phase)*12.0+0.08})*self.temperature_scale;
            if nv>=val || self.rng.f()<((nv-val)/temperature).exp(){std::mem::swap(&mut current,&mut candidate);val=nv;if typ>=116{triple_accepted+=1;}}else{self.supplier=old_supplier;}
            if val>bestv{best=current.clone();bestv=val;best_supplier=self.supplier;if typ>=116{triple_improved+=1;}}
            if use_archive&&it%4096==0&&val>=bestv-120.0 {
                let out=self.evaluate(&g,obs,steps,&current[0],false,None);
                self.archive_push(&mut archive,&current[0],out);
            }
            if it%4096==0 && began.elapsed()<budget {
                let mut repaired=current.clone();let d=if next_steps>0{self.rng.n(2)}else{0};
                let a=(0..n).filter(|&x|x!=self.supplier).nth(self.rng.n(n-1)).unwrap();
                let token=if self.rng.n(2)==0&&!repaired[d].routes[a].is_empty(){let idx=self.rng.n(repaired[d].routes[a].len());repaired[d].routes[a].remove(idx)}else{self.rng.n(m+2*STOPS)};
                let mut repair_best=repaired.clone();let mut repair_value=self.horizon(&g,&ng,obs,steps,next_steps,&repaired);
                for b in 0..n{if b==self.supplier||repaired[d].routes[b].len()>=24{continue;}
                    for idx in 0..=repaired[d].routes[b].len(){
                        repaired[d].routes[b].insert(idx,token);let v=self.horizon(&g,&ng,obs,steps,next_steps,&repaired);
                        if v>repair_value{repair_value=v;repair_best=repaired.clone();}repaired[d].routes[b].remove(idx);
                    }
                }
                if repair_value>val{current=repair_best;val=repair_value;if val>bestv{best=current.clone();bestv=val;best_supplier=self.supplier;}}
            }
            if it%8192==0 && budget.saturating_sub(began.elapsed())>Duration::from_millis(40) {
                let mut repair=current.clone();let d=if next_steps>0{self.rng.n(2)}else{0};
                let a=(0..n).filter(|&x|x!=self.supplier).nth(self.rng.n(n-1)).unwrap();
                let mut b=a;while b==a||b==self.supplier{b=self.rng.n(n);}
                for car in [a,b]{repair[d].routes[car].retain(|_|self.rng.n(100)<25);}
                let mut rv=self.horizon(&g,&ng,obs,steps,next_steps,&repair);
                for _ in 0..24 {
                    if budget.saturating_sub(began.elapsed())<Duration::from_millis(25){break;}
                    let mut chosen=None;let mut new_value=rv;
                    for car in [a,b] {if repair[d].routes[car].len()>=24{continue;}
                        for token in 0..m+2*STOPS {
                            if budget.saturating_sub(began.elapsed())<Duration::from_millis(15){break;}
                            for idx in 0..=repair[d].routes[car].len(){
                                repair[d].routes[car].insert(idx,token);let v=self.horizon(&g,&ng,obs,steps,next_steps,&repair);repair[d].routes[car].remove(idx);
                                if v>new_value+0.00001{new_value=v;chosen=Some((car,idx,token));}
                            }
                        }
                    }
                    if let Some((car,idx,token))=chosen{repair[d].routes[car].insert(idx,token);rv=new_value;}else{break;}
                }
                if rv>val {current=repair;val=rv;if val>bestv{best=current.clone();bestv=val;best_supplier=self.supplier;}}
            }
            if next_steps==0&&it%32768==0&&budget.saturating_sub(began.elapsed())>Duration::from_millis(60){
                let mut repair=current[0].clone();
                let needy=if limit>=2000&&it%131072==32768&&budget.saturating_sub(began.elapsed())>Duration::from_millis(200){
                    (0..n).filter(|&a|a!=self.supplier&&obs.agents[a].fuel<self.p.fuel_limits/3&&g.paths[m+self.supplier][m+a].time<steps*3/4&&(obs.agents[a].fuel as i32)<g.fuel_distance(&self.p,obs.agents[a].pos,m+self.supplier)).collect::<Vec<_>>()
                }else{vec![]};
                let fleet=!needy.is_empty();
                if fleet {
                    fleet_calls+=1;
                    let mut order=(0..n).filter(|&a|a!=self.supplier).collect::<Vec<_>>();
                    for &a in &order{repair.routes[a].clear();}
                    for i in 1..order.len(){let j=self.rng.n(i+1);order.swap(i,j);}
                    let a=needy[self.rng.n(needy.len())];repair.hubs[0]=m+a;
                    repair.departures[0]=(g.paths[m+self.supplier][m+a].time+1).min(steps);repair.departures.sort();
                    repair.routes[a].push(m);let index=order.iter().position(|&x|x==a).unwrap();order.swap(0,index);fleet_rescues+=1;
                    let deadline=(began+budget-Duration::from_millis(15)).min(Instant::now()+Duration::from_millis(400));
                    for (i,&car) in order.iter().enumerate(){
                        let remaining=deadline.saturating_duration_since(Instant::now());
                        let car_deadline=Instant::now()+remaining/((order.len()-i)as u32);
                        beam_layers+=self.beam_car(&g,obs,steps,&mut repair,car,car_deadline,None);beam_calls+=1;
                    }
                }else{
                    let car=(0..n).filter(|&x|x!=self.supplier).nth(self.rng.n(n-1)).unwrap();
                    let mut other=car;
                    if it%65536==0 {while other==car||other==self.supplier{other=self.rng.n(n);}repair.routes[car].clear();repair.routes[other].clear();}
                    let deadline=(began+budget-Duration::from_millis(15)).min(Instant::now()+Duration::from_millis(if other==car{200}else{400}));
                    let first_deadline=if other==car{deadline}else{Instant::now()+(deadline-Instant::now())/2};
                    beam_layers+=self.beam_car(&g,obs,steps,&mut repair,car,first_deadline,None);beam_calls+=1;
                    if other!=car{beam_layers+=self.beam_car(&g,obs,steps,&mut repair,other,deadline,None);beam_calls+=1;}
                }
                let rv=self.evaluate(&g,obs,steps,&repair,false,None).value;
                if rv>val||(fleet&&(rv>=val||self.rng.f()<((rv-val)/temperature).exp())){
                    if fleet{fleet_accepted+=1;if rv>bestv{fleet_improved+=1;}}
                    current[0]=repair;val=rv;if val>bestv{best=current.clone();bestv=val;best_supplier=self.supplier;}
                }
            }
            if it%1024==0 {
                let first=self.evaluate(&g,obs,steps,&current[0],false,None);
                for a in 0..n{current[0].routes[a].truncate(first.actual[a]);}
                if next_steps>0{let second=self.evaluate(&ng,obs,next_steps,&current[1],false,Some(&first));for a in 0..n{current[1].routes[a].truncate(second.actual[a]);}}
                val=self.horizon(&g,&ng,obs,steps,next_steps,&current);
                // C012: only isolated receivers poll. Independent workers consume no new RNG.
                if next_steps==0 {
                    if let Some(binding)=cross.as_mut().filter(|binding|binding.is_receiver()) {
                        let next_restart=budget.mul_f64((restart_cycle as f64+1.0)/12.0);
                        let until_restart=next_restart.saturating_sub(began.elapsed());
                        let offers=binding.take(bestv,until_restart);
                        let mut chosen=None;let mut chosen_value=f64::NEG_INFINITY;
                        for offer in offers {
                            if !binding.active_for_seed(){break;}
                            if offer.solution()==&best[0]||offer.solution()==&current[0]{continue;}
                            let probe_started=Instant::now();
                            let value=self.evaluate(&g,obs,steps,offer.solution(),false,None).value;
                            binding.record_probe(probe_started.elapsed());
                            if value.is_finite()&&value>chosen_value{chosen_value=value;chosen=Some(offer);}
                        }
                        if let Some(offer)=chosen {
                            if binding.active_for_seed() {
                                let repair_started=Instant::now();let mut trial=offer.solution().clone();
                                if let Some(car)=cross_elite::destroy_quarter(&mut trial,self.supplier,&mut self.rng){
                                    let deadline=(began+budget-Duration::from_millis(20)).min(repair_started+cross_elite::REPAIR_LIMIT);
                                    beam_layers+=self.beam_car(&g,obs,steps,&mut trial,car,deadline,None);beam_calls+=1;
                                    let value=self.evaluate(&g,obs,steps,&trial,false,None).value;
                                    if value.is_finite(){
                                        let improved=value>bestv;
                                        current[0]=trial;val=value;
                                        if improved{best=current.clone();bestv=value;best_supplier=self.supplier;}
                                        binding.seeded(repair_started.elapsed(),improved);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if it%16384==0 {
                if let Some(binding)=&mut cross {binding.publish(bestv,&best[0]);}
                if let Some(exchange)=&self.exchange {
                    let mut elite=exchange.lock().unwrap();
                    if bestv>elite.value {elite.value=bestv;elite.solution=Some(best[0].clone());}
                }
            }
            let next_cycle=(phase*12.0)as u32;
            if next_cycle>restart_cycle {
                if let Some(exchange)=&self.exchange {
                    let elite=exchange.lock().unwrap();
                    if elite.value>bestv {
                        if let Some(solution)=&elite.solution {
                            let candidate_value=self.evaluate(&g,obs,steps,solution,false,None).value;
                            if candidate_value>bestv{best[0]=solution.clone();bestv=candidate_value;}
                        }
                    }
                }
                restart_cycle=next_cycle;restart_calls+=1;current=best.clone();self.supplier=best_supplier;
                let first=self.evaluate(&g,obs,steps,&current[0],false,None);
                for a in 0..n{current[0].routes[a].truncate(first.actual[a]);}
                if next_steps>0{let second=self.evaluate(&ng,obs,next_steps,&current[1],false,Some(&first));for a in 0..n{current[1].routes[a].truncate(second.actual[a]);}}
                val=self.horizon(&g,&ng,obs,steps,next_steps,&current);
                if next_steps==0&&limit>=10000&&restart_cycle%3==0&&budget.saturating_sub(began.elapsed())>Duration::from_millis(250){
                    let deadline=(began+budget-Duration::from_millis(20)).min(Instant::now()+Duration::from_millis((limit/50).clamp(100,1000)));
                    let mut pooled=best[0].clone();let (expanded,complete)=self.fleet_pool(&g,obs,steps,&mut pooled,deadline);
                    let value=self.evaluate(&g,obs,steps,&pooled,false,None).value;
                    eprintln!("g007 fleet_pool day={} candidates={} gain={:.3} complete={}",obs.day,expanded,value-bestv,complete);
                    if value>bestv{best[0]=pooled;bestv=value;current=best.clone();val=bestv;}
                }
            }
        }
        self.supplier=best_supplier;
        if use_archive&&!coverage_guard {
            if let Some(game)=&self.game {
                let mut after=game.clone();
                let plan=self.evaluate(&g,obs,steps,&best[0],true,None).plans;
                if after.play_day(plan).is_ok(){forecast.traffics=self.forecast_traffic(game.last_stays(),after.last_stays(),&obs.traffics);}
            }
            next_graph=Some(Graph::new(&self.p,&forecast,&g.positions[self.p.spots.len()+obs.agents.len()..g.targets]));
        }
        let ng=next_graph.as_ref().unwrap_or(&g);
        let mut used_forecast=false;let mut inspected=0usize;let mut lookahead_value=f64::NEG_INFINITY;let mut guard_brands=0usize;let mut guard_base_brands=0usize;let mut guard_changed=false;let mut guard_has_future=false;
        if use_archive {
            let out=self.evaluate(&g,obs,steps,&best[0],false,None);
            let brand_count=|out:&Outcome|self.p.spots.iter().enumerate().filter(|(i,_)|out.masks.iter().any(|&mask|mask&(1u32<<*i)!=0)).fold(0u64,|mask,(_,spot)|mask|(1u64<<spot.brand)).count_ones();
            let guard_current_brands=brand_count(&out);
            self.archive_push(&mut archive,&best[0],out);
            archive.retain(|entry|entry.1.value>=bestv-120.0&&(!coverage_guard||brand_count(&entry.1)>=guard_current_brands));
            let deadline=began+total_budget-Duration::from_millis(20);
            let count=((deadline.saturating_duration_since(Instant::now()).as_millis()/if coverage_guard{700}else{220})as usize).clamp(1,if coverage_guard{4}else{8}).min(archive.len());
            let mut choices=vec![archive.remove(0)];
            let reference=choices[0].1.value;
            while choices.len()<count&&!archive.is_empty(){
                let mut selected=0;let mut priority=f64::NEG_INFINITY;
                for (i,entry) in archive.iter().enumerate(){
                    let distance=choices.iter().map(|other|{
                        (0..n).map(|a|{let x=g.positions[entry.1.endnodes[a]];let y=g.positions[other.1.endnodes[a]];
                            (x%self.p.map.width).abs_diff(y%self.p.map.width)+(x/self.p.map.width).abs_diff(y/self.p.map.width)+(entry.1.endfuel[a]-other.1.endfuel[a]).unsigned_abs()as usize/8
                        }).sum::<usize>()
                    }).min().unwrap_or(0);
                    let value=entry.1.value-reference+2.0*distance as f64;
                    if value>priority{priority=value;selected=i;}
                }
                choices.push(archive.remove(selected));
            }
            let candidates=choices.len();let tomorrow=self.p.day_steps[obs.day+1]as i32;
            for (index,(today,out,_)) in choices.into_iter().enumerate(){
                let left=deadline.saturating_duration_since(Instant::now());if left<Duration::from_millis(30){break;}
                let slice=left/((candidates-index)as u32);let candidate_deadline=Instant::now()+slice;
                if coverage_guard {
                    // Preserve today's objective, but distinguish endpoints by attainable brands tomorrow.
                    let current_game=self.game.as_ref().unwrap();let mut after=current_game.clone();
                    let plan=self.evaluate(&g,obs,steps,&today,true,None).plans;
                    if after.play_day(plan).is_err(){continue;}
                    let mut next_observation=after.observation(0);
                    next_observation.traffics=self.forecast_traffic(current_game.last_stays(),after.last_stays(),&obs.traffics);
                    let mut probe=Solver::new(self.p.clone(),self.rng.u(),self.coverage_weight,self.beam_time_weight,0.0);
                    probe.supplier=self.supplier;probe.game=Some(after);probe.seen_brands=u64::MAX;
                    probe.future_weight=0.0;probe.reserve_weight=self.reserve_weight;
                    let probe_limit=candidate_deadline.saturating_duration_since(Instant::now()).as_millis()as u64;
                    if probe_limit<200{break;}
                    let next_plan=probe.plan_inner(&next_observation,tomorrow as u32,probe_limit,true,false);
                    let mut result=probe.game.take().unwrap();let before=result.score.daily_brands;
                    if result.play_day(next_plan).is_err(){continue;}
                    let brands=result.score.daily_brands-before;
                    if index==0{guard_base_brands=brands;}
                    let combined=out.value+1000.0*brands as f64;inspected+=1;
                    if combined>lookahead_value{
                        let mut next=probe.cache.as_ref().unwrap()[0].clone();
                        for hub in &mut next.hubs{*hub=g.cell_node[probe.cache_positions[*hub]];}
                        lookahead_value=combined;bestv=out.value;best=[today,next];guard_brands=brands;guard_changed=index>0;guard_has_future=true;
                    }
                    continue;
                }
                let mut next=self.initial(&ng,obs,tomorrow,Some(&out));
                let rounds=if slice>Duration::from_millis(1000){2}else{1};
                let mut order=(0..n).filter(|&a|a!=self.supplier).collect::<Vec<_>>();
                for round in 0..rounds {
                    for i in 1..order.len(){let j=self.rng.n(i+1);order.swap(i,j);}
                    for (j,&car) in order.iter().enumerate(){
                        let remaining=candidate_deadline.saturating_duration_since(Instant::now());if remaining<Duration::from_millis(2){break;}
                        let calls=(rounds-round)*order.len()-j;
                        let car_deadline=Instant::now()+remaining/(calls as u32);
                        beam_layers+=self.beam_car(&ng,obs,tomorrow,&mut next,car,car_deadline,Some(&out));beam_calls+=1;
                    }
                }
                let future=self.evaluate(&ng,obs,tomorrow,&next,false,Some(&out));
                let terminal=(0..n).filter(|&a|a!=self.supplier).map(|a|self.terminal_value(&g,out.endnodes[a],out.endfuel[a],out.endnodes[self.supplier])).sum::<f64>();
                let combined=out.value-terminal+self.archive_weight*future.value;inspected+=1;
                if combined>lookahead_value{lookahead_value=combined;bestv=out.value;best=[today,next];used_forecast=true;}
            }
        }
        if self.selecting {let mut kinds=vec![0;n];kinds[self.supplier]=1;self.game=Some(Game::new(self.p.clone(),&kinds));}
        let predicted=self.evaluate(&g,obs,steps,&best[0],true,None);
        let mut plan=predicted.plans.clone();
        if let Some(game)=&self.game{
            if let Some(mut after)=self.check_prediction(game,&g,&predicted,"current"){
                if used_forecast {
                    let future=self.evaluate(&ng,obs,self.p.day_steps[obs.day+1]as i32,&best[1],true,Some(&predicted));
                    after.set_observed_traffic(forecast.traffics.clone());
                    if self.check_prediction(&after,&ng,&future,"forecast").is_none(){used_forecast=false;}
                }
            }else{plan=game.waiting_plan();used_forecast=false;}
        }
        eprintln!("g007 day={} score={:.1} iterations={} hubs={:?} beam_calls={} beam_layers={} fleet_calls={} fleet_accepted={} fleet_improved={} fleet_rescues={} triple_calls={} triple_accepted={} triple_improved={} restart_calls={} future_candidates={} future_value={:.1} coverage_guard={} guard_brands={} guard_base_brands={} guard_changed={} graph_targets={} graph_sources={} graph_ms={}",obs.day,bestv,it,best[0].hubs,beam_calls,beam_layers,fleet_calls,fleet_accepted,fleet_improved,fleet_rescues,triple_calls,triple_accepted,triple_improved,restart_calls,inspected,if inspected>0{lookahead_value}else{0.0},usize::from(coverage_guard),guard_brands,guard_base_brands,usize::from(guard_changed),g.targets,g.positions.len(),graph_millis);
        if self.future_weight==0.0&&!used_forecast&&!guard_has_future{best[1]=best[0].clone();}
        let mut brands=0u64;let mut balls=0;
        for i in 0..m{let count=predicted.masks.iter().filter(|&&mask|mask&(1<<i)!=0).count().min(self.p.spots[i].stocks as usize);if count>0{brands|=1u64<<self.p.spots[i].brand;}balls+=count;}
        self.raw_value=brands.count_ones()as f64*1500.0+balls as f64*20.0;
        if obs.day>0{self.raw_value+=(brands&!self.seen_brands).count_ones()as f64*100000.0;}
        let reserve=self.reserve_weight;self.reserve_weight=15.0;g.memo.borrow_mut().clear();
        self.canonical_value=self.evaluate(&g,obs,steps,&best[0],false,None).value;self.reserve_weight=reserve;
        self.choice_value=if coverage_guard&&inspected>0{lookahead_value}else{bestv};self.last_value=bestv;self.last=Some(plan.clone());self.cache=Some(best);self.cache_positions=g.positions.clone();self.cache_day=obs.day;self.cache_complete=true;plan
    }
}


// Only plain, owned data crosses worker boundaries; each worker owns its graph
// and RefCell caches. A snapshot is the state BEFORE executing the selected plan.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Snapshot {
    supplier:usize, game:Option<Game>, last:Option<Plan>, cache:Option<[Solution;2]>,
    cache_positions:Vec<usize>, cache_day:usize, cache_complete:bool,
    seen_brands:u64, opponent_stays:Option<(usize,Vec<f64>)>, forecast_other:Vec<f64>,
}
struct Elite {value:f64,solution:Option<Solution>}
enum Work {
    Select(u64),
    Plan(Snapshot,Observation,u32,u64,bool,Arc<Mutex<Elite>>),
}
struct ResultPacket {worker:usize, value:f64, answer:serde_json::Value, state:Snapshot}
pub struct PoolSolver {
    workers:Vec<mpsc::Sender<Work>>, results:mpsc::Receiver<ResultPacket>,
    handles:Vec<thread::JoinHandle<()>>, state:Option<Snapshot>,
}
impl Solver {
    fn snapshot(&self)->Snapshot {Snapshot{supplier:self.supplier,game:self.game.clone(),last:self.last.clone(),cache:self.cache.clone(),cache_positions:self.cache_positions.clone(),cache_day:self.cache_day,cache_complete:self.cache_complete,seen_brands:self.seen_brands,opponent_stays:self.opponent_stays.clone(),forecast_other:self.forecast_other.clone()}}
    fn restore(&mut self,s:Snapshot){
        self.supplier=s.supplier;self.game=s.game;self.last=s.last;self.cache=s.cache;
        self.cache_positions=s.cache_positions;self.cache_day=s.cache_day;self.cache_complete=s.cache_complete;
        self.seen_brands=s.seen_brands;self.opponent_stays=s.opponent_stays;self.forecast_other=s.forecast_other;
        self.selection_graph=None;self.selecting=false;
    }
    fn plan_with_rollout(&mut self,obs:&Observation,steps:u32,limit:u64,accepted:bool)->Plan {
        let began=Instant::now();
        let rollout=limit>=3500&&obs.day+1<self.p.day_steps.len();
        let first_limit=if rollout{limit*67/100}else{limit};
        let plan=self.plan(obs,steps,first_limit,accepted);
        let first=self.snapshot();let first_raw=self.raw_value;let first_canonical=self.canonical_value;
        let first_exchange=self.exchange.take();let first_cross=self.cross.take();let reserve=self.reserve_weight;
        let remaining=limit.saturating_sub(began.elapsed().as_millis()as u64+180);
        let mut chosen_value=first_canonical;let mut carry:Option<Solution>=None;
        if rollout&&remaining>=700 {
            if let Some(game)=&self.game {
                let mut next=game.clone();
                if next.play_day(plan.clone()).is_ok(){
                    let mut next_obs=next.observation(0);
                    next_obs.traffics=self.forecast_traffic(game.last_stays(),next.last_stays(),&obs.traffics);
                    // On compact shop sets, also test the loss of optimistic road recovery.
                    let scenarios=if self.p.spots.len()<=16&&remaining>=1500{2}else{1};
                    let mut worst=f64::INFINITY;let mut inspected=0;
                    for scenario in 0..scenarios {
                        let available=limit.saturating_sub(began.elapsed().as_millis()as u64+180);
                        let probe_limit=available/(scenarios-scenario)as u64;if probe_limit<500{break;}
                        self.restore(first.clone());self.reserve_weight=15.0;
                        if let (Some(cache),Some(next_solution))=(&mut self.cache,&carry){cache[1]=next_solution.clone();}
                        let mut forecast=next_obs.clone();
                        if scenario==1 {
                            for (road,observed) in forecast.traffics.iter_mut().zip(&obs.traffics){road.status=road.status.max(observed.status.min(1));}
                        }
                        let _=self.plan_inner(&forecast,self.p.day_steps[obs.day+1],probe_limit,true,false);
                        worst=worst.min(self.canonical_value);inspected+=1;
                        let mut next_solution=self.cache.as_ref().unwrap()[0].clone();
                        for hub in &mut next_solution.hubs{
                            let pos=self.cache_positions[*hub];
                            *hub=first.cache_positions.iter().position(|&p|p==pos).unwrap();
                        }
                        carry=Some(next_solution);
                    }
                    if inspected>0{
                        chosen_value=first_raw+0.75*worst+0.05*(first_canonical-first_raw);
                        eprintln!("p0 rollout day={} raw={:.3} canonical={:.3} robust_next={:.3} scenarios={} combined={:.3}",obs.day,first_raw,first_canonical,worst,inspected,chosen_value);
                    }
                }
            }
        }
        self.restore(first);
        if let (Some(cache),Some(next_solution))=(&mut self.cache,carry){cache[1]=next_solution;}
        self.choice_value=chosen_value;self.reserve_weight=reserve;self.exchange=first_exchange;self.cross=first_cross;
        plan
    }
    fn select_role(&mut self,worker:usize,limit:u64)->Vec<u8>{
        if limit<=110 {return self.select(limit);}
        let began=Instant::now();let n=self.p.agents.len();self.supplier=worker%n;
        let mut kinds=vec![0;n];kinds[self.supplier]=1;
        self.game=Some(Game::new(self.p.clone(),&kinds));
        let obs=self.game.as_ref().unwrap().observation(0);
        let rollout=self.p.day_steps.len()>1&&limit>=3000;
        let first_limit=if rollout{limit*62/100}else{limit};
        let plan=self.plan_inner(&obs,self.p.day_steps[0],first_limit,true,false);
        self.last=None;let first=self.snapshot();let first_raw=self.raw_value;let first_value=self.canonical_value;
        let remaining=limit.saturating_sub(began.elapsed().as_millis()as u64+150);
        let mut role_value=if rollout{1.75*first_raw+0.05*(first_value-first_raw)}else{first_value};
        if rollout&&remaining>=700 {
            let mut next=self.game.as_ref().unwrap().clone();
            if next.play_day(plan).is_ok(){
                let mut next_obs=next.observation(0);
                next_obs.traffics=self.forecast_traffic(self.game.as_ref().unwrap().last_stays(),next.last_stays(),&obs.traffics);
                self.game=Some(next);self.seen_brands=u64::MAX;
                self.plan_inner(&next_obs,self.p.day_steps[1],remaining,true,false);
                role_value=first_raw+0.75*self.canonical_value+0.05*(first_value-first_raw);
            }
        }
        self.restore(first);self.choice_value=role_value;
        eprintln!("p0 selection worker={} supplier={} value={:.3}",worker,self.supplier,role_value);
        kinds
    }
}
impl PoolSolver {
    pub fn new(p:Problem,seed:u64,total_threads:usize)->Self {
        assert!((2..=80).contains(&total_threads));
        let (result_tx,results)=mpsc::channel();let mut workers=Vec::new();let mut handles=Vec::new();
        for worker in 0..total_threads-1 {
            let (tx,rx)=mpsc::channel();workers.push(tx);let result_tx=result_tx.clone();let p=p.clone();
            handles.push(thread::Builder::new().name(format!("p0-search-{worker}")).spawn(move||{
                let mut solver=Solver::new(p,seed.wrapping_add((worker as u64+1)*104729),0.0,[0.8,1.2,2.0,3.0,4.0,1.6,2.4][worker%7],0.0);
                solver.temperature_scale=[0.65,0.85,1.0,1.2,1.5,1.8,2.2][worker%7];
                while let Ok(work)=rx.recv(){
                    let answer=match work {
                        Work::Select(limit)=>serde_json::json!(solver.select_role(worker,limit)),
                        Work::Plan(state,obs,steps,limit,accepted,exchange)=>{
                            solver.restore(state);solver.exchange=Some(exchange);
                            solver.reserve_weight=[4.0,8.0,12.0,15.0,20.0,30.0,45.0][(worker/2)%7];
                            let plan=solver.plan_with_rollout(&obs,steps,limit,accepted);solver.exchange=None;
                            serde_json::json!(plan)
                        }
                    };
                    if result_tx.send(ResultPacket{worker,value:solver.choice_value,answer,state:solver.snapshot()}).is_err(){break;}
                }
            }).unwrap());
        }
        Self{workers,results,handles,state:None}
    }
    fn collect(&mut self)->serde_json::Value {
        let mut best:Option<ResultPacket>=None;
        for _ in 0..self.workers.len(){
            let item=self.results.recv().expect("search worker failed");
            if best.as_ref().is_none_or(|b|item.value>b.value||(item.value==b.value&&item.worker<b.worker)){best=Some(item);}
        }
        let best=best.unwrap();eprintln!("p0 chosen worker={} value={:.3}",best.worker,best.value);
        self.state=Some(best.state);best.answer
    }
    pub fn select(&mut self,limit:u64)->serde_json::Value {
        for tx in &self.workers{tx.send(Work::Select(limit.saturating_sub(60))).unwrap();}self.collect()
    }
    pub fn plan(&mut self,obs:&Observation,steps:u32,limit:u64,accepted:bool)->serde_json::Value {
        let exchanges=(0..7).map(|_|Arc::new(Mutex::new(Elite{value:f64::NEG_INFINITY,solution:None}))).collect::<Vec<_>>();
        for (i,tx) in self.workers.iter().enumerate(){tx.send(Work::Plan(self.state.as_ref().unwrap().clone(),obs.clone(),steps,limit.saturating_sub(60),accepted,exchanges[(i/2)%7].clone())).unwrap();}self.collect()
    }
}
impl Drop for PoolSolver {
    fn drop(&mut self){self.workers.clear();for handle in self.handles.drain(..){let _=handle.join();}}
}

#[path = "../pool.rs"]
mod distributed_pool;
pub(crate) use distributed_pool::{Candidate, SearchPool, Task, fallback};

#[path="../cross_elite.rs"]
mod cross_elite;
pub(crate) use cross_elite::{CrossBus, ElitePacket, EliteCursor, ELITE_INTERVAL, ELITE_MAX_BYTES};
