use std::cmp::Reverse;
use std::cell::{Cell,RefCell};
use std::collections::BinaryHeap;
use std::rc::Rc;
use std::time::Instant;
use tester::engine::Game;
use tester::model::{Observation, Plan, Problem, Traffic};

const INF: u32 = 1_000_000_000;
const STYLES:usize=4;
const TOKEN_BASE:usize=1024;
fn target(token:i16)->usize{token as usize&(TOKEN_BASE-1)}
fn path_style(base:usize,token:i16)->usize{if token as i32>=TOKEN_BASE as i32{token as usize/TOKEN_BASE-1}else{base}}

#[derive(Clone, Default)]
struct Leg { id:u32, time: u32, fuel: u32, mask: u32, actions: Vec<i64>, hits:Vec<(usize,u32,u32,u32)>, cells:Vec<(usize,u32,u32,u32)> }

struct Network { legs: Vec<Leg>, nodes: Vec<usize>, node_at:Vec<usize>, size: usize, costs:Vec<u32>,fast_access:Vec<[u32;32]>,access_fuel:Vec<u32>,nearest_store:Vec<(u32,u32)> }
impl Network {
    fn build(p: &Problem, traffic: &[Traffic],extra:&[usize]) -> Self {
        let mut nodes: Vec<usize> = p.spots.iter().map(|s| s.pos).collect();
        for &pos in &p.agents{if !nodes.contains(&pos){nodes.push(pos);}}
        for pos in 0..p.map.len(){if p.map.terrain(pos)!=3&&(pos%p.map.width+2*(pos/p.map.width))%3==0&&!nodes.contains(&pos){nodes.push(pos);}}
        for &pos in extra{if !nodes.contains(&pos){nodes.push(pos);}}
        let size = nodes.len();let mut node_at=vec![usize::MAX;p.map.len()];for (i,&v) in nodes.iter().enumerate(){node_at[v]=i;}
        let mut costs = vec![2; p.map.len()];
        let mut fuels = vec![1; p.map.len()];
        let mut at = vec![0u32; p.map.len()];
        for (i,s) in p.spots.iter().enumerate() { at[s.pos] = 1 << i; }
        for pos in 0..p.map.len() {
            match p.map.terrain(pos) { 1 => { costs[pos]=1; fuels[pos]=2; }, 2 => { costs[pos]=3; fuels[pos]=2; }, _=>{} }
        }
        for t in traffic { costs[t.pos] = [1,2,4][t.status as usize]; }
        let mut legs = vec![Leg::default(); STYLES*size*size];
        for style in 0..STYLES {
            let (wt,wf) = [(4096,1),(8,7),(1,4096),(4096,1)][style];
            for (goal,&target) in nodes.iter().enumerate() {
                let mut dist = vec![INF;p.map.len()];
                let mut next = vec![usize::MAX;p.map.len()];
                let mut heap=BinaryHeap::new();
                dist[target]=0; heap.push(Reverse((0,target)));
                while let Some(Reverse((d,v)))=heap.pop() {
                    if d!=dist[v] { continue; }
                    for u in p.map.neighbors(v) {
                        if p.map.terrain(u)==3 {continue;}
                        let bonus=if style==3&&at[u]!=0{32}else{0};
                        let nd=d+wt*costs[u]+wf*fuels[u]-bonus;
                        if nd<dist[u] {dist[u]=nd;next[u]=v;heap.push(Reverse((nd,u)));}
                    }
                }
                for (source,&start) in nodes.iter().enumerate() {
                    let mut l=Leg::default();l.id=((style*size+source)*size+goal)as u32;let mut v=start;
                    l.mask|=at[v];
                    while v!=target {
                        let u=next[v];
                        if u==usize::MAX {l.time=INF;l.fuel=INF;break;}
                        if node_at[v]!=usize::MAX{l.hits.push((node_at[v],l.time,l.time+costs[v]-1,l.fuel));}
                        l.cells.push((v,l.time,l.time+costs[v]-1,l.fuel));
                        l.time+=costs[v]; l.fuel+=fuels[v];
                        l.actions.push((0..6).find(|&a|p.map.neighbor(v,a)==Some(u)).unwrap() as i64);
                        l.mask|=at[u];v=u;
                    }
                    l.hits.push((goal,l.time,l.time,l.fuel));
                    l.cells.push((target,l.time,l.time,l.fuel));
                    legs[(style*size+source)*size+goal]=l;
                }
            }
        }
        let mut fast_access=vec![[0;32];size];let mut access_fuel=vec![0;size];let mut nearest_store=vec![(INF,INF);size];
        for a in 0..size{for b in 0..p.spots.len(){let leg=&legs[a*size+b];fast_access[a][b]=leg.time;access_fuel[a]=access_fuel[a].max(leg.fuel);if a!=b{nearest_store[a]=nearest_store[a].min((leg.time,leg.fuel));}}}
        Self{legs,nodes,node_at,size,costs,fast_access,access_fuel,nearest_store}
    }
    fn leg(&self,style:usize,a:usize,b:usize)->&Leg{&self.legs[(style*self.size+a)*self.size+b]}
    fn node(&self,pos:usize)->usize{self.nodes.iter().position(|&v|v==pos).unwrap()}
}

#[derive(Clone)]
struct Day { routes:Vec<Vec<i16>>, styles:Vec<usize>, supply:Vec<usize>, dwell:Vec<u32>, automatic:bool, fixed:Option<Vec<(usize,u32)>>, start_wait:Vec<u32> }
#[derive(Clone)]
struct Solution { days:Vec<Day>, supplier:Option<usize>, value:f64 }
#[derive(Clone,Copy)]
struct Car { node:usize, fuel:u32 }
#[derive(Clone)]
struct PatrolResult{car:Car,mask:u32,time:u32,waste:f64,actions:Vec<i64>}
struct PatrolEntry{tag:u64,start:Car,steps:u32,delay:u32,style:usize,automatic:bool,route:Vec<i16>,result:PatrolResult}
#[derive(Clone,Copy)]
struct AccessEntry { key:u64, distances:[u32;32] }
#[derive(Clone,Copy)]
struct FuelEntry { tag:u64,key:u64,required:u32,remaining:u32,filled:bool,valid:bool }
#[derive(Clone,Copy)]
struct Stop {node:usize,cell:usize,begin:u32,end:u32}
struct Schedule{stops:Vec<Stop>,bits:Vec<[u64;5]>,tag:u64}
struct FixedSchedule{future:bool,start:usize,steps:u32,appointments:Vec<(usize,u32)>,schedule:Rc<Schedule>,end:usize,actions:Vec<i64>}
impl std::ops::Deref for Schedule{type Target=[Stop];fn deref(&self)->&[Stop]{&self.stops}}
impl Schedule{
 fn new(stops:Vec<Stop>,n:usize,tag:u64)->Self{let mut bits=vec![[0u64;5];n];for s in &stops{for t in s.begin.max(1)..=s.end.min(256){bits[s.cell][t as usize/64]|=1u64<<(t%64);}}Self{stops,bits,tag}}
 fn present(&self,node:usize,begin:u32,end:u32)->bool{let b=begin.max(1);let e=end.min(256);if b>e{return false;}let lo=b as usize/64;let hi=e as usize/64;let bits=&self.bits[node];if lo==hi{bits[lo]&((u64::MAX<<(b%64))&(u64::MAX>>(63-e%64)))!=0}else{bits[lo]&(u64::MAX<<(b%64))!=0||bits[hi]&(u64::MAX>>(63-e%64))!=0||(lo+1..hi).any(|i|bits[i]!=0)}}
}
struct Rng(u64);
impl Rng{
 fn next(&mut self)->u64{self.0^=self.0<<7;self.0^=self.0>>9;self.0^=self.0<<8;self.0}
 fn n(&mut self,n:usize)->usize{(self.next()%n as u64) as usize}
 fn f(&mut self)->f64{(self.next()>>11) as f64*(1.0/9007199254740992.0)}
}

pub struct Solver {
 problem:Problem, game:Game, net:Network, future_net:Option<Network>, future_mode:Cell<bool>, preview_traffic:Option<Vec<Traffic>>, best:Option<Solution>, rng:Rng, seen:u32,
 brands:Vec<u32>, day:usize, initial:Vec<Car>, kinds:Vec<u8>, discount:f64, terminal_scale:f64,
 access_cache:RefCell<Vec<AccessEntry>>,
 fixed_cache:RefCell<Vec<Option<Rc<FixedSchedule>>>>,
 patrol_cache:RefCell<Vec<Option<PatrolEntry>>>,patrol_recent:RefCell<Vec<usize>>,patrol_queries:Cell<u64>,patrol_hits:Cell<u64>,patrol_checks:Cell<u64>,
 fuel_cache:RefCell<Vec<FuelEntry>>,schedule_serial:Cell<u64>,fuel_queries:Cell<u64>,fuel_hits:Cell<u64>,
}
impl Solver {
 pub fn new(problem:Problem,seed:u64)->Self{
  let n=problem.agents.len();let kinds=vec![0;n];
  let game=Game::new(problem.clone(),&kinds);let net=Network::build(&problem,&[],&[]);
  let mut names:Vec<usize>=problem.spots.iter().map(|s|s.brand).collect();names.sort();names.dedup();
  let brands=problem.spots.iter().map(|s|1u32<<names.binary_search(&s.brand).unwrap()).collect();
  let initial=problem.agents.iter().map(|&pos|Car{node:net.node(pos),fuel:problem.fuel_limits}).collect();
  Self{problem,game,net,future_net:None,future_mode:Cell::new(false),preview_traffic:None,best:None,rng:Rng(seed.wrapping_add(0xa17c08de924fa321)),seen:0,brands,day:0,initial,kinds,discount:0.94,terminal_scale:0.1,access_cache:RefCell::new(vec![AccessEntry{key:u64::MAX,distances:[0;32]};16384]),fixed_cache:RefCell::new(vec![None;128]),patrol_cache:RefCell::new((0..16384).map(|_|None).collect()),patrol_recent:RefCell::new(vec![0;16]),patrol_queries:Cell::new(0),patrol_hits:Cell::new(0),patrol_checks:Cell::new(0),fuel_cache:RefCell::new(vec![FuelEntry{tag:0,key:0,required:0,remaining:0,filled:false,valid:false};65536]),schedule_serial:Cell::new(1),fuel_queries:Cell::new(0),fuel_hits:Cell::new(0)}
 }

