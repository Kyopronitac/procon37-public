use std::cmp::Reverse;
use std::collections::{BinaryHeap,HashMap};
use std::cell::RefCell;
use std::time::{Duration, Instant};
use tester::model::{Problem, Observation, Plan, Event, Traffic};
use tester::engine::Game;
const STOPS:usize=4;
const PATH_MODES:usize=4;
const MAX_AGENTS:usize=8;
const MAX_STEPS:usize=256;

#[derive(Clone, Default)]
struct Path { dirs: Vec<i64>, cells: Vec<usize>, time: i32, fuel: i32, mask: u32 }
struct Graph {
    paths: Vec<Vec<Path>>, positions: Vec<usize>, alternatives: Vec<Vec<Vec<Path>>>, durations:Vec<i32>, costs:Vec<i32>,
    spot_at: Vec<i32>, potential:Vec<Vec<(i32,i32)>>, mobility:Vec<i32>, fuel_to: RefCell<Vec<Vec<i32>>>, cell_node:Vec<usize>, targets:usize, memo:RefCell<HashMap<(u64,u64),(u32,i32,usize,usize,i32,f64,i32)>>,
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
                let target=positions[j];let mut d=vec![i32::MAX;size];let mut next=vec![(0,0);size];
                let mut heap=BinaryHeap::new();d[target]=0;heap.push((Reverse(0),target));
                while let Some((Reverse(cost),x))=heap.pop(){if d[x]!=cost{continue;}
                    for dir in 0..6{if let Some(y)=p.map.neighbor(x,dir){if p.map.terrain(y)==3{continue;}
                        let nd=cost+durations[y]*[100,100,10,100][mode]+costs[y]*[3,100,100,3][mode]-if mode==3&&spot_at[y]>=0{20}else{0};
                        if nd<d[y]{d[y]=nd;next[y]=(x,((dir+3)%6)as i64);heap.push((Reverse(nd),y));}
                    }}
                }
                for (i,&source)in positions.iter().enumerate(){
                    let mut x=source;let mut path=Path::default();
                    while x!=target{let (y,dir)=next[x];path.dirs.push(dir);path.cells.push(y);path.time+=durations[x];path.fuel+=costs[x];if spot_at[y]>=0{path.mask|=1<<spot_at[y];}x=y;}
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
        let potential=paths.iter().map(|row|row[..p.spots.len()].iter().map(|path|(path.time,path.fuel)).collect()).collect();
        let mobility=positions.iter().map(|&pos|(0..p.spots.len()).filter(|&i|p.spots[i].pos!=pos).map(|i|fuel_to[i][pos]).min().unwrap_or(0)).collect();
        Self{paths,positions,alternatives:variants,durations,costs,spot_at,potential,mobility,fuel_to:RefCell::new(fuel_to),cell_node,targets,memo:RefCell::new(HashMap::new())}
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
#[derive(Clone)]
struct Solution { hubs:[usize;STOPS], departures:[i32;STOPS-1], routes:Vec<Vec<usize>> }
impl Solution {
    fn overwrite(&mut self,other:&Self){
        self.hubs=other.hubs;self.departures=other.departures;
        for (route,source) in self.routes.iter_mut().zip(&other.routes){route.clear();route.extend_from_slice(source);}
    }
}
#[derive(Clone,Default)]
struct Outcome {value:f64, masks:[u32;MAX_AGENTS], plans:Plan, endfuel:[i32;MAX_AGENTS], arrivefuel:[i32;MAX_AGENTS], endtime:[i32;MAX_AGENTS], endnodes:[usize;MAX_AGENTS], actual:[usize;MAX_AGENTS]}
struct Rng(u64);
impl Rng {fn u(&mut self)->u64{self.0^=self.0<<13;self.0^=self.0>>7;self.0^=self.0<<17;self.0}fn n(&mut self,n:usize)->usize{(self.u() as usize)%n.max(1)}fn f(&mut self)->f64{(self.u()>>11) as f64/(1u64<<53)as f64}}

pub struct Solver {p:Problem, supplier:usize, game:Option<Game>, last:Option<Plan>, rng:Rng, cache:Option<[Solution;2]>, cache_positions:Vec<usize>, cache_day:usize, cache_complete:bool, selecting:bool, seen_brands:u64, coverage_weight:f64, beam_time_weight:f64, archive_weight:f64, future_weight:f64, reserve_weight:f64,last_value:f64}
impl Solver {
    pub fn new(p:Problem,exploration_seed:u64,coverage_weight:f64,beam_time_weight:f64,archive_weight:f64)->Self{let seed=p.agents.iter().fold(9183479123,|a,&b|(a^b as u64).wrapping_mul(673493))^exploration_seed.wrapping_mul(0x9e3779b185ebca87);Self{p,supplier:0,game:None,last:None,rng:Rng(seed|1),cache:None,cache_positions:vec![],cache_day:0,cache_complete:false,selecting:false,seen_brands:0,coverage_weight,beam_time_weight,archive_weight,future_weight:std::env::var("PROCON37_G007_FUTURE_WEIGHT").ok().and_then(|x|x.parse::<f64>().ok()).filter(|x|x.is_finite()&&*x>=0.0&&*x<=1.5).unwrap_or(0.0),reserve_weight:std::env::var("PROCON37_G007_RESERVE_WEIGHT").ok().and_then(|x|x.parse::<f64>().ok()).filter(|x|x.is_finite()&&*x>=0.0&&*x<=100.0).unwrap_or(15.0),last_value:f64::NEG_INFINITY}}
    pub fn select(&mut self,limit:u64)->Vec<u8>{
        let began=Instant::now();let n=self.p.agents.len();
        let obs=Game::new(self.p.clone(),&vec![0;n]).observation(0);
        let g=Graph::new(&self.p,&obs,&[]);let steps=self.p.day_steps[0]as i32;
        let mut bestv=f64::NEG_INFINITY;let mut best_supplier=0;let mut warm_all=Vec::new();
        for a in 0..n {
            if a>0&&began.elapsed().as_millis()>u128::from(limit*70/100){break;}
            self.supplier=a;
            let candidate=self.initial(&g,&obs,steps,None);let out=self.evaluate(&g,&obs,steps,&candidate,false,None);
            if out.value>bestv {bestv=out.value;best_supplier=a;}
            warm_all.push((a,candidate));
        }
        let positions=g.positions.clone();drop(g);
        self.supplier=best_supplier;
        let initial=warm_all.iter().find(|(a,_)|*a==best_supplier).unwrap().1.clone();
        self.cache=Some([initial.clone(),initial]);self.cache_positions=positions.clone();self.cache_complete=false;self.cache_day=0;
        let mut kinds=vec![0;n];kinds[self.supplier]=1;self.game=Some(Game::new(self.p.clone(),&kinds));
        let remaining=limit.saturating_sub(began.elapsed().as_millis()as u64+200);
        if remaining>=700*warm_all.len()as u64 {
            // Optimize each role independently: changing the supplier need not preserve an old route assignment.
            let slot=remaining/warm_all.len()as u64;
            warm_all.sort_by_key(|(a,_)|usize::from(*a!=best_supplier));
            let mut selected:Option<(usize,[Solution;2],Vec<usize>,f64)>=None;
            for (a,initial) in warm_all {
                let available=limit.saturating_sub(began.elapsed().as_millis()as u64+150);if available<700{break;}
                self.supplier=a;self.last=None;self.selecting=false;
                let mut kinds=vec![0;n];kinds[a]=1;self.game=Some(Game::new(self.p.clone(),&kinds));
                self.cache=Some([initial.clone(),initial]);self.cache_positions=positions.clone();self.cache_day=0;self.cache_complete=false;
                let observation=self.game.as_ref().unwrap().observation(0);
                let role_started=Instant::now();let role_budget=slot.min(available);
                let rollout=self.future_weight==0.0&&self.p.day_steps.len()>1&&role_budget>=1000;
                let forecast_count=if rollout{if role_budget>=4000&&self.p.day_steps.len()>2{2}else{1}}else{0};
                let first_limit=if forecast_count==2{role_budget/2}else if rollout{role_budget*60/100}else{role_budget};
                let first_plan=self.plan_inner(&observation,steps as u32,first_limit,true,false);self.last=None;
                let first_value=self.last_value;let first_cache=self.cache.as_ref().unwrap().clone();let first_coords=self.cache_positions.clone();
                let mut forecast_values=[0.0;2];let mut forecast_done=0;
                if rollout {
                    let mut previous_plan=first_plan;let mut previous_observation=observation.clone();
                    for day in 1..=forecast_count {
                        let forecast_limit=role_budget.saturating_sub(role_started.elapsed().as_millis()as u64+50)/(forecast_count-day+1)as u64;
                        if forecast_limit<300{break;}
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
            if extra>700{let observation=self.game.as_ref().unwrap().observation(0);let _=self.plan_inner(&observation,steps as u32,extra,true,false);self.last=None;}
        }else if remaining>700 {
            let observation=self.game.as_ref().unwrap().observation(0);self.selecting=true;
            let _=self.plan_inner(&observation,steps as u32,remaining,true,false);self.selecting=false;self.last=None;
        }
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
        self.forecast_traffic_ratio(previous,current,observed,1,2)
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
        let mut windows=[(steps+1,steps+1);STOPS];
        let mut st=0;let mut snode=prev.map_or(m+sup,|v|v.endnodes[sup]);
        let mut endhub=snode;let mut trace=[g.positions[snode];MAX_STEPS+1];
        for k in 0..STOPS {
            let h=s.hubs[k];let path=&g.paths[snode][h];let arrival=st+path.time;
            let mut q=g.positions[snode];let mut clock=st;let mut moved=0;
            for &r in &path.cells {let finish=clock+g.durations[q];if finish>steps{break;}
                for v in clock+1..finish{trace[v as usize]=q;}trace[finish as usize]=r;clock=finish;q=r;moved+=1;
            }
            if output{out.plans[sup].extend(&path.dirs[..moved]);}
            st=clock;snode=g.cell_node[q];endhub=snode;
            if moved<path.cells.len(){out.value-=0.002*(arrival-steps).max(0)as f64;break;}
            endhub=h;snode=h;
            let depart=if k<STOPS-1{s.departures[k].max(st).min(steps)}else{steps};
            windows[k]=(st,depart);for v in st+1..=depart{trace[v as usize]=g.positions[h];}
            if output&&depart>st{out.plans[sup].push(-i64::from(depart-st));}
            st=depart;
        }
        for v in st+1..=steps{trace[v as usize]=g.positions[endhub];}
        if output&&st<steps{out.plans[sup].push(-i64::from(steps-st));}
        out.endnodes[sup]=endhub;out.endfuel[sup]=cap;out.arrivefuel[sup]=cap;
        let mut base_hash=(17389853233441u64,672594024112321u64);
        let mix=|h:&mut(u64,u64),v:usize|{h.0=(h.0^v as u64).wrapping_mul(0x9e3779b185ebca87);h.1=(h.1.wrapping_add(v as u64+17)).rotate_left(17).wrapping_mul(0xc2b2ae3d27d4eb4f);};
        for v in [sup,steps as usize,prev.is_some()as usize,prev.map_or(m+sup,|v|v.endnodes[sup])]{mix(&mut base_hash,v);}
        for &v in &s.hubs{mix(&mut base_hash,v);}for &v in &s.departures{mix(&mut base_hash,v as usize);}
        if g.memo.borrow().len()>100000{g.memo.borrow_mut().clear();}
        for a in 0..n {if a==sup{continue;}
            let mut node=prev.map_or(m+a,|v|v.endnodes[a]);let mut pos=g.positions[node];let mut fuel=prev.map_or(obs.agents[a].fuel as i32,|v|v.endfuel[a]);let mut t=0;let mut mask=0u32;
            let mut hash=base_hash;mix(&mut hash,node);mix(&mut hash,fuel as usize);mix(&mut hash,s.routes[a].len());
            for &v in &s.routes[a]{mix(&mut hash,v);}
            if !output {if let Some(&(cm,cf,cn,ca,ct,cv,caf))=g.memo.borrow().get(&hash){out.masks[a]=cm;out.endfuel[a]=cf;out.arrivefuel[a]=caf;out.endnodes[a]=cn;out.actual[a]=ca;out.endtime[a]=ct;out.value+=cv;continue;}}
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
                    if output{out.plans[a].push(path.dirs[moved]);}
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
            if !output {g.memo.borrow_mut().insert(hash,(mask,fuel,node,out.actual[a],t,out.value-before,out.arrivefuel[a]));}
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
        let mut s=Solution{hubs:[best.1;STOPS],departures:std::array::from_fn(|i|steps*(i+1)as i32/STOPS as i32),routes:vec![vec![];n]};
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
    fn beam_car(&self,g:&Graph,obs:&Observation,steps:i32,s:&mut Solution,car:usize,deadline:Instant,prev:Option<&Outcome>)->usize{
        let m=self.p.spots.len();let base=m+2*STOPS;
        let time_weight=self.beam_time_weight*(m as f64/32.0).powi(2);
        let mut best=s.routes[car].clone();let mut best_value=self.evaluate(g,obs,steps,s,false,prev).value;
        let mut trial=s.clone();trial.routes[car].clear();
        let zero=self.evaluate(g,obs,steps,&trial,false,prev);
        let mut beam=vec![(Vec::<usize>::new(),zero.masks[car])];
        let mut layers=0;
        for depth in 0..20 {
            let mut expanded=Vec::<(f64,Vec<usize>,u32,usize,i32,i32)>::new();
            let mut seen=HashMap::<(u32,usize),Vec<usize>>::new();
            for (route,mask) in &beam {
                if Instant::now()>=deadline {s.routes[car]=best;return layers;}
                for token in 0..base {
                    if token<m&&mask&(1<<token)!=0{continue;}
                    if route.last().is_some_and(|v|v%base==token){continue;}
                    for mode in 0..PATH_MODES {
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
            beam=expanded.into_iter().map(|(_,r,m,_,_,_)|(r,m)).collect();
        }
        s.routes[car]=best;layers
    }
    fn horizon(&self,g:&Graph,ng:&Graph,obs:&Observation,steps:i32,next_steps:i32,s:&[Solution;2])->f64{
        let first=self.evaluate(g,obs,steps,&s[0],false,None);
        if next_steps<=0{return first.value;}
        let second=self.evaluate(ng,obs,next_steps,&s[1],false,Some(&first));
        first.value+self.future_weight*second.value
    }
    pub fn plan(&mut self,obs:&Observation,steps:u32,limit:u64,accepted:bool)->Plan{
        self.plan_inner(obs,steps,limit,accepted,true)
    }
    fn plan_inner(&mut self,obs:&Observation,steps:u32,limit:u64,accepted:bool,allow_archive:bool)->Plan{
        let began=Instant::now();
        let coverage_guard=allow_archive&&self.archive_weight==0.0&&self.future_weight==0.0&&self.p.spots.len()<=16&&limit>=20000&&obs.day+1<self.p.day_steps.len();
        let use_archive=allow_archive&&self.future_weight==0.0&&obs.day+1<self.p.day_steps.len()&&((self.archive_weight>0.0&&limit>=5000)||coverage_guard);
        if let Some(game)=self.game.as_mut(){
            if let Some(last)=self.last.take(){let plan=if accepted{last}else{game.waiting_plan()};if let Ok(day)=game.play_day(plan){for frame in day.frames{for event in frame.events{if let Event::Collect{brand,..}=event{self.seen_brands|=1u64<<brand;}}}}}
            game.agents=obs.agents.clone();game.day=obs.day;game.set_observed_traffic(obs.traffics.clone());
        }
        let extra=self.cache.as_ref().map(|old|old.iter().flat_map(|s|s.hubs.iter().map(|&h|self.cache_positions[h])).collect::<Vec<_>>()).unwrap_or_default();
        let graph_started=Instant::now();
        let g=Graph::new(&self.p,obs,&extra);
        let graph_millis=graph_started.elapsed().as_millis();
        let mut forecast=obs.clone();
        for t in &mut forecast.traffics {t.status=t.status.max(1);}
        let mut next_graph=if self.future_weight>0.0{Some(Graph::new(&self.p,&forecast,&extra))}else{None};
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
            let alternative=if same_day&&self.cache_complete{old}else{[old[if same_day{0}else{1}].clone(),current[1].clone()]};
            let v=self.horizon(&g,&ng,obs,steps,next_steps,&alternative);if v>val{current=alternative;val=v;}
        }
        let mut best=current.clone();let mut bestv=val;let mut best_supplier=self.supplier;
        let total_budget=Duration::from_millis(limit.saturating_sub(110).min(limit*91/100).max(1));
        let budget=if coverage_guard{total_budget*4/5}else if use_archive{total_budget*3/4}else{total_budget};
        let mut archive=Vec::<(Solution,Outcome,Vec<u64>)>::new();
        let mut it=0u64;let mut phase=0.0;let mut beam_calls=0usize;let mut beam_layers=0usize;
        let mut fleet_calls=0usize;let mut fleet_accepted=0usize;let mut fleet_improved=0usize;let mut fleet_rescues=0usize;
        let mut triple_calls=0usize;let mut triple_accepted=0usize;let mut triple_improved=0usize;
        let mut restart_cycle=0u32;let mut restart_calls=0usize;
        let mut candidate=current.clone();
        while began.elapsed()<budget {
            it+=1;if it%512==0{phase=began.elapsed().as_secs_f64()/budget.as_secs_f64();}
            for day in 0..2{candidate[day].overwrite(&current[day]);}
            let day=if next_steps>0&&self.rng.n(100)<46{1}else{0};let next=&mut candidate[day];let ds=if day==0{steps}else{next_steps};let mut a=self.rng.n(n);while a==self.supplier{a=self.rng.n(n);}
            let typ=self.rng.n(if n>=4{124}else{116});
            if typ<6 {let len=next.routes[a].len();if len>0{let i=self.rng.n(len);next.routes[a][i]=next.routes[a][i]%(m+2*STOPS)+(m+2*STOPS)*self.rng.n(PATH_MODES);}}
            else if typ<30 {let len=next.routes[a].len();if len<24{let idx=self.rng.n(len+1);let token=if self.rng.n(6)==0{m+self.rng.n(2*STOPS)}else{self.rng.n(m)};next.routes[a].insert(idx,token);}}
            else if typ<47 {let len=next.routes[a].len();if len>0{let idx=self.rng.n(len);next.routes[a].remove(idx);}}
            else if typ<66 {let len=next.routes[a].len();if len>0{let i=self.rng.n(len);let j=self.rng.n(len);if self.rng.n(2)==0{next.routes[a].swap(i,j);}else{next.routes[a][i.min(j)..=i.max(j)].reverse();}}}
            else if typ<79 {let mut b=self.rng.n(n);while b==self.supplier{b=self.rng.n(n);}let len=next.routes[a].len();if len>0{let i=self.rng.n(len);let token=next.routes[a].remove(i);let j=self.rng.n(next.routes[b].len()+1);next.routes[b].insert(j,token);}}
            else if typ<91 {let len=next.routes[a].len();if len>0{let i=self.rng.n(len);next.routes[a][i]=if self.rng.n(6)==0{m+self.rng.n(2*STOPS)}else{self.rng.n(m)};}}
            else if typ<97 {let k=self.rng.n(STOPS);next.hubs[k]=if self.rng.n(4)==0{next.hubs[(k+1)%STOPS]}else{self.rng.n(g.targets)};if self.rng.n(4)==0{let j=self.rng.n(STOPS);next.hubs[j]=next.hubs[k];}}
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
                let d=if next_steps>0{self.rng.n(2)}else{0};let graph=if d==0{&g}else{&ng};let ds=if d==0{steps}else{next_steps};
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
            let cycle=(phase*12.0).fract();let temperature=if phase<0.8{(350.0f64).powf(1.0-cycle)*0.8}else{(1.0-phase)*12.0+0.08};
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
            }
            let next_cycle=(phase*12.0)as u32;
            if next_cycle>restart_cycle {restart_cycle=next_cycle;restart_calls+=1;current=best.clone();self.supplier=best_supplier;
                let first=self.evaluate(&g,obs,steps,&current[0],false,None);
                for a in 0..n{current[0].routes[a].truncate(first.actual[a]);}
                if next_steps>0{let second=self.evaluate(&ng,obs,next_steps,&current[1],false,Some(&first));for a in 0..n{current[1].routes[a].truncate(second.actual[a]);}}
                val=self.horizon(&g,&ng,obs,steps,next_steps,&current);
            }
        }
        self.supplier=best_supplier;
        if use_archive&&!coverage_guard {
            if let Some(game)=&self.game {
                let mut after=game.clone();
                let plan=self.evaluate(&g,obs,steps,&best[0],true,None).plans;
                if after.play_day(plan).is_ok(){forecast.traffics=self.forecast_traffic(game.last_stays(),after.last_stays(),&obs.traffics);}
            }
            next_graph=Some(Graph::new(&self.p,&forecast,&extra));
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
        self.last_value=bestv;self.last=Some(plan.clone());self.cache=Some(best);self.cache_positions=g.positions.clone();self.cache_day=obs.day;self.cache_complete=true;plan
    }
}