 pub fn select(&mut self,deadline:Instant)->Vec<u8>{
  let n=self.initial.len();let start=Instant::now();let span=deadline.saturating_duration_since(start);
  self.discount=0.0;
  let mut choices:Vec<Option<usize>>=(0..n).map(Some).collect();
  if self.problem.fuel_limits>=self.problem.day_steps.iter().sum::<u32>()*2{choices.push(None);}
  let count=choices.len();let mut candidates=Vec::new();
  for (i,supplier) in choices.into_iter().enumerate(){
   self.kinds.fill(0);if let Some(a)=supplier{self.kinds[a]=1;}self.game=Game::new(self.problem.clone(),&self.kinds);
   let mut candidate=self.initialize(supplier);candidate.days.truncate(1);
   self.search(&mut candidate,start+span*11*(i+1)as u32/(20*count)as u32);candidates.push(candidate);
  }
  candidates.sort_by(|a,b|b.value.total_cmp(&a.value));
  let preview=self.problem.day_steps.len()>1&&candidates.len()>1;
  for i in (0..candidates.len().min(2)).rev(){
   self.kinds.fill(0);if let Some(a)=candidates[i].supplier{self.kinds[a]=1;}self.game=Game::new(self.problem.clone(),&self.kinds);
   let until=if preview{if i==0{start+span*3/4}else{start+span*13/20}}else if i==0{deadline}else{start+span*3/4};
   self.search(&mut candidates[i],until);
  }
  let chosen=if preview{candidates.truncate(2);self.preview_candidates(candidates,deadline)}else{candidates.into_iter().max_by(|a,b|a.value.total_cmp(&b.value)).unwrap()};
  self.kinds.fill(0);if let Some(a)=chosen.supplier{self.kinds[a]=1;}
  eprintln!("generation6 select supplier={:?} estimate={:.1}",chosen.supplier,chosen.value);
  self.game=Game::new(self.problem.clone(),&self.kinds);self.best=Some(chosen);self.discount=0.0;self.kinds.clone()
 }

 pub fn observe(&mut self,o:&Observation,previous:Option<Plan>,accepted:bool){
  if let Some(plan)=previous{
   let plan=if accepted{plan}else{self.game.waiting_plan()};
   if let Ok(replay)=self.game.play_day(plan){
    for f in replay.frames{for e in f.events{if let tester::model::Event::Collect{spot,..}=e{self.seen|=self.brands[spot];}}}
   }
  }
  if o.day>self.day { if let Some(s)=self.best.as_mut(){if s.days.len()>o.day-self.day{s.days.drain(0..o.day-self.day);}} }
  self.day=o.day;self.game.day=o.day;self.game.agents=o.agents.clone();self.game.set_observed_traffic(o.traffics.clone());
  self.net=Network::build(&self.problem,&o.traffics,&o.agents.iter().map(|a|a.pos).collect::<Vec<_>>());
  self.clear_network_caches();
  self.initial=o.agents.iter().map(|a|Car{node:self.network().node(a.pos),fuel:a.fuel}).collect();
 }

 fn network(&self)->&Network{
  if self.future_mode.get(){self.future_net.as_ref().unwrap_or(&self.net)}else{&self.net}
 }

 fn clear_network_caches(&mut self){
  for entry in self.access_cache.get_mut(){entry.key=u64::MAX;}
  self.fixed_cache.get_mut().fill(None);
  for entry in self.patrol_cache.get_mut(){*entry=None;}self.patrol_recent.get_mut().fill(0);
 }

 pub fn plan(&mut self,deadline:Instant)->Plan{
  let mut best=self.best.take().unwrap_or_else(||self.initialize(self.kinds.iter().position(|&k|k==1)));
  self.discount=0.0;best.days.truncate(1);
  best.value=self.evaluate(&best,false).0;
  let seed=best.clone();for variant in 1..4{let mut alternative=seed.clone();if variant&1!=0{for route in &mut alternative.days[0].routes{route.reverse();}}if variant&2!=0{alternative.days[0].fixed=None;alternative.days[0].automatic=true;}alternative.value=self.evaluate(&alternative,false).0;if alternative.value>best.value{best=alternative;}}
  let start=Instant::now();let remaining=deadline.saturating_duration_since(start);
  let final_day=self.day+1==self.problem.day_steps.len();
  if !final_day&&remaining.as_millis()>200{
   let joint=remaining.as_millis()>2000;let single=if joint{remaining*4/5}else{remaining};
   self.polish_day(&mut best,start+single*3/5);
   let mut conservative=best.clone();self.terminal_scale=1.0;
   self.search(&mut conservative,start+single*3/4);self.terminal_scale=0.1;
   best=self.preview_candidates(vec![best,conservative],start+single);
   if joint&&best.days.len()>1{self.joint_refine(&mut best,deadline);}
  }else{self.polish_day(&mut best,if final_day{deadline-remaining/50}else{deadline});}
  let (_,plan,_,predicted)=self.evaluate_raw(&best,true);
  let mut check=self.game.clone();
  let plan=if let Err(err)=check.play_day(plan.clone()){
   eprintln!("generation6 validation failed: {}",err.message);self.game.waiting_plan()
  }else{
   for (a,car) in predicted.iter().enumerate(){let actual=&check.agents[a];if actual.pos!=self.network().nodes[car.node]||actual.fuel!=car.fuel{eprintln!("generation6 model mismatch agent={a} predicted={}/{} actual={}/{}",self.network().nodes[car.node],car.fuel,actual.pos,actual.fuel);}}
   if final_day{self.reposition(plan,deadline)}else{plan}
  };
  eprintln!("generation6 day={} estimate={:.1} fuel_cache={}/{} patrol_cache={}/{} patrol_checks={}",self.day+1,best.value,self.fuel_hits.get(),self.fuel_queries.get(),self.patrol_hits.get(),self.patrol_queries.get(),self.patrol_checks.get());
  self.best=Some(best);plan
 }

 fn polish_day(&mut self,best:&mut Solution,deadline:Instant){
  let remaining=deadline.saturating_duration_since(Instant::now());
  self.search(best,deadline-remaining/3);
  let active:Vec<usize>=(0..self.initial.len()).filter(|&a|Some(a)!=best.supplier).collect();
  for (i,&a)in active.iter().enumerate(){let now=Instant::now();if now>=deadline{break;}let until=now+deadline.saturating_duration_since(now)/(active.len()-i)as u32;self.beam_improve(best,until,Some(a));}
  if Instant::now()<deadline{self.search(best,deadline);}
 }

 fn preview_candidates(&mut self,mut candidates:Vec<Solution>,deadline:Instant)->Solution{
  self.preview_traffic=None;
  let saved_day=self.day;let saved_initial=self.initial.clone();let saved_game=self.game.clone();let saved_seen=self.seen;let saved_kinds=self.kinds.clone();let saved_rng=self.rng.0;
  let traffic=saved_game.traffic();let templates:Vec<Day>=candidates.iter().map(|s|s.days[0].clone()).collect();
  for candidate in &mut candidates{candidate.value=self.evaluate(candidate,false).0;}
  let mut chosen=candidates.iter().max_by(|a,b|a.value.total_cmp(&b.value)).unwrap().clone();
  let mut best_value=f64::NEG_INFINITY;let mut chosen_index=None;let mut original_net=None;
  for (index,candidate)in candidates.iter().enumerate(){
   let now=Instant::now();let until=now+deadline.saturating_duration_since(now)/(candidates.len()-index)as u32;
   if let Some(net)=original_net.take(){self.net=net;self.clear_network_caches();}
   if Instant::now()>=deadline{break;}
   self.day=saved_day;self.initial=saved_initial.clone();self.game=saved_game.clone();self.seen=saved_seen;self.kinds=saved_kinds.clone();self.rng.0=saved_rng;
   self.kinds.fill(0);if let Some(a)=candidate.supplier{self.kinds[a]=1;}
   for(a,car)in self.game.agents.iter_mut().enumerate(){car.kind=self.kinds[a];}
   let (_,plan,_,end)=self.evaluate_raw(candidate,true);let mut forecast=self.game.clone();
   let Ok(replay)=forecast.play_day(plan)else{eprintln!("generation6 validation failed: preview current day");continue;};
   let current=forecast.score.clone();
   for frame in replay.frames{for event in frame.events{if let tester::model::Event::Collect{spot,..}=event{self.seen|=self.brands[spot];}}}
   let mut future_traffic=traffic.clone();
   {
    let now=forecast.last_stays();let prior=saved_game.last_stays();let numerator=if self.problem.players>1{4u64}else{5u64};
    for road in &mut future_traffic{
     let count=u64::from(now[road.pos])+u64::from(prior.get(road.pos).copied().unwrap_or(0));
     let status=if count*numerator>=u64::from(self.problem.jammed_threshold)*5{2}else if count*numerator>=u64::from(self.problem.busy_threshold)*5{1}else{0};road.status=if saved_day<2{road.status.max(status)}else{status};
    }
   }
   let changed=future_traffic.iter().zip(&traffic).filter(|(a,b)|a.status!=b.status).count();
   if changed>0{
    let began=Instant::now();let predicted=Network::build(&self.problem,&future_traffic,&self.network().nodes);
    original_net=Some(std::mem::replace(&mut self.net,predicted));self.clear_network_caches();
    eprintln!("generation6 preview_traffic day={} changed={} network_ms={}",saved_day+1,changed,began.elapsed().as_millis());
   }
   self.day=saved_day+1;self.initial=end;self.game=forecast;self.game.set_observed_traffic(future_traffic.clone());
   let mut next=self.initialize(candidate.supplier);next.days.truncate(1);
   for template in &templates{for reverse in [false,true]{
    let mut day=template.clone();day.fixed=None;day.automatic=true;day.start_wait.fill(0);
    if reverse{for route in &mut day.routes{route.reverse();}}
    let mut alternative=Solution{days:vec![day],supplier:candidate.supplier,value:0.};alternative.value=self.evaluate(&alternative,false).0;
    if alternative.value>next.value{next=alternative;}
   }}
   self.polish_day(&mut next,until);
   let (_,plan)=self.evaluate(&next,true);let mut outcome=self.game.clone();
   if outcome.play_day(plan).is_err(){eprintln!("generation6 validation failed: preview next day");continue;}
   let extra_daily=outcome.score.daily_brands-current.daily_brands;let extra_udon=outcome.score.udon-current.udon;
   let value=outcome.score.unique_brands as f64*1_000_000.+current.daily_brands as f64*10000.+current.udon as f64*100.+0.8*(extra_daily as f64*10000.+extra_udon as f64*100.);
   eprintln!("generation6 preview candidate={} today={}/{} next={}/{} value={:.1}",index+1,current.daily_brands-saved_game.score.daily_brands,current.udon-saved_game.score.udon,extra_daily,extra_udon,value);
   if value>best_value||(value==best_value&&candidate.value>chosen.value){best_value=value;chosen=candidate.clone();chosen.days.truncate(1);chosen.days.push(next.days[0].clone());chosen_index=Some(index+1);self.preview_traffic=Some(future_traffic);}
  }
  if let Some(net)=original_net{self.net=net;self.clear_network_caches();}
  self.day=saved_day;self.initial=saved_initial;self.game=saved_game;self.seen=saved_seen;self.kinds=saved_kinds;
  chosen.value=self.evaluate(&chosen,false).0;
  eprintln!("generation6 preview_choice={chosen_index:?}");chosen
 }

 fn joint_refine(&mut self,best:&mut Solution,deadline:Instant){
  let Some(traffic)=self.preview_traffic.clone()else{return;};
  if Instant::now()>=deadline{return;}
  self.future_net=Some(Network::build(&self.problem,&traffic,&self.network().nodes));
  self.clear_network_caches();self.discount=0.8;
  best.value=self.evaluate_frozen(best,false).0;let before=best.value;
  self.search(best,deadline);
  eprintln!("generation6 joint_gain={:.1}",best.value-before);
  let(_,first,_,predicted)=self.evaluate_raw(best,true);let mut check=self.game.clone();
  if check.play_day(first).is_err(){eprintln!("generation6 validation failed: joint current day");}else{
   let today=check.score.clone();let saved_day=self.day;let saved_initial=self.initial.clone();
   self.day+=1;self.initial=check.agents.iter().map(|a|Car{node:self.network().node(a.pos),fuel:a.fuel}).collect();
   self.future_mode.set(true);self.discount=0.0;
   let next=Solution{days:vec![best.days[1].clone()],supplier:best.supplier,value:0.0};
   let(_,plan)=self.evaluate(&next,true);check.set_observed_traffic(traffic);
   if check.play_day(plan).is_err(){eprintln!("generation6 validation failed: joint next day");}else{
    eprintln!("generation6 joint_outcome day={} today={}/{} next={}/{}",saved_day+1,today.daily_brands-self.game.score.daily_brands,today.udon-self.game.score.udon,check.score.daily_brands-today.daily_brands,check.score.udon-today.udon);
    for(a,car)in predicted.iter().enumerate(){let actual=&check.agents[a];if actual.pos!=self.network().nodes[car.node]||actual.fuel!=car.fuel{eprintln!("generation6 model mismatch joint agent={a} predicted={}/{} actual={}/{}",self.network().nodes[car.node],car.fuel,actual.pos,actual.fuel);}}
   }
   self.day=saved_day;self.initial=saved_initial;self.future_mode.set(false);
  }
  self.discount=0.0;self.future_net=None;self.clear_network_caches();best.value=self.evaluate(best,false).0;
 }

 fn reposition(&self,mut plan:Plan,deadline:Instant)->Plan{
  let mut game=self.game.clone();let Ok(mut replay)=game.play_day(plan.clone())else{return plan;};let future=self.day+1<self.problem.day_steps.len();
  let value=|g:&Game|{
   let mut score=g.score.unique_brands as f64*1_000_000.+g.score.daily_brands as f64*10000.+g.score.udon as f64*100.;
   if future{
    let cars:Vec<Car>=g.agents.iter().map(|a|Car{node:self.network().node_at[a.pos],fuel:a.fuel}).collect();
    score+=self.terminal_value(&cars,self.kinds.iter().position(|&k|k==1),self.problem.day_steps[self.day+1]);
   }
   score
  };
  let mut best=value(&game);let steps=self.problem.day_steps[self.day];
  for a in 0..self.initial.len(){if Instant::now()>=deadline{break;}if self.kinds[a]!=0{continue;}let Some(&wait)=plan[a].last()else{continue;};if wait>=-1{continue;}
   let duration=(-wait)as u32;let starts=steps-duration;let pos=game.agents[a].pos;let Some(source)=self.network().nodes.iter().position(|&v|v==pos)else{continue;};
   let mut offsets=vec![0];
   for f in replay.frames.iter().skip(starts as usize+1){if f.events.iter().any(|e|matches!(e,tester::model::Event::Refill{agent,..}if *agent==a)){offsets.push(f.step-starts);}}
   offsets.sort();offsets.dedup();
   let mut chosen=None;
   'options: for offset in offsets{for target in 0..self.problem.spots.len(){for style in 0..STYLES{
    if Instant::now()>=deadline{break 'options;}
    let leg=self.network().leg(style,source,target);let mut at=pos;let mut used=offset;let mut fuel=replay.frames[(starts+offset)as usize].agents[a].fuel;let mut actions=Vec::new();
    for &direction in &leg.actions{let time=self.network().costs[at];let cost=if self.problem.map.terrain(at)==0{1}else{2};if used+time>duration||cost>fuel{break;}actions.push(direction);used+=time;fuel-=cost;at=self.problem.map.neighbor(at,direction as usize).unwrap();}
    if actions.is_empty(){continue;}
    let mut candidate=plan.clone();candidate[a].pop();if offset>0{candidate[a].push(-(offset as i64));}candidate[a].extend(actions);if used<duration{candidate[a].push(-((duration-used)as i64));}
    let mut g=self.game.clone();let Ok(r)=g.play_day(candidate.clone())else{continue;};
    if (g.score.unique_brands,g.score.daily_brands,g.score.udon)<(game.score.unique_brands,game.score.daily_brands,game.score.udon){continue;}
    if future&&(g.score==game.score||g.agents.iter().any(|a|self.network().node_at[a.pos]==usize::MAX)){continue;}
    let v=value(&g);if v>best+0.0001{best=v;chosen=Some((candidate,g,r));}
   }}}
   if let Some((p,g,r))=chosen{eprintln!("generation6 tail agent={a} udon_gain={} future={future}",g.score.udon-game.score.udon);plan=p;game=g;replay=r;}
  }
  plan
 }

 fn initialize(&mut self,supplier:Option<usize>)->Solution{
  let m=self.problem.spots.len();let n=self.initial.len();let mut days=Vec::new();
  let center=(0..m).min_by_key(|&s|(0..m).map(|t|self.network().leg(0,s,t).time).sum::<u32>()).unwrap();
  let mut ends:Vec<usize>=self.initial.iter().map(|c|c.node).collect();
  for _ in self.day..self.problem.day_steps.len(){
   let mut routes=vec![Vec::new();n];let mut taken=vec![false;m];
   for _ in 0..m{
    let mut choice=None;let mut value=f64::INFINITY;
    for a in 0..n{if Some(a)==supplier{continue;}
     for s in 0..m{if taken[s]{continue;}let v=self.network().leg(0,ends[a],s).time as f64+routes[a].len() as f64*3.0;
      if v<value{value=v;choice=Some((a,s));}
     }
    }
    if let Some((a,s))=choice{routes[a].push(s as i16);ends[a]=s;taken[s]=true;}
   }
   for a in 0..n{if Some(a)!=supplier && supplier.is_some(){routes[a].push(-1);}}
   days.push(Day{routes,styles:vec![0;n],supply:vec![center],dwell:vec![8],automatic:true,fixed:None,start_wait:vec![0;n]});
  }
  let mut s=Solution{days,supplier,value:0.};s.value=self.evaluate(&s,false).0;s
 }

 fn automatic_supply(&self,day:&Day,cars:&[Car],supplier:usize,steps:u32,calendar_day:usize)->(Vec<(usize,u32)>,Vec<u32>){
  let n=cars.len();let mut visits=vec![Vec::new();n];let mut totals=vec![0u32;n];let mut extended=vec![usize::MAX;n];
  for a in 0..n{if a==supplier{continue;}
   let mut pos=cars[a].node;let mut t=0;let mut f=0;
   for &token in &day.routes[a]{if token<=-2{let delay=((-token-1) as u32).min(steps-t);visits[a].push((pos,t.max(1),t+delay,f));t+=delay;continue;}if token<0{continue;}let goal=target(token);let leg=self.network().leg(path_style(day.styles[a],token),pos,goal);
    if t+leg.time>steps{continue;}
    for &(node,b,e,prefix) in &leg.hits{visits[a].push((node,(t+b).max(1),t+e,f+prefix));}
    t+=leg.time;f+=leg.fuel;pos=goal;
   }
   visits[a].push((pos,t.max(1),steps,f));totals[a]=f;
   if cars[a].fuel*2<self.problem.fuel_limits{extended[a]=visits[a].len();visits[a].push((cars[a].node,1,steps,0));}
  }
  let mut appointments=Vec::new();let mut pos=cars[supplier].node;let mut time=0;
  let mut allowance:Vec<u32>=cars.iter().map(|c|c.fuel).collect();let mut last=vec![0;n];let mut delays=vec![0;n];
  for visit_index in 0..n*2{
   let mut choice=None;let mut best=0.0;
   for a in 0..n{if a==supplier{continue;}
    let deficit=totals[a].saturating_sub(allowance[a]);
    let reserve=if calendar_day+1<self.problem.day_steps.len(){self.problem.fuel_limits.min(self.problem.day_steps[calendar_day+1])/2}else{0};
    let future_deficit=totals[a].saturating_add(reserve).saturating_sub(allowance[a]);
    let priority=if day.styles[supplier]>0{if day.supply.get(visit_index).is_some_and(|&v|v%n==a){if day.styles[supplier]==1{100.0}else{10.0}}else{1.0}}else{1.0+day.supply.iter().position(|&v|v%n==a).map(|i|1.0/(i+1) as f64).unwrap_or(0.0)};
    for &(node,b,e,prefix) in &visits[a]{
     if prefix>allowance[a] || prefix<last[a]{continue;}
     let leg=self.network().leg(0,pos,node);let meet=(time+leg.time).max(b).max(1);
     if meet>e || meet>steps {continue;}
     let refill=self.problem.fuel_limits.saturating_sub(allowance[a]-prefix);
     if refill<4{continue;}
     let value=refill as f64*(if deficit>0{12.0+deficit as f64*0.2}else if future_deficit>0{4.0+future_deficit as f64*0.1}else{1.0})*priority/(meet-time+6) as f64;
     if value>best{best=value;choice=Some((a,node,meet,prefix));}
    }
   }
   let Some((a,node,meet,prefix))=choice else{break;};
   if prefix==0&&node==cars[a].node&&meet>delays[a]{let shift=meet-delays[a];delays[a]=meet;for (_,b,e,_) in &mut visits[a]{*b+=shift;*e=(*e+shift).min(steps);}}
   appointments.push((node,meet));
   pos=node;time=meet;allowance[a]=prefix+self.problem.fuel_limits;last[a]=prefix;
   for b in 0..n{if b==supplier||b==a{continue;}for (i,&(at,begin,end,used)) in visits[b].iter().enumerate(){if i!=extended[b]&&at==node&&begin<=meet&&meet<=end&&used<=allowance[b]{allowance[b]=allowance[b].max(used+self.problem.fuel_limits);last[b]=last[b].max(used);}}}
   if time==steps{break;}
  }
  (appointments,delays)
 }

 fn path_stops(&self,leg:&Leg,time:u32,stops:&mut Vec<Stop>){
  for &(cell,begin,end,_) in &leg.cells{stops.push(Stop{node:self.network().node_at[cell],cell,begin:(time+begin).max(1),end:time+end});}
 }

 fn make_schedule(&self,stops:Vec<Stop>)->Rc<Schedule>{
  let tag=self.schedule_serial.get();self.schedule_serial.set(tag+1);Rc::new(Schedule::new(stops,self.problem.map.len(),tag))
 }

 fn cached_supply(&self,appointments:&[(usize,u32)],start:usize,steps:u32)->Rc<FixedSchedule>{
  let future=self.future_mode.get();let mut hash=start as u64^((steps as u64)<<32)^((future as u64)<<63);for &(node,time) in appointments{hash=(hash^(node as u64|((time as u64)<<16))).wrapping_mul(0x9e3779b97f4a7c15);}
  let slot=((hash^(hash>>32))as usize)&127;
  if let Some(entry)=self.fixed_cache.borrow()[slot].as_ref(){if entry.future==future&&entry.start==start&&entry.steps==steps&&entry.appointments==appointments{return Rc::clone(entry);}}
  let(stops,end,actions)=self.fixed_supply(appointments,start,steps,true);
  let entry=Rc::new(FixedSchedule{future,start,steps,appointments:appointments.to_vec(),schedule:self.make_schedule(stops),end,actions});
  self.fixed_cache.borrow_mut()[slot]=Some(Rc::clone(&entry));entry
 }

 fn fixed_supply(&self,appointments:&[(usize,u32)],start:usize,steps:u32,emit:bool)->(Vec<Stop>,usize,Vec<i64>){
  let mut stops=Vec::new();let mut actions=Vec::new();let mut pos=start;let mut time=0;
  for &(node,target_time) in appointments{
   let leg=self.network().leg(0,pos,node);if time+leg.time>steps{break;}
   self.path_stops(leg,time,&mut stops);
   let meet=(time+leg.time).max(target_time).min(steps);
   if emit{actions.extend(&leg.actions);if meet>time+leg.time{actions.push(-((meet-time-leg.time) as i64));}}
   stops.push(Stop{node,cell:self.network().nodes[node],begin:(time+leg.time).max(1),end:meet});time=meet;pos=node;
  }
  stops.push(Stop{node:pos,cell:self.network().nodes[pos],begin:time.max(1),end:steps});if emit&&time<steps{actions.push(-((steps-time) as i64));}
  (stops,pos,actions)
 }

 fn supply_plan(&self,day:&Day,start:usize,steps:u32,emit:bool)->(Vec<Stop>,usize,Vec<i64>){
  let mut stops=Vec::new();let mut pos=start;let mut time=0;let mut actions=Vec::new();
  for (i,&target) in day.supply.iter().enumerate(){
   let leg=self.network().leg(0,pos,target);
   let dwell=if i==0{day.dwell.first().copied().unwrap_or(0).min(steps-time)}else{day.dwell.get(i).copied().unwrap_or(8).min(steps-time)};
   stops.push(Stop{node:pos,cell:self.network().nodes[pos],begin:time.max(1),end:time+dwell});
   if dwell>0 && emit{actions.push(-(dwell as i64));}time+=dwell;
   if time+leg.time>steps{break;}
   self.path_stops(leg,time,&mut stops);
   if emit{actions.extend(&leg.actions);}time+=leg.time;pos=target;
  }
  stops.push(Stop{node:pos,cell:self.network().nodes[pos],begin:time.max(1),end:steps});
  if emit && time<steps{actions.push(-((steps-time) as i64));}
  (stops,pos,actions)
 }

 fn travel_fuel(&self,car:Car,time:u32,leg:&Leg,stops:&Schedule)->Option<u32>{
  if stops.is_empty(){return car.fuel.checked_sub(leg.fuel);}
  self.fuel_queries.set(self.fuel_queries.get()+1);
  let key=(leg.id as u64)<<9|time as u64;let hash=(key^stops.tag.wrapping_mul(0x9e3779b97f4a7c15)).wrapping_mul(0xbf58476d1ce4e5b9);let slot=((hash^(hash>>32))as usize)&65535;
  let mut memo=self.fuel_cache.borrow_mut();let mut entry=memo[slot];
  if entry.tag==stops.tag&&entry.key==key{self.fuel_hits.set(self.fuel_hits.get()+1);}else{
   entry=FuelEntry{tag:stops.tag,key,required:leg.fuel,remaining:0,filled:false,valid:true};let mut consumed=0;
   for &(cell,begin,end,prefix) in &leg.cells{
    if stops.present(cell,time+begin,time+end){
     if !entry.filled{entry.required=prefix;entry.filled=true;}else if prefix-consumed>self.problem.fuel_limits{entry.valid=false;break;}
     consumed=prefix;
    }
   }
   if entry.filled&&entry.valid{let tail=leg.fuel-consumed;if tail>self.problem.fuel_limits{entry.valid=false;}else{entry.remaining=self.problem.fuel_limits-tail;}}
   memo[slot]=entry;
  }
  if !entry.valid||car.fuel<entry.required{None}else if entry.filled{Some(entry.remaining)}else{car.fuel.checked_sub(entry.required)}
 }

 fn rendezvous(&self,car:Car,time:u32,goal:Option<usize>,style:usize,stops:&Schedule,steps:u32)->Option<(usize,u32)>{
  let mut best=None;let mut cost=INF;
  for (k,s) in stops.iter().enumerate(){
   if s.node==usize::MAX{continue;}
   let l=self.network().leg(style,car.node,s.node);
   if self.travel_fuel(car,time,l,stops).is_none(){continue;}let meet=(time+l.time).max(s.begin);
   if meet>s.end || meet>steps{continue;}
   let after=goal.map(|g|self.network().leg(style,s.node,g)).map(|l|(l.time,l.fuel)).unwrap_or((0,0));
   if meet+after.0>steps || after.1>self.problem.fuel_limits{continue;}
   let c=meet+after.0;
   if c<cost{cost=c;best=Some((k,meet));}
  }
  best
 }

 fn evaluate(&self,s:&Solution,emit:bool)->(f64,Plan){let(value,plan,_,_)=self.evaluate_raw(s,emit);(value,plan)}

 fn evaluate_frozen(&self,s:&mut Solution,emit:bool)->(f64,Plan){
  let(value,plan,cache,_)=self.evaluate_raw(s,emit);for(d,appointments,delays)in cache{s.days[d].fixed=Some(appointments);s.days[d].start_wait=delays;}(value,plan)
 }

 fn next_access(&self,car:Car,supplier:Option<Car>,goal:usize,steps:u32)->u32{
  let fastest=self.network().leg(0,car.node,goal);if fastest.fuel<=car.fuel{return fastest.time.min(steps*2);}
  let mut best=steps*2;
  let wait=supplier.map(|s|self.network().leg(0,s.node,car.node).time.max(1)).unwrap_or(INF);
  for style in 0..3{
   let leg=self.network().leg(style,car.node,goal);
   if leg.fuel<=car.fuel{best=best.min(leg.time);}
   if leg.fuel<=self.problem.fuel_limits{best=best.min(wait.saturating_add(leg.time));}
  }
  if let Some(supplier)=supplier{
   let meet=(0..3).filter_map(|style|{let leg=self.network().leg(style,car.node,supplier.node);(leg.fuel<=car.fuel).then_some(leg.time.max(1))}).min();
   if let Some(meet)=meet{for style in 0..3{let leg=self.network().leg(style,supplier.node,goal);if leg.fuel<=self.problem.fuel_limits{best=best.min(meet+leg.time);}}}
  }
  best
 }

 fn access_row(&self,car:Car,supplier:Option<Car>,steps:u32)->[u32;32]{
  if car.fuel>=self.network().access_fuel[car.node]{let mut row=self.network().fast_access[car.node];for time in &mut row{*time=(*time).min(steps*2);}return row;}
  let key=(car.node as u64)<<42|(car.fuel as u64)<<10|supplier.map(|c|c.node as u64).unwrap_or(1023)|(steps as u64)<<52|(self.future_mode.get()as u64)<<63;
  let hash=key.wrapping_mul(0x9e3779b97f4a7c15);let slot=((hash^(hash>>32))as usize)&16383;
  let entry=self.access_cache.borrow()[slot];if entry.key==key{return entry.distances;}
  let mut distances=[0;32];for (spot,distance) in distances.iter_mut().enumerate().take(self.problem.spots.len()){*distance=self.next_access(car,supplier,spot,steps);}
  self.access_cache.borrow_mut()[slot]=AccessEntry{key,distances};distances
 }

 fn patrol(&self,day:&Day,a:usize,start:Car,stops:&Schedule,steps:u32,delay:u32,emit:bool)->PatrolResult{
  let recent=a+if self.future_mode.get(){8}else{0};let mut slot=0;let mut expected=None;
  if !emit{
   self.patrol_queries.set(self.patrol_queries.get()+1);
   let matches=|e:&PatrolEntry|e.tag==stops.tag&&e.start.node==start.node&&e.start.fuel==start.fuel&&e.steps==steps&&e.delay==delay&&e.style==day.styles[a]&&e.automatic==day.automatic&&e.route==day.routes[a];
   let table=self.patrol_cache.borrow();slot=self.patrol_recent.borrow()[recent];
   let mut cached=table[slot].as_ref().filter(|e|matches(e));
   if cached.is_none(){
    let mut hash=stops.tag.wrapping_mul(0x9e3779b97f4a7c15)^(start.node as u64)<<32^(start.fuel as u64)<<16^(steps as u64)<<8^delay as u64^day.styles[a]as u64;
    for &v in &day.routes[a]{hash=(hash^v as u64).wrapping_mul(0xbf58476d1ce4e5b9);}
    slot=((hash^(hash>>32))as usize)&16383;
    cached=table[slot].as_ref().filter(|e|matches(e));
   }
   if let Some(e)=cached{
    self.patrol_recent.borrow_mut()[recent]=slot;self.patrol_hits.set(self.patrol_hits.get()+1);
    if self.patrol_hits.get()%4096!=0{return e.result.clone();}
    expected=Some(e.result.clone());
   }
  }
    let mut waste=0.;let style=day.styles[a];let mut car=start;let mut time=delay;let mut mask=if car.node<self.problem.spots.len(){1u32<<car.node}else{0};
    let mut actions=Vec::new();if time>0{if emit{actions.push(-(time as i64));}if stops.iter().any(|s|s.node==car.node&&s.begin<=time&&s.end>=1){car.fuel=self.problem.fuel_limits;}}
    for &token in &day.routes[a]{
     let style=path_style(style,token);
     if token<=-2{
      let delay=((-token-1) as u32).min(steps-time);
      if stops.iter().any(|s|s.node==car.node&&s.begin<=time+delay&&s.end>=time.max(1)){car.fuel=self.problem.fuel_limits;}
      if emit&&delay>0{actions.push(-(delay as i64));}time+=delay;continue;
     }
     if day.automatic && token<0{continue;}
     let target=if token>=0{Some(target(token))}else{None};
     let need=target.map(|v|self.travel_fuel(car,time,self.network().leg(style,car.node,v),&stops).is_none()).unwrap_or(true);
     if need{
      if let Some((k,meet))=self.rendezvous(car,time,target,style,&stops,steps){
       let stop=&stops[k];let leg=self.network().leg(style,car.node,stop.node);
       if emit{actions.extend(&leg.actions);if meet>time+leg.time{actions.push(-((meet-time-leg.time) as i64));}}
       mask|=leg.mask;car.node=stop.node;car.fuel=self.problem.fuel_limits;time=meet;
      }else {waste+=8.0;continue;}
     }
     if let Some(target)=target{
      let leg=self.network().leg(style,car.node,target);
      if time+leg.time>steps{waste+=8.0;continue;}
      let Some(fuel)=self.travel_fuel(car,time,leg,&stops) else{waste+=8.0;continue;};
      if target<self.problem.spots.len()&&mask & (1u32<<target)!=0{waste+=2.0;}
      if emit{actions.extend(&leg.actions);}mask|=leg.mask;time+=leg.time;car.node=target;car.fuel=fuel;
     }
    }
    if emit && time<steps{actions.push(-((steps-time) as i64));}
    if stops.iter().any(|s|s.node==car.node&&s.begin<=steps&&s.end>=time.max(1)){car.fuel=self.problem.fuel_limits;}

  let result=PatrolResult{car,mask,time,waste,actions};
  if let Some(e)=&expected{
   self.patrol_checks.set(self.patrol_checks.get()+1);
   if e.car.node!=result.car.node||e.car.fuel!=result.car.fuel||e.mask!=result.mask||e.time!=result.time||e.waste!=result.waste{eprintln!("generation6 model mismatch patrol cache");}
  }
  if !emit&&expected.is_none(){self.patrol_cache.borrow_mut()[slot]=Some(PatrolEntry{tag:stops.tag,start,steps,delay,style:day.styles[a],automatic:day.automatic,route:day.routes[a].clone(),result:result.clone()});self.patrol_recent.borrow_mut()[recent]=slot;}
  result
 }

 fn evaluate_raw(&self,s:&Solution,emit:bool)->(f64,Plan,Vec<(usize,Vec<(usize,u32)>,Vec<u32>)>,Vec<Car>){
  let saved_mode=self.future_mode.get();let mut cars=self.initial.clone();let mut global=self.seen;let mut daily=0f64;let mut udon=0f64;let mut waste=0f64;
  let mut first_plan=vec![Vec::new();cars.len()];let mut residual=0f64;let mut slack=0f64;let mut mobility=0f64;
  let horizon=if self.discount<0.9{if self.discount>0.01{s.days.len().min(2)}else{1}}else{s.days.len()};let mut cache=Vec::new();
  for (d,day) in s.days.iter().take(horizon).enumerate(){
   self.future_mode.set(saved_mode||d>0);
   let steps=self.problem.day_steps[self.day+d];let want=emit&&d==0;
   let (stops,supply_end,supply_actions,delays)=if let Some(a)=s.supplier{
    if let Some(fixed)=&day.fixed{
     let built=self.cached_supply(fixed,cars[a].node,steps);
     (Rc::clone(&built.schedule),built.end,if want{built.actions.clone()}else{Vec::new()},day.start_wait.clone())
    }else if day.automatic{
     let (appointments,delays)=self.automatic_supply(day,&cars,a,steps,self.day+d);
     let built=self.cached_supply(&appointments,cars[a].node,steps);cache.push((d,appointments,delays.clone()));
     (Rc::clone(&built.schedule),built.end,if want{built.actions.clone()}else{Vec::new()},delays)
    }else{
     let(stops,end,actions)=self.supply_plan(day,cars[a].node,steps,want);
     (self.make_schedule(stops),end,actions,vec![0;cars.len()])
    }
   }else{(self.make_schedule(Vec::new()),0,Vec::new(),vec![0;cars.len()])};
   let mut collected=vec![0u32;self.problem.spots.len()];
   for a in 0..cars.len(){
    if Some(a)==s.supplier{cars[a].node=supply_end;if want{first_plan[a]=supply_actions.clone();}continue;}
    let result=self.patrol(day,a,cars[a],&stops,steps,delays[a],want);
    let mut mask=result.mask;while mask!=0{let b=mask.trailing_zeros()as usize;mask&=mask-1;collected[b]+=1;}
    slack+=(steps-result.time)as f64*self.discount.powi(d as i32);waste+=result.waste;
    cars[a]=result.car;if want{first_plan[a]=result.actions;}
   }
   let mut brands=0u32;
   let discount=self.discount.powi(d as i32);
   for (i,&v) in collected.iter().enumerate(){if v>0{brands|=self.brands[i];udon+=v.min(self.problem.spots[i].stocks) as f64*discount;}}
   global|=brands;daily+=brands.count_ones() as f64*discount;
   if self.day+d+1<self.problem.day_steps.len(){mobility+=discount*self.mobility_loss(&cars,s.supplier,self.problem.day_steps[self.day+d+1]);}
   // A small smooth reward helps the search preserve fuel for future route repairs.
   if d+1<s.days.len(){residual+=discount*cars.iter().enumerate().filter(|(a,_)|Some(*a)!=s.supplier).map(|(_,c)|(c.fuel as f64).sqrt()).sum::<f64>();}
  }
  let fuel=cars.iter().enumerate().filter(|(a,_)|Some(*a)!=s.supplier).map(|(_,c)|c.fuel as f64).sum::<f64>();
  let mut potential=0.;
  if self.discount<0.9&&self.day+horizon<self.problem.day_steps.len(){
   potential=self.terminal_value(&cars,s.supplier,self.problem.day_steps[self.day+horizon]);if horizon>1{potential*=0.1;}
  }
  let value=global.count_ones() as f64*1_000_000. + daily*10000. + udon*100. + residual*0.5 + fuel*0.015+slack*0.1+potential*self.discount.powi(horizon as i32-1)-waste-mobility;
  self.future_mode.set(saved_mode);
  (value,first_plan,cache,cars)
 }

 fn mobility_loss(&self,cars:&[Car],supplier:Option<usize>,next_steps:u32)->f64{
  let mut loss=0.;
  for (a,&car)in cars.iter().enumerate(){if Some(a)==supplier{continue;}
   let(direct,needed)=self.network().nearest_store[car.node];if direct==INF||car.fuel>=needed{continue;}
   let row=self.access_row(car,supplier.map(|a|cars[a]),next_steps);let mut reachable=INF;
   for (spot,&time)in row.iter().enumerate().take(self.problem.spots.len()){if spot!=car.node{reachable=reachable.min(time);}}
   loss+=reachable.saturating_sub(direct).saturating_sub(next_steps/8)as f64*8.0;
  }
  loss
 }

 fn terminal_value(&self,cars:&[Car],supplier:Option<usize>,next_steps:u32)->f64{
  let mut potential=0.;let mut shortage=0.;let reserve=self.problem.fuel_limits.min(next_steps)/2;let fuel_price=if self.problem.fuel_limits<=next_steps{30.0}else{8.0};
  for (a,c) in cars.iter().enumerate(){if Some(a)!=supplier{potential+=20.0*(c.fuel as f64+1.0).sqrt();let penalty=reserve.saturating_sub(c.fuel)as f64*fuel_price;potential-=penalty;shortage+=penalty;if let Some(b)=supplier{potential-=self.network().leg(0,cars[b].node,c.node).time as f64*12.0/(c.fuel as f64+8.0).sqrt();}}}
  let mut nearest=[INF;32];for (a,&car) in cars.iter().enumerate(){if Some(a)!=supplier{let row=self.access_row(car,supplier.map(|a|cars[a]),next_steps);for spot in 0..self.problem.spots.len(){nearest[spot]=nearest[spot].min(row[spot]);}}}
  let mut access=[INF;32];for spot in 0..self.problem.spots.len(){let distance=nearest[spot];potential-=distance as f64*1.5;let b=self.brands[spot].trailing_zeros()as usize;access[b]=access[b].min(distance);}
  for &distance in &access{if distance<INF{potential-=(distance as f64-next_steps as f64*0.45).max(0.).powi(2)*0.8;}}
  potential*self.terminal_scale-shortage*(1.0-self.terminal_scale)
 }

 fn search(&mut self,best:&mut Solution,deadline:Instant){
  if best.days.is_empty(){return;}
  best.value=self.evaluate_frozen(best,false).0;
  let mut cur=best.clone();let start=Instant::now();let total=deadline.saturating_duration_since(start).as_secs_f64();
  let mut rebuild_ms=0;let rebuild_limit_ms=(total*1000.0*0.35)as u128;
  if self.discount<0.9{let mut rebuilt=best.clone();let began=Instant::now();let until=(began+std::time::Duration::from_secs_f64(total*0.20)).min(deadline);self.rebuild(&mut rebuilt,until,true);rebuilt.value=self.evaluate_frozen(&mut rebuilt,false).0;rebuild_ms+=began.elapsed().as_millis();if rebuilt.value>best.value{*best=rebuilt.clone();cur=rebuilt;}}
  let mut iteration=0u64;let mut meeting_attempts=0u64;let mut meeting_improvements=0u64;let mut paired=false;let m=self.problem.spots.len();let n=self.initial.len();
  while Instant::now()<deadline{
   let elapsed=start.elapsed().as_secs_f64();let progress=(elapsed/total.max(0.001)).min(1.0);
   let wave=(progress*5.0).fract();
   let temperature=if wave<0.2{10000.0*(1.0-wave/0.2)+250.0}else{250.0*((1.0-wave)/0.8).powi(3)+1.0};
   for _ in 0..128{
    let mut sampled_meeting=false;let mut next=cur.clone();let horizon=if self.discount<0.9{if self.discount>0.01{next.days.len().min(2)}else{1}}else{next.days.len()};let d=self.rng.n(horizon);let a=self.rng.n(n);let kind=self.rng.n(100);
    if Some(a)==next.supplier && kind<85{
     if (d==0||next.days[d].fixed.is_some())&&self.rng.n(4)!=0{
      if next.days[d].fixed.is_none(){let schedule=self.automatic_supply(&next.days[d],&self.initial,a,self.problem.day_steps[self.day],self.day);next.days[d].fixed=Some(schedule.0);next.days[d].start_wait=schedule.1;}
      let steps=self.problem.day_steps[self.day+d];let k=self.rng.n(7);
      let meeting=if k==3&&d==0&&next.days[d].fixed.as_ref().is_some_and(|v|v.len()<n*2)&&self.rng.n(4)!=0{self.sample_meeting(&next,steps)}else{None};
      sampled_meeting=meeting.is_some();if sampled_meeting{meeting_attempts+=1;}
      let day=&mut next.days[d];let fixed=day.fixed.as_mut().unwrap();
      match k{
       0 if !fixed.is_empty()=>{let i=self.rng.n(fixed.len());let node=self.rng.n(self.network().size);if self.network().leg(0,fixed[i].0,node).time<=10{fixed[i].0=node;}},
       1 if !fixed.is_empty()=>{let i=self.rng.n(fixed.len());fixed[i].1=(fixed[i].1 as i32+self.rng.n(17)as i32-8).clamp(1,steps as i32)as u32;},
       2 if !fixed.is_empty()=>{let i=self.rng.n(fixed.len());fixed.remove(i);},
       3 if fixed.len()<n*2=>{let(node,t)=meeting.unwrap_or_else(||{let b=self.rng.n(n);let node=if self.rng.n(3)==0{self.initial[b].node}else{self.rng.n(m)};(node,1+self.rng.n(steps as usize)as u32)});let i=fixed.iter().position(|v|v.1>=t).unwrap_or(fixed.len());fixed.insert(i,(node,t));},
       4 if fixed.len()>1=>{let i=self.rng.n(fixed.len()-1);let node=fixed[i].0;fixed[i].0=fixed[i+1].0;fixed[i+1].0=node;},
       5=>{let b=self.rng.n(n);day.start_wait[b]=if self.rng.n(3)==0{0}else{(day.start_wait[b]as i32+self.rng.n(17)as i32-8).clamp(0,steps as i32)as u32};},
       _ if !fixed.is_empty()=>{let i=self.rng.n(fixed.len());let offset=self.rng.n(17)as i32-8;for v in &mut fixed[i..]{v.1=(v.1 as i32+offset).clamp(1,steps as i32)as u32;}},
       _=>{}
      }
     }else{
     let day=&mut next.days[d];let k=self.rng.n(7);day.fixed=None;
     match k{
      0 if day.supply.len()<4=>{let i=self.rng.n(day.supply.len()+1);day.supply.insert(i,self.rng.n(m));day.dwell.insert(i,self.rng.n(24) as u32);},
      1 if !day.supply.is_empty()=>{let i=self.rng.n(day.supply.len());day.supply.remove(i);day.dwell.remove(i);},
      2 if !day.supply.is_empty()=>{let i=self.rng.n(day.supply.len());day.supply[i]=self.rng.n(m);},
      5=>{day.styles[a]=self.rng.n(3);},
      6=>{day.automatic=!day.automatic;},
      _ if !day.dwell.is_empty()=>{let i=self.rng.n(day.dwell.len());day.dwell[i]=self.rng.n((self.problem.day_steps[self.day+d]/2+1) as usize) as u32;},
      _=>{}
     }
     }
    }else{
     let a=if Some(a)==next.supplier{(a+1)%n}else{a};
     let len=next.days[d].routes[a].len();
     if kind<25 && len<36{
      let token=if next.supplier.is_some()&&self.rng.n(6)==0{if self.rng.n(2)==0{self.rng.n(self.network().size)as i16}else{-2-self.rng.n(16) as i16}}else{self.rng.n(m) as i16};
      if token<0||!next.days[d].routes[a].iter().any(|&v|v>=0&&target(v)==target(token)){let i=if token>=0&&d==0&&self.rng.n(4)!=0{self.cheapest_insertion(&next.days[d],a,token)}else{self.rng.n(len+1)};next.days[d].routes[a].insert(i,token);}
     }else if kind<43 && len>0{let i=self.rng.n(len);next.days[d].routes[a].remove(i);
     }else if kind<65&&len>1{let i=self.rng.n(len);let j=self.rng.n(len);if self.rng.n(2)==0{next.days[d].routes[a].swap(i,j);}else{next.days[d].routes[a][i.min(j)..=i.max(j)].reverse();}
     }else if kind<82&&len>0{
      let b=self.rng.n(n);if Some(b)!=next.supplier{
       let i=self.rng.n(len);
       if a!=b&&self.rng.n(3)==0{
        let j=self.rng.n(next.days[d].routes[b].len()+1);let tail_a=next.days[d].routes[a].split_off(i);let tail_b=next.days[d].routes[b].split_off(j);next.days[d].routes[a].extend(tail_b);next.days[d].routes[b].extend(tail_a);
       }else if a!=b && !next.days[d].routes[b].is_empty() && self.rng.n(2)==0{
        let j=self.rng.n(next.days[d].routes[b].len());let x=next.days[d].routes[a][i];next.days[d].routes[a][i]=next.days[d].routes[b][j];next.days[d].routes[b][j]=x;
       }else{
        let token=next.days[d].routes[a].remove(i);let j=if token>=0&&d==0&&self.rng.n(4)!=0{self.cheapest_insertion(&next.days[d],b,token)}else{self.rng.n(next.days[d].routes[b].len()+1)};next.days[d].routes[b].insert(j,token);
       }
      }
     }else if kind<89{next.days[d].styles[a]=self.rng.n(STYLES);
     }else if kind<96&&len>0{
      let i=self.rng.n(len);if next.days[d].routes[a][i]>=0&&self.rng.n(2)==0{next.days[d].routes[a][i]=(target(next.days[d].routes[a][i])+TOKEN_BASE*(1+self.rng.n(STYLES)))as i16;}else{next.days[d].routes[a][i]=if next.supplier.is_some()&&self.rng.n(6)==0{if self.rng.n(2)==0{self.rng.n(self.network().size)as i16}else{-2-self.rng.n(16) as i16}}else{self.rng.n(m) as i16};}
     }else if next.days.len()>1{
      if self.rng.n(2)==0 {
       let e=self.rng.n(next.days.len());next.days[d].routes[a]=next.days[e].routes[a].clone();next.days[d].styles[a]=next.days[e].styles[a];
      }else{
       let b=self.rng.n(n);if Some(b)!=next.supplier{for day in &mut next.days[d..]{day.routes.swap(a,b);day.styles.swap(a,b);}}
      }
     }else if len>1{next.days[d].routes[a].reverse();}
    }
    if self.discount>=0.9 && self.rng.n(1000)==0{
     if let Some(a)=next.supplier{let b=self.rng.n(n);next.supplier=Some(b);for day in &mut next.days{day.routes.swap(a,b);day.styles.swap(a,b);}}
    }
    if self.rng.n(32)==0&&next.days[d].automatic{next.days[d].fixed=None;}
    next.value=self.evaluate_frozen(&mut next,false).0;let delta=next.value-cur.value;if sampled_meeting&&next.value>best.value{meeting_improvements+=1;}
    if delta>=0.0||self.rng.f()< (delta/temperature).exp(){cur=next;}
    if cur.value>best.value{*best=cur.clone();}
    iteration+=1;
   }
   if iteration%8192==0&&self.discount<0.9&&rebuild_ms<rebuild_limit_ms{
    let mut rebuilt=best.clone();let began=Instant::now();let until=(began+std::time::Duration::from_secs_f64((total*0.05).min((rebuild_limit_ms-rebuild_ms)as f64/1000.0))).min(deadline);self.rebuild(&mut rebuilt,until,self.rng.0%4==0);rebuilt.value=self.evaluate_frozen(&mut rebuilt,false).0;rebuild_ms+=began.elapsed().as_millis();
    if rebuilt.value>best.value{*best=rebuilt.clone();cur=rebuilt;}else if rebuilt.value>cur.value{cur=rebuilt;}
   }
   if iteration%32768==0&&self.discount<0.9{let mut polished=best.clone();let until=(Instant::now()+std::time::Duration::from_secs_f64(total*0.04)).min(deadline);self.beam_improve(&mut polished,until,None);if polished.value>best.value{*best=polished.clone();cur=polished;}}
   if progress>0.45&&!paired&&self.discount<0.9{paired=true;let mut polished=best.clone();let until=(Instant::now()+std::time::Duration::from_secs_f64(total*0.15)).min(deadline);self.paired_beam(&mut polished,until);if polished.value>best.value{*best=polished.clone();cur=polished;}}
   if iteration%16384==0 && self.rng.n(3)==0{cur=best.clone();}
  }
  eprintln!("generation6 iterations={iteration} rebuild_ms={rebuild_ms} meeting_improvements={meeting_improvements}/{meeting_attempts}");
 }

 fn sample_meeting(&mut self,s:&Solution,steps:u32)->Option<(usize,u32)>{
  let active:Vec<usize>=(0..self.initial.len()).filter(|&a|Some(a)!=s.supplier).collect();
  let a=active[self.rng.n(active.len())];let(_,plan)=self.evaluate(s,true);
  let mut time=0;let mut cell=self.network().nodes[self.initial[a].node];let mut visits=Vec::new();
  for &action in &plan[a]{
   let duration=if action<0{(-action)as u32}else{self.network().costs[cell]};
   let begin=time.max(1);let end=(time+duration-if action>=0{1}else{0}).min(steps);
   let node=self.network().node_at[cell];if node!=usize::MAX&&begin<=end{visits.push((node,begin,end));}
   time+=duration;if action>=0{cell=self.problem.map.neighbor(cell,action as usize).unwrap();}
  }
  let node=self.network().node_at[cell];if node!=usize::MAX&&time<=steps{visits.push((node,time.max(1),steps));}
  if visits.is_empty(){return None;}let(node,begin,end)=visits[self.rng.n(visits.len())];
  Some((node,begin+self.rng.n((end-begin+1)as usize)as u32))
 }

 fn cheapest_insertion(&self,day:&Day,a:usize,token:i16)->usize{
  let route=&day.routes[a];let goal=target(token);let style=path_style(day.styles[a],token);let mut source=self.initial[a].node;let mut best=f64::INFINITY;let mut index=0;
  for i in 0..=route.len(){
   let first=self.network().leg(style,source,goal);let mut cost=first.time as f64+first.fuel as f64*0.2;
   if let Some(&next)=route[i..].iter().find(|&&v|v>=0){let mode=path_style(day.styles[a],next);let after=self.network().leg(mode,goal,target(next));let before=self.network().leg(mode,source,target(next));cost+=after.time as f64-before.time as f64+(after.fuel as f64-before.fuel as f64)*0.2;}
   if cost<best{best=cost;index=i;}if i<route.len()&&route[i]>=0{source=target(route[i]);}
  }
  index
 }

 fn beam_improve(&mut self,s:&mut Solution,deadline:Instant,agent:Option<usize>){
  let budget=deadline.saturating_duration_since(Instant::now()).as_secs_f64();let width=if budget>2.0{8192}else if budget>0.6{2048}else{1024};
  let n=self.initial.len();let m=self.problem.spots.len();let steps=self.problem.day_steps[self.day];
  let a=agent.unwrap_or_else(||self.rng.n(n));if Some(a)==s.supplier{return;}
  let mut candidate=s.clone();
  let stops=if let Some(supplier)=s.supplier{
   if let Some(fixed)=&s.days[0].fixed{Rc::clone(&self.cached_supply(fixed,self.initial[supplier].node,steps).schedule)}
   else if s.days[0].automatic{let (appointments,delays)=self.automatic_supply(&s.days[0],&self.initial,supplier,steps,self.day);let built=self.cached_supply(&appointments,self.initial[supplier].node,steps);candidate.days[0].fixed=Some(appointments);candidate.days[0].start_wait=delays;Rc::clone(&built.schedule)}
   else{self.make_schedule(self.supply_plan(&s.days[0],self.initial[supplier].node,steps,false).0)}
  }else{self.make_schedule(Vec::new())};
  let (_,plan)=self.evaluate(&candidate,true);let mut game=self.game.clone();let Ok(replay)=game.play_day(plan) else{return;};
  let mut masks=vec![0u32;n];let mut at=vec![0u32;self.problem.map.len()];for(i,spot)in self.problem.spots.iter().enumerate(){at[spot.pos]=1<<i;}
  for frame in replay.frames.iter().skip(1){for (i,car) in frame.agents.iter().enumerate(){if car.kind==0{masks[i]|=at[car.pos];}}}
  let mut other_brands=0u32;let mut remaining=vec![0i32;m];
  for i in 0..m{let count=(0..n).filter(|&b|b!=a&&(masks[b]>>i)&1!=0).count() as i32;remaining[i]=self.problem.spots[i].stocks as i32-count;if count>0{other_brands|=self.brands[i];}}
  let reward=|mut mask:u32|->f64{let mut brands=other_brands;let mut value=0.;while mask!=0{let i=mask.trailing_zeros()as usize;mask&=mask-1;if remaining[i]>0{value+=100.;if brands&self.brands[i]==0{value+=10000.;if self.seen&self.brands[i]==0{value+=1_000_000.;}brands|=self.brands[i];}}}value};
  #[derive(Clone)] struct State{car:Car,time:u32,mask:u32,value:f64,route:Vec<i16>}
  let style=self.rng.n(STYLES);let mut car=self.initial[a];let mask=if car.node<m{1<<car.node}else{0};let time=if candidate.days[0].fixed.is_some(){candidate.days[0].start_wait[a]}else{0};
  if time>0&&stops.iter().any(|s|s.node==car.node&&s.begin<=time&&s.end>=1){car.fuel=self.problem.fuel_limits;}
  let mut beam=vec![State{car,time,mask,value:reward(mask),route:Vec::new()}];let mut finals=Vec::new();
  for _ in 0..self.problem.spots.len()+3{
   if Instant::now()>=deadline{break;}
   let mut next=Vec::new();
   for state in &beam{
    for goal in 0..m{for mode in 0..STYLES{
     let leg=self.network().leg(mode,state.car.node,goal);if state.time+leg.time>steps{continue;}
     if (0..mode).any(|old|{let l=self.network().leg(old,state.car.node,goal);l.time==leg.time&&l.fuel==leg.fuel&&l.mask==leg.mask}){continue;}
     let Some(fuel)=self.travel_fuel(state.car,state.time,leg,&stops)else{continue;};
     let mask=state.mask|leg.mask;if mask==state.mask{continue;}
     let mut route=state.route.clone();route.push((goal+TOKEN_BASE*(mode+1))as i16);
     next.push(State{car:Car{node:goal,fuel},time:state.time+leg.time,mask,value:reward(mask),route});
    }}
    if state.car.fuel<self.problem.fuel_limits/2||state.car.fuel<(steps-state.time)*2{
    let mut offered=[0u64;16];for stop in stops.iter(){
     if stop.node==usize::MAX{continue;}
     if offered[stop.node/64]&(1<<(stop.node%64))!=0{continue;}
     let leg=self.network().leg(style,state.car.node,stop.node);let meet=(state.time+leg.time).max(stop.begin);
     if meet>stop.end||meet>steps{continue;}
     let Some(_fuel)=self.travel_fuel(state.car,state.time,leg,&stops)else{continue;};
     if state.car.fuel.saturating_sub(leg.fuel)+self.problem.fuel_limits/5>=self.problem.fuel_limits{continue;}offered[stop.node/64]|=1<<(stop.node%64);
     let mut route=state.route.clone();route.push((stop.node+TOKEN_BASE*(style+1))as i16);if meet>state.time+leg.time{route.push(-((meet-state.time-leg.time+1)as i16));}
     let mask=state.mask|leg.mask;next.push(State{car:Car{node:stop.node,fuel:self.problem.fuel_limits},time:meet,mask,value:reward(mask),route});
    }}
   }
   if next.is_empty(){break;}
   next.sort_unstable_by(|x,y|(y.value-y.time as f64*0.5+y.car.fuel as f64*0.05).total_cmp(&(x.value-x.time as f64*0.5+x.car.fuel as f64*0.05)));
   let mut keys=std::collections::HashMap::<(usize,u32),Vec<(u32,u32)>>::new();
   next.retain(|s|{let entries=keys.entry((s.car.node,s.mask)).or_default();if entries.iter().any(|&(t,f)|t<=s.time&&f>=s.car.fuel){false}else{entries.push((s.time,s.car.fuel));true}});next.truncate(width);
   finals.extend(next.iter().take(64).cloned());beam=next;
  }
  for state in finals{
   candidate.days[0].routes[a]=state.route;candidate.days[0].styles[a]=style;candidate.value=self.evaluate(&candidate,false).0;
   if candidate.value>s.value{*s=candidate.clone();}
  }
 }

 fn paired_beam(&mut self,s:&mut Solution,deadline:Instant){
  let active:Vec<usize>=(0..self.initial.len()).filter(|&a|Some(a)!=s.supplier).collect();if active.len()<2{return;}
  let a=active[self.rng.n(active.len())];let b=*active.iter().filter(|&&b|b!=a).min_by_key(|&&b|self.network().leg(0,self.initial[a].node,self.initial[b].node).time).unwrap();
  let mut next=s.clone();next.days[0].routes[a].clear();next.days[0].routes[b].clear();next.value=self.evaluate_frozen(&mut next,false).0;
  for (i,car)in [a,b,a].into_iter().enumerate(){let now=Instant::now();if now>=deadline{break;}let until=now+deadline.saturating_duration_since(now)/(3-i)as u32;self.beam_improve(&mut next,until,Some(car));}
  if next.value>s.value{eprintln!("generation6 paired gain={:.1}",next.value-s.value);*s=next;}
 }

 fn rebuild(&mut self,s:&mut Solution,deadline:Instant,all:bool){
  let n=self.initial.len();let m=self.problem.spots.len();
  let a=self.rng.n(n);let selected:Vec<usize>=(0..n).filter(|&b|Some(b)!=s.supplier&&(all||a==b||self.rng.n(3)==0)).collect();
  if selected.is_empty(){return;}
  if s.supplier.is_some()&&(all||self.rng.n(4)==0){
   let day=&mut s.days[0];day.fixed=None;day.automatic=true;day.start_wait.fill(0);day.supply.clear();day.dwell.clear();day.styles[s.supplier.unwrap()]=0;
  }
  for &a in &selected{s.days[0].routes[a].clear();s.days[0].styles[a]=self.rng.n(2);}
  s.value=self.evaluate(s,false).0;
  let exponent=0.35+self.rng.f()*0.65;
  let noise:Vec<f64>=(0..n*m).map(|_|0.65+self.rng.f()*0.7).collect();
  for _ in 0..m*n{
   if Instant::now()>=deadline{return;}
   let mut choice=None;let mut merit=0.0;
   for &a in &selected{
    let route=&s.days[0].routes[a];let style=s.days[0].styles[a];
    for spot in 0..m{if route.iter().any(|&v|v>=0&&target(v)==spot){continue;}
     for i in 0..=route.len(){
      if (i+spot*32)%32==0&&Instant::now()>=deadline{return;}
      let from=route[..i].iter().rev().find(|&&v|v>=0).map(|&v|target(v)).unwrap_or(self.initial[a].node);
      let after=route[i..].iter().find(|&&v|v>=0).map(|&v|target(v));
      let extra=(self.network().leg(style,from,spot).time as i32+after.map(|v|self.network().leg(style,spot,v).time as i32-self.network().leg(style,from,v).time as i32).unwrap_or(0)).max(0) as f64;
      let mut next=s.clone();next.days[0].routes[a].insert(i,spot as i16);next.value=self.evaluate(&next,false).0;
      let score=(next.value-s.value)*noise[a*m+spot]/(extra+3.0).powf(exponent);
      if score>merit{merit=score;choice=Some(next);}
     }
    }
   }
   if let Some(next)=choice{*s=next;}else{break;}
  }
 }
}
