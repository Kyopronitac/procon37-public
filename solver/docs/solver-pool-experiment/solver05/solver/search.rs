use std::collections::HashMap;
use std::time::{Duration, Instant};
use tester::{engine::Game, model::*};
use crate::network::{Network, Path};

#[derive(Clone)]
struct Rng(u64);
impl Rng {
    fn next(&mut self)->u64 {self.0^=self.0<<7;self.0^=self.0>>9;self.0^=self.0<<8;self.0}
    fn at(&mut self,n:usize)->usize {(self.next()%n.max(1) as u64) as usize}
    fn unit(&mut self)->f64 {(self.next()>>11) as f64/(1u64<<53) as f64}
    fn shuffle<T>(&mut self,a:&mut [T]) {for i in (1..a.len()).rev(){a.swap(i,self.at(i+1));}}
}

#[derive(Clone,Copy)]
struct Stop {node:usize,wait:u16}
#[derive(Clone)]
struct Route {actions:Vec<i64>,vis:u32,fuel:u16,end:usize}
#[derive(Clone)]
struct Solution {routes:Vec<Route>,support:Vec<Vec<Stop>>,raw:Option<Vec<Vec<i64>>>,value:f64}
#[derive(Clone)]
struct Schedule {present:Vec<bool>,windows:Vec<(usize,u16,u16)>,len:usize,steps:u16,ends:Vec<usize>}
impl Schedule {
    fn has(&self,pos:usize,t:u16)->bool {t>0&&self.present[t as usize*self.len+pos]}
}
#[derive(Clone,Copy)]
struct State {node:usize,t:u16,fuel:u16,vis:u32,gain:f64,parent:usize,variant:u8,wait:u16}

pub struct Planner {rng:Rng,seen:u32,seed:u64,opening:Option<Solution>,future_hint:Option<(usize,Vec<Vec<(usize,u16,usize)>>)>,selecting:bool,stream:u64,select_iterations:usize,select_trials:usize}
impl Planner {
    pub fn new(seed:u64)->Self {Self {rng:Rng(seed.wrapping_add(0x9e3779b97f4a7c15)),seen:0,seed,opening:None,future_hint:None,selecting:false,stream:0,select_iterations:0,select_trials:0}}
    fn reset_rng(&mut self,day:usize) {
        self.rng=Rng(self.seed.wrapping_add(self.stream).wrapping_add(0x9e3779b97f4a7c15).wrapping_add((day as u64).wrapping_mul(0xbf58476d1ce4e5b9)));
    }
    pub fn remember(&mut self,day:&DayReplay) {
        for frame in &day.frames {for e in &frame.events {if let Event::Collect {spot,..}=e {self.seen|=1u32<<spot;}}}
    }
    pub fn select(&mut self,p:&Problem,deadline:Instant,budget_ms:u64)->Vec<u8> {
        self.future_hint=None;self.selecting=true;
        let n=p.agents.len();let started=Instant::now();
        let short=budget_ms<15000;
        let seconds=budget_ms as f64*0.001;
        let mut round_cost=p.day_steps.iter().map(|&t|f64::from(t).powi(2)).sum::<f64>()*if n==5{0.00014}else{0.00008};
        let scale=if n<=5{(seconds*0.65/round_cost).min(1.)}else{1.};
        self.select_iterations=if n<=5{(32.*scale).floor().max(4.) as usize}else if short{3}else{6};
        self.select_trials=if n<=5{(25000.*scale).floor().max(2500.) as usize}else if short{7500}else{15000};
        round_cost*=scale;
        let rounds=if n>5&&short{1}else{(seconds*0.75/round_cost).floor().clamp(1.,3.) as usize};
        eprintln!("selection work rounds={rounds} iterations={} trials={}",self.select_iterations,self.select_trials);
        let mut configs=Vec::new();
        if p.fuel_limits as usize*2>=p.day_steps.iter().map(|&x|x as usize).sum::<usize>(){configs.push(vec![0;n]);}
        for id in 0..n{let mut kinds=vec![0;n];kinds[id]=1;configs.push(kinds);}
        let mut scores=vec![f64::NEG_INFINITY;configs.len()];let mut openings=vec![None;configs.len()];
        let mut best_index=0;
        'rounds: for round in 0..rounds {
            self.stream=(round as u64).wrapping_mul(0x94d049bb133111eb);
            for(ci,kinds)in configs.iter().enumerate(){
                let mut g=Game::new(p.clone(),kinds);let mut seen=0;let mut opening=None;
                for day in 0..p.day_steps.len(){
                    if Instant::now()+Duration::from_millis(50)>=deadline{break 'rounds;}
                    self.reset_rng(day);
                    let sol=self.optimize(&g,seen,deadline,28,None,None);
                    if day==0{opening=Some(sol.clone());}
                    if let Ok(rep)=g.play_day(sol.routes.iter().map(|r|r.actions.clone()).collect()){
                        for f in &rep.frames{for e in &f.events{if let Event::Collect{spot,..}=e{seen|=1u32<<spot;}}}
                    }else{let _=g.play_day(g.waiting_plan());}
                }
                let value=g.score.unique_brands as f64*1e12+g.score.daily_brands as f64*1e7+g.score.udon as f64*1000.;
                eprintln!("fixed forecast round={round} kinds={kinds:?} score={:?} elapsed_ms={}",g.score,started.elapsed().as_millis());
                if value>scores[ci]{scores[ci]=value;openings[ci]=opening;}
                if scores[ci]>scores[best_index]{best_index=ci;}
            }
        }
        self.opening=if n<=5{openings[best_index].take()}else{None};
        self.selecting=false;self.stream=0;
        eprintln!("selection value={:.0} kinds={:?}",scores[best_index],configs[best_index]);configs.swap_remove(best_index)
    }
    pub fn plan(&mut self,g:&Game,deadline:Instant)->Plan {
        self.reset_rng(g.day);
        let beam=if g.problem.agents.len()<=4 {120} else {90};
        let opening=if g.day==0{self.opening.take()}else{None};
        let began=Instant::now();
        let remaining=deadline.saturating_duration_since(began);
        if g.agents.len()<=5&&g.problem.fuel_limits*10<g.problem.day_steps[0]*14&&g.day+1<g.problem.day_steps.len()&&remaining.as_millis()>=1000 {
            let first_end=began+remaining.mul_f64(0.35);
            let current=self.optimize(g,self.seen,first_end,beam,opening,None);
            let mut next=g.clone();
            if let Ok(replay)=next.play_day(current.routes.iter().map(|r|r.actions.clone()).collect()) {
                let mut seen=self.seen;for f in &replay.frames{for e in &f.events{if let Event::Collect{spot,..}=e{seen|=1u32<<spot;}}}
                let old_traffic=g.traffic();let mut traffic=next.traffic();
                for tr in &mut traffic{tr.status=old_traffic.iter().find(|x|x.pos==tr.pos).map_or(0,|old|old.status);}
                next.set_observed_traffic(traffic.clone());self.reset_rng(next.day);
                let next_end=began+remaining.mul_f64(0.50);
                let future=self.optimize(&next,seen,next_end,90,None,None);
                let build_began=Instant::now();let current_traffic=g.traffic();
                let base=Network::new(&g.problem,&g.agents,&current_traffic);
                let mut anchors:Vec<_>=base.nodes.iter().map(|&pos|Agent{kind:0,pos,fuel:g.problem.fuel_limits}).collect();
                let today:Vec<_>=current.routes.iter().enumerate().map(|(i,r)|(g.agents[i].pos,r.actions.as_slice())).collect();
                let tomorrow:Vec<_>=future.routes.iter().enumerate().map(|(i,r)|(next.agents[i].pos,r.actions.as_slice())).collect();
                for pos in Network::route_anchors(&g.problem,&current_traffic,&today).into_iter().chain(Network::route_anchors(&g.problem,&traffic,&tomorrow)) {
                    if !anchors.iter().any(|a|a.pos==pos){anchors.push(Agent{kind:0,pos,fuel:g.problem.fuel_limits});}
                }
                let net=Network::new(&g.problem,&anchors,&current_traffic);
                let anchors:Vec<_>=net.nodes.iter().map(|&pos|Agent{kind:0,pos,fuel:g.problem.fuel_limits}).collect();
                let future_net=Network::new(&g.problem,&anchors,&traffic);
                eprintln!("faithful network nodes={}/{} base={} build_ms={}",net.n,future_net.n,base.n,build_began.elapsed().as_millis());
                self.reset_rng(g.day);
                let (routes,hint)=two_day_polish(g,&mut next,&net,&future_net,&current.routes,&future.routes,self.seen,&mut self.rng,deadline);
                self.future_hint=Some((g.day+1,hint.into_iter().map(|ps|ps.into_iter().map(|p|(future_net.nodes[p.node],p.until,p.mode)).collect()).collect()));
                eprintln!("day={} joint two-day",g.day+1);
                return routes.into_iter().map(|r|r.actions).collect()
            }
            return current.routes.into_iter().map(|r|r.actions).collect()
        }
        let lookahead=g.agents.len()>=5&&g.day+1<g.problem.day_steps.len()&&remaining.as_millis()>=20000
            &&g.problem.fuel_limits>=g.problem.day_steps[g.day]*2
            &&g.agents.iter().filter(|a|a.kind==0).all(|a|a.fuel>=g.problem.day_steps[g.day]);
        let until=if lookahead {began+remaining.mul_f64(0.65)}else{deadline};
        let mut archive=Vec::new();
        let mut sol=self.optimize(g,self.seen,until,beam,opening,if lookahead {Some(&mut archive)} else {None});
        if lookahead {
            archive.retain(|x|x.value>=sol.value-3500.);
            archive.sort_by(|a,b|b.value.total_cmp(&a.value));
            let limit=if remaining.as_secs_f64()<35.0 {3}else{4};
            if archive.len()>limit {
                let mut selected=vec![archive[0].clone()];
                for j in 1..limit {selected.push(archive[j*(archive.len()-1)/(limit-1)].clone());}
                archive=selected;
            }
            if archive.is_empty(){archive.push(sol.clone());}
            let count=archive.len();let mut bestvalue=f64::NEG_INFINITY;
            for (k,candidate) in archive.into_iter().enumerate() {
                if Instant::now()>=deadline {break}
                let mut next=g.clone();
                let Ok(replay)=next.play_day(candidate.routes.iter().map(|r|r.actions.clone()).collect()) else {continue};
                let mut seen=self.seen;
                for f in &replay.frames{for e in &f.events{if let Event::Collect{spot,..}=e{seen|=1u32<<spot;}}}
                let candidate_end=Instant::now()+deadline.saturating_duration_since(Instant::now()).div_f64((count-k) as f64);
                let forecast_stop=(next.day+1).min(next.problem.day_steps.len());
                let old_traffic=g.traffic();let mut traffic=next.traffic();
                for tr in &mut traffic {if let Some(old)=old_traffic.iter().find(|x|x.pos==tr.pos){tr.status=tr.status.max(old.status.saturating_sub(1));}}
                next.set_observed_traffic(traffic);
                self.reset_rng(next.day);
                while next.day<forecast_stop {
                    let left=forecast_stop-next.day;
                    let slice=candidate_end.saturating_duration_since(Instant::now()).as_secs_f64()/left as f64;
                    let future=self.optimize(&next,seen,(Instant::now()+Duration::from_secs_f64(slice)).min(deadline),90,None,None);
                    if let Ok(rep)=next.play_day(future.routes.iter().map(|r|r.actions.clone()).collect()) {
                        for f in &rep.frames{for e in &f.events{if let Event::Collect{spot,..}=e{seen|=1u32<<spot;}}}
                    }else{break}
                }
                let mut value=(next.score.unique_brands-g.score.unique_brands) as f64*1000000.0
                    +(next.score.daily_brands-g.score.daily_brands) as f64*100000.0
                    +(next.score.udon-g.score.udon) as f64*1000.0;
                if next.day<next.problem.day_steps.len(){value+=next.agents.iter().filter(|a|a.kind==0).map(|a|reserve_value(&next,a.fuel as u16,next.agents.iter().any(|b|b.kind==1))).sum::<f64>();}
                if value>bestvalue {bestvalue=value;sol=candidate;}
            }
        }
        eprintln!("day={} value={:.1} kinds={:?}",g.day+1,sol.value,g.agents.iter().map(|a|a.kind).collect::<Vec<_>>());
        sol.routes.into_iter().map(|r|r.actions).collect()
    }

    fn optimize(&mut self,g:&Game,seen:u32,deadline:Instant,width:usize,warm:Option<Solution>,mut archive:Option<&mut Vec<Solution>>)->Solution {
        let p=&g.problem;let steps=p.day_steps[g.day] as u16;
        let mut anchors=g.agents.clone();
        if g.agents.len()<=5{if let Some((_,hints))=self.future_hint.as_ref().filter(|(day,_)|*day==g.day){for points in hints{for &(pos,_,_)in points{if !anchors.iter().any(|a|a.pos==pos){anchors.push(Agent{kind:0,pos,fuel:p.fuel_limits});}}}}}
        let mut net=if self.selecting{Network::with_variants(p,&anchors,&g.traffic(),3)}else{Network::new(p,&anchors,&g.traffic())};
        let mut suppliers:Vec<usize>=(0..g.agents.len()).filter(|&i|g.agents[i].kind==1).collect();
        let patrols:Vec<usize>=(0..g.agents.len()).filter(|&i|g.agents[i].kind==0).collect();
        if suppliers.is_empty() {suppliers=Vec::new();}
        let default_support:Vec<Vec<Stop>>=suppliers.iter().map(|&i|vec![Stop{node:net.node(g.agents[i].pos),wait:steps}]).collect();
        let mut best=warm.unwrap_or_else(||Solution {routes:Vec::new(),support:default_support,raw:None,value:f64::NEG_INFINITY});
        let mut incumbent=best.clone();let mut iter=0;
        let began=Instant::now();
        let final_deadline=deadline;
        let ample=width>=28&&p.fuel_limits as f64>=f64::from(steps)*1.3;
        let deadline=if self.selecting{final_deadline}else{began+deadline.saturating_duration_since(began).mul_f64(if ample{0.20}else{0.45})};
        loop {
            if iter>0&&(Instant::now()>=deadline||(self.selecting&&iter>=self.select_iterations)) {break}
            let partial=!self.selecting&&iter%3==1&&!best.routes.is_empty();
            let partial_base=if iter%9==1 {&best}else{&incumbent};
            let mut raw=if partial {partial_base.raw.clone()}else{None};
            let mut support=if iter%13==12 {self.random_support(&net,g,&suppliers,steps)} else if iter%7==6 {incumbent.support.clone()} else {best.support.clone()};
            if partial {support=partial_base.support.clone();}
            if !partial&&iter>0&&iter%13!=12&&!suppliers.is_empty() {self.mutate_support(&mut support,&net,g,&suppliers,steps,iter);}
            if !partial&&iter%5==2&&!best.routes.is_empty()&&!suppliers.is_empty() {
                let si=self.rng.at(support.len());
                let i=patrols[self.rng.at(patrols.len())];
                let node=best.routes[i].end;
                let k=self.rng.at(support[si].len());
                support[si][k]=Stop {node,wait:if self.rng.at(2)==0 {steps} else {self.rng.at(15) as u16}};
            }
            if !partial&&(iter%12==3||self.selecting&&iter==1)&&!best.routes.is_empty() {
                support=meeting_support(g,&net,&suppliers,&best.routes,&mut self.rng);
            }
            if !partial&&iter%120==9&&width>=90&&!best.routes.is_empty()&&!suppliers.is_empty()&&Instant::now()+Duration::from_millis(250)<deadline {
                let mut targets=best.routes.clone();
                if iter%240==9 {
                    let (ds,_)=if let Some(ref actions)=best.raw {raw_schedule(g,&net,&suppliers,actions)}else{make_schedule(g,&net,&suppliers,&best.support)};
                    let mut relaxed=g.clone();relaxed.problem.fuel_limits=10000;for a in &mut relaxed.agents{a.fuel=10000;}
                    let mut counts=vec![0;p.spots.len()];let mut other=0;
                    for &i in &patrols {
                        targets[i]=patrol_route(&relaxed,i,&net,&ds,&counts,other,seen,40,&vec![1.0;32],deadline,None);
                        for(s,c)in counts.iter_mut().enumerate(){if targets[i].vis&(1u32<<s)!=0{*c+=1;}}
                        other|=net.brand_mask(targets[i].vis);
                    }
                }
                raw=Some(exact_support(g,&net,&suppliers,&targets,&mut self.rng,deadline,1100));
            }
            let (sched,supplyroutes)=if let Some(ref actions)=raw {raw_schedule(g,&net,&suppliers,actions)}else{make_schedule(g,&net,&suppliers,&support)};
            let mut order=patrols.clone();
            if iter>0 {self.rng.shuffle(&mut order);}
            if partial {order.truncate((2+self.rng.at(2)).min(order.len()));}
            let mut routes=if partial {partial_base.routes.clone()} else {vec![Route {actions:vec![-i64::from(steps)],vis:0,fuel:0,end:0};g.agents.len()]};
            for (&i,r) in suppliers.iter().zip(supplyroutes) {routes[i]=r;}
            let mut counts=vec![0u8;p.spots.len()]; let mut others=0;
            if partial {for &i in &patrols {if !order.contains(&i) {
                for(s,c)in counts.iter_mut().enumerate(){if routes[i].vis&(1u32<<s)!=0{*c+=1;}}
                others|=net.brand_mask(routes[i].vis);
            }}}
            let mut noise=vec![1.0;32];
            if iter%3!=0 {for x in &mut noise {*x=0.84+self.rng.unit()*0.32;}}
            let late=!self.selecting&&began.elapsed().as_secs_f64()>deadline.saturating_duration_since(began).as_secs_f64()*0.5;
            let bw=if late {width*3} else if iter%7==0 {width*2} else {width};
            for (oi,&i) in order.iter().enumerate() {
                let mut local=noise.clone();
                if iter%3!=0 {
                    let mut mine=[u16::MAX;32];let mut theirs=[u16::MAX;32];
                    for s in 0..p.spots.len() {
                        let b=net.brands[s].trailing_zeros() as usize;
                        mine[b]=mine[b].min(net.path(net.node(g.agents[i].pos),s,0).time);
                        for &j in &order[oi+1..] {theirs[b]=theirs[b].min(net.path(net.node(g.agents[j].pos),s,0).time);}
                    }
                    for b in 0..32 {if mine[b]!=u16::MAX&&theirs[b]!=u16::MAX {
                        local[b]*=(0.55+0.9*(theirs[b] as f64+4.0)/(mine[b] as f64+theirs[b] as f64+8.0))*(0.82+self.rng.unit()*0.36);
                    }}
                }
                let route=patrol_route(g,i,&net,&sched,&counts,others,seen,bw,&local,deadline,None);
                for (s,c) in counts.iter_mut().enumerate(){if route.vis&(1u32<<s)!=0 {*c+=1;}}
                others|=net.brand_mask(route.vis);routes[i]=route;
            }
            // Reoptimize each route against its actual teammates instead of preserving the first assignment.
            if iter%4==0 && Instant::now()<deadline {
                self.rng.shuffle(&mut order);
                for &i in &order {
                    let mut count=vec![0u8;p.spots.len()];let mut masks=0;
                    for &j in &patrols {if i!=j {masks|=net.brand_mask(routes[j].vis);for(s,c)in count.iter_mut().enumerate(){if routes[j].vis&(1u32<<s)!=0 {*c+=1;}}}}
                    routes[i]=patrol_route(g,i,&net,&sched,&count,masks,seen,bw,&vec![1.0;32],deadline,None);
                    if Instant::now()>=deadline {break}
                }
            }
            if !self.selecting&&iter%8==5&&Instant::now()+Duration::from_millis(60)<deadline {
                let packed=pack_routes(g,&net,&sched,&patrols,&suppliers,&routes,seen,bw,&mut self.rng,deadline);
                if solution_value(g,&net,&packed,seen,&suppliers)>solution_value(g,&net,&routes,seen,&suppliers){routes=packed;}
            }
            if Instant::now()+Duration::from_millis(10)<deadline {
                let mut improve=patrols.clone();self.rng.shuffle(&mut improve);
                if iter%4!=0 {improve.truncate(2);}
                for &i in &improve {
                    let mut count=vec![0u8;p.spots.len()];let mut other=0;
                    for &j in &patrols {if i!=j {other|=net.brand_mask(routes[j].vis);for(s,c)in count.iter_mut().enumerate(){if routes[j].vis&(1u32<<s)!=0{*c+=1;}}}}
                    routes[i]=polish_route(g,i,&net,&sched,&count,other,seen,&routes[i],&mut self.rng,deadline);
                }
            }
            let value=solution_value(g,&net,&routes,seen,&suppliers);
            let candidate=Solution {routes,support,raw,value};
            if value>best.value {best=candidate.clone();}
            if let Some(pool)=archive.as_mut() {
                if value>=best.value-110000.0&&!pool.iter().any(|x|x.routes.iter().zip(&candidate.routes).all(|(a,b)|a.end==b.end)) {
                    pool.push(candidate.clone());
                    pool.sort_by(|a,b|b.value.total_cmp(&a.value));
                    let mut levels=HashMap::<i64,usize>::new();
                    pool.retain(|x| {if x.value<best.value-110000.0{return false}let level=(x.value/10000.0).floor() as i64;let c=levels.entry(level).or_default();*c+=1;*c<=3});
                    if pool.len()>30 {
                        let highest=pool[0].value;let mut levels=[0usize;2];pool.retain(|x|{let k=usize::from(x.value<highest-50000.);levels[k]+=1;levels[k]<=15});
                    }
                }
            }
            let fraction=if self.selecting{iter as f64/self.select_iterations as f64}else if g.agents.len()>5 {began.elapsed().as_secs_f64()/deadline.saturating_duration_since(began).as_secs_f64().max(0.001)}else{(iter%64) as f64/64.0};
            let temp=3500.0*(1.0-fraction).max(0.)+400.0;
            if value>incumbent.value||self.rng.unit()<((value-incumbent.value)/temp).exp() {incumbent=candidate;}
            iter+=1;
        }
        if Instant::now()<final_deadline {
            if g.agents.len()<=5&&width>=90 {
                let build_began=Instant::now();let old_n=net.n;let traffic=g.traffic();
                let raw:Vec<_>=best.routes.iter().enumerate().map(|(i,r)|(g.agents[i].pos,r.actions.as_slice())).collect();
                let mut anchors:Vec<_>=net.nodes.iter().map(|&pos|Agent{kind:0,pos,fuel:p.fuel_limits}).collect();
                for pos in Network::route_anchors(p,&traffic,&raw){if !anchors.iter().any(|a|a.pos==pos){anchors.push(Agent{kind:0,pos,fuel:p.fuel_limits});}}
                if anchors.len()>net.n{net=Network::new(p,&anchors,&traffic);}
                eprintln!("single faithful network base={old_n} nodes={} build_ms={}",net.n,build_began.elapsed().as_millis());
            }
            let (sched,_)=if let Some(ref actions)=best.raw{raw_schedule(g,&net,&suppliers,actions)}else{make_schedule(g,&net,&suppliers,&best.support)};
            let polishing_end=Instant::now()+final_deadline.saturating_duration_since(Instant::now()).mul_f64(if suppliers.is_empty()||g.day+1==g.problem.day_steps.len(){1.0}else if ample{0.95}else{0.73});
            let mut finish_rng=if self.selecting&&g.agents.len()>5{self.rng.clone()}else{Rng(self.seed.wrapping_add(self.stream).wrapping_add(0xd1b54a32d192ed03).wrapping_add((g.day as u64).wrapping_mul(0xbf58476d1ce4e5b9)))};
            let hint=self.future_hint.as_ref().filter(|(day,_)|*day==g.day).map(|(_,rs)|rs.iter().map(|ps|ps.iter().filter_map(|&(pos,until,mode)|net.find_node(pos).map(|node|Waypoint{node,until,mode})).collect::<Vec<_>>()).collect::<Vec<_>>());
            best.routes=if width>=28 {soft_polish_fleet(g,&net,&sched,&best.routes,seen,&suppliers,&mut finish_rng,polishing_end,ample&&width>=90,archive.as_deref_mut(),hint.as_deref(),g.agents.len()<=5&&width>=90,if self.selecting{Some(self.select_trials)}else{None})}else{polish_fleet(g,&net,&sched,&best.routes,seen,&suppliers,&mut finish_rng,polishing_end,ample,archive.as_deref_mut())};
            if g.agents.len()>5{self.rng=finish_rng;}
            best.raw=Some(suppliers.iter().map(|&i|best.routes[i].actions.clone()).collect());
            best.value=solution_value(g,&net,&best.routes,seen,&suppliers);
            let mut support_trials=0;
            while !suppliers.is_empty()&&(!self.selecting||support_trials<1)&&Instant::now()+Duration::from_millis(60)<final_deadline {
                support_trials+=1;
                let raw=exact_support(g,&net,&suppliers,&best.routes,&mut self.rng,final_deadline,if self.selecting{160}else{1100});
                let (schedule,supply)=raw_schedule(g,&net,&suppliers,&raw);
                let mut routes=best.routes.clone();for(&id,r)in suppliers.iter().zip(supply){routes[id]=r;}
                let mut valid=true;
                for &i in &patrols {if let Some(r)=reschedule_route(g,i,&net,&schedule,&routes[i]){routes[i]=r;}else{valid=false;break}}
                if valid {
                    let value=solution_value(g,&net,&routes,seen,&suppliers);
                    if value>best.value{best.routes=routes;best.raw=Some(raw);best.value=value;}
                }
            }
            best.value=solution_value(g,&net,&best.routes,seen,&suppliers);
            if let Some(pool)=archive.as_mut(){pool.push(best.clone());}
        }
        if width>=90 {eprintln!("search iterations={iter} score={:.1}",best.value);}
        best
    }
    fn random_support(&mut self,net:&Network,g:&Game,suppliers:&[usize],steps:u16)->Vec<Vec<Stop>> {
        suppliers.iter().map(|&i|{
            let mut result=Vec::new();let mut cur=net.node(g.agents[i].pos);let mut t=0;
            for _ in 0..(1+self.rng.at(4)) {
                let candidates:Vec<usize>=(0..net.n).filter(|&s|net.path(cur,s,0).time<steps.saturating_sub(t)).collect();
                if candidates.is_empty(){break}
                let dest=candidates[self.rng.at(candidates.len())];
                let wait=(self.rng.at(15)+2) as u16;
                t+=net.path(cur,dest,0).time+wait;result.push(Stop{node:dest,wait});cur=dest;
                if t>=steps {break}
            }
            if result.is_empty(){result.push(Stop{node:cur,wait:steps});}
            result
        }).collect()
    }
    fn mutate_support(&mut self,support:&mut [Vec<Stop>],net:&Network,g:&Game,suppliers:&[usize],steps:u16,iter:usize) {
        let si=self.rng.at(support.len());let r=&mut support[si];
        if r.is_empty(){r.push(Stop{node:net.node(g.agents[suppliers[si]].pos),wait:steps});}
        let k=self.rng.at(r.len());
        match self.rng.at(7) {
            0=>{let d=self.rng.at(19) as i32-9;r[k].wait=(r[k].wait as i32+d).clamp(0,steps as i32) as u16;}
            1|2=>{r[k].node=self.rng.at(net.n);if r[k].wait>steps/2{r[k].wait=(self.rng.at(20)+1) as u16;}}
            3=>{if r.len()<6 {r.insert(k,Stop{node:self.rng.at(net.n),wait:self.rng.at(20) as u16});}}
            4=>{if r.len()>1 {r.remove(k);}else{r[k].wait=self.rng.at(steps as usize) as u16;}}
            5=>{if r.len()>1{let j=self.rng.at(r.len());r.swap(k,j);}else{r.push(Stop{node:self.rng.at(net.n),wait:0});}}
            _=>{
                // Supply the least fueled collector or a nearby stock point.
                let mut ids:Vec<usize>=(0..g.agents.len()).filter(|&i|g.agents[i].kind==0).collect();
                ids.sort_by_key(|&i|g.agents[i].fuel);
                let a=ids[self.rng.at(ids.len().min(3))];
                r[k].node=net.node(g.agents[a].pos);r[k].wait=(self.rng.at(12)+1) as u16;
            }
        }
        if iter%19==0 {r.truncate(1);r[0].wait=steps;}
    }
}

fn exact_support(g:&Game,net:&Network,suppliers:&[usize],routes:&[Route],rng:&mut Rng,deadline:Instant,beam_width:usize)->Vec<Vec<i64>> {
    #[derive(Clone,Copy)]
    struct N {pos:usize,mask:u16,score:f32,rank:f32,parent:usize,action:i64}
    let steps=g.problem.day_steps[g.day] as usize;
    let patrols:Vec<usize>=(0..g.agents.len()).filter(|&i|g.agents[i].kind==0).collect();
    let mut poses=vec![vec![0;patrols.len()];steps+1];let mut need=vec![vec![0f32;patrols.len()];steps+1];let mut exhausted=vec![steps+1;patrols.len()];
    for (j,&i) in patrols.iter().enumerate() {
        let mut pos=g.agents[i].pos;let mut t=0usize;let mut consumed=0u32;
        let weight=(0.75+rng.unit()*0.5) as f32;
        for &action in &routes[i].actions {
            let duration=if action<0 {(-action) as usize}else{net.duration[pos] as usize};
            for dt in 1..=duration {if t+dt<=steps {poses[t+dt][j]=pos;need[t+dt][j]=((g.problem.fuel_limits-g.agents[i].fuel+consumed).min(g.problem.fuel_limits) as f32+3.0)*weight;}}
            if action>=0 {consumed+=net.cost[pos] as u32;pos=g.problem.map.neighbor(pos,action as usize).unwrap();}
            t+=duration;if t>steps{break}
            if consumed>g.agents[i].fuel {exhausted[j]=exhausted[j].min(t);}
            poses[t][j]=pos;need[t][j]=((g.problem.fuel_limits-g.agents[i].fuel+consumed).min(g.problem.fuel_limits) as f32+3.0)*weight;
        }
    }
    for t in 1..=steps {for j in 0..patrols.len() {
        need[t][j]*=if t<exhausted[j] {1.6}else{0.45};
        if g.day+1==g.problem.day_steps.len(){need[t][j]*=0.1+0.9*(steps-t) as f32/steps as f32;}
    }}
    let mut results=Vec::new();
    for &id in suppliers {
        let mut arena=vec![N {pos:g.agents[id].pos,mask:0,score:0.,rank:0.,parent:usize::MAX,action:0}];
        let mut fronts=vec![Vec::<usize>::new();steps+1];fronts[0].push(0);
        let mut bests:Vec<HashMap<(usize,u16),f32>>=(0..=steps).map(|_|HashMap::new()).collect();
        let mut best_end=None;
        for t in 0..=steps {
            let mut now=std::mem::take(&mut fronts[t]);
            now.retain(|&idx|{let s=arena[idx];t==0||bests[t].get(&(s.pos,s.mask)).is_some_and(|&x|s.score>=x)});
            now.sort_unstable_by(|&a,&b|arena[b].rank.total_cmp(&arena[a].rank));now.truncate(beam_width);
            if t==steps {
                for idx in now {let s=arena[idx];if net.nodes.contains(&s.pos)&&best_end.is_none_or(|old:usize|s.score>arena[old].score){best_end=Some(idx);}}
                break
            }
            for idx in now {
                let st=arena[idx];
                for action in -1i64..=5 {
                    let (duration,dest)=if action<0 {(1,st.pos)} else {
                        let Some(q)=g.problem.map.neighbor(st.pos,action as usize).filter(|&q|g.problem.map.terrain(q)!=3)else{continue};
                        (net.duration[st.pos] as usize,q)
                    };
                    if t+duration>steps{continue}
                    for take in 0..2 {
                    let mut mask=st.mask;let mut score=st.score;
                    for dt in 1..=duration {
                        let pos=if dt==duration {dest}else{st.pos};
                        for j in 0..patrols.len() {if take==1&&mask&(1<<j)==0&&poses[t+dt][j]==pos {mask|=1<<j;score+=need[t+dt][j];}}
                    }
                    let nt=t+duration;
                    if bests[nt].get(&(dest,mask)).is_some_and(|&old|old>=score){continue}
                    let mut heuristic=0.;let x=dest%g.problem.map.width;let y=dest/g.problem.map.width;
                    for j in 0..patrols.len() {if mask&(1<<j)==0 {
                        let q=poses[steps][j];let dist=x.abs_diff(q%g.problem.map.width).max(y.abs_diff(q/g.problem.map.width)) as f32;
                        heuristic+=need[steps][j]*0.2*(-dist/10.0).exp();
                    }}
                    bests[nt].insert((dest,mask),score);
                    arena.push(N{pos:dest,mask,score,rank:score+heuristic,parent:idx,action});fronts[nt].push(arena.len()-1);
                    }
                }
            }
            if t%8==0&&Instant::now()+Duration::from_millis(30)>=deadline {break}
        }
        if let Some(mut at)=best_end {
            let mut actions=Vec::new();while arena[at].parent!=usize::MAX {actions.push(arena[at].action);at=arena[at].parent;}actions.reverse();
            // Coalesce consecutive waits for compact protocol output.
            let mut packed=Vec::<i64>::new();for a in actions {if a<0&&packed.last().is_some_and(|&x|x<0){*packed.last_mut().unwrap()-=1;}else{packed.push(a);}}
            results.push(packed);
        }else{results.push(routes[id].actions.clone());}
    }
    results
}

fn raw_schedule(g:&Game,net:&Network,suppliers:&[usize],actions:&[Vec<i64>])->(Schedule,Vec<Route>) {
    let steps=g.problem.day_steps[g.day] as u16;let len=g.problem.map.len();
    let mut schedule=Schedule {present:vec![false;(steps as usize+1)*len],windows:Vec::new(),len,steps,ends:Vec::new()};let mut routes=Vec::new();
    for (&id,plan) in suppliers.iter().zip(actions) {
        let mut pos=g.agents[id].pos;let mut t=0u16;
        for &action in plan {
            let duration=if action<0 {(-action) as u16}else{net.duration[pos]};
            let dest=if action<0 {pos}else{g.problem.map.neighbor(pos,action as usize).unwrap()};
            for dt in 1..=duration {schedule.present[(t+dt) as usize*len+if dt==duration{dest}else{pos}]=true;}
            t+=duration;pos=dest;
        }
        let end=net.node(pos);schedule.ends.push(end);routes.push(Route{actions:plan.clone(),vis:0,fuel:g.problem.fuel_limits as u16,end});
    }
    for (node,&pos) in net.nodes.iter().enumerate(){let mut t=1;while t<=steps {if !schedule.has(pos,t){t+=1;continue}let start=t;while t<steps&&schedule.has(pos,t+1){t+=1;}schedule.windows.push((node,start,t));t+=1;}}
    (schedule,routes)
}

fn meeting_support(g:&Game,net:&Network,suppliers:&[usize],routes:&[Route],rng:&mut Rng)->Vec<Vec<Stop>> {
    #[derive(Clone)]
    struct Meeting {node:usize,start:u16,end:u16,car:usize,need:f64}
    #[derive(Clone)]
    struct Tour {node:usize,t:u16,mask:u16,value:f64,stops:Vec<Stop>}
    let steps=g.problem.day_steps[g.day] as u16;
    let mut at_node=vec![usize::MAX;g.problem.map.len()];
    for (i,&pos) in net.nodes.iter().enumerate(){at_node[pos]=i;}
    let mut meets=Vec::new();
    for(i,r)in routes.iter().enumerate(){if g.agents[i].kind==0 {
        let mut poses=vec![g.agents[i].pos;steps as usize+1];let mut spent=vec![0u16;steps as usize+1];
        let mut t=0u16;let mut pos=g.agents[i].pos;let mut consumed=0;
        for &a in &r.actions {
            let dur=if a<0 {(-a) as u16}else{net.duration[pos]};
            for dt in 1..=dur {if t+dt>steps{break}poses[(t+dt) as usize]=pos;spent[(t+dt) as usize]=consumed;}
            if a>=0 {consumed+=net.cost[pos];pos=g.problem.map.neighbor(pos,a as usize).unwrap();}
            t+=dur;if t>steps {break}poses[t as usize]=pos;spent[t as usize]=consumed;
        }
        let noise=0.7+rng.unit()*0.7;
        let mut t=1usize;
        while t<=steps as usize {
            let pos=poses[t];let lo=t;while t<steps as usize&&poses[t+1]==pos{t+=1;}
            if at_node[pos]!=usize::MAX {
                let need=(g.problem.fuel_limits as f64-g.agents[i].fuel as f64+spent[lo] as f64).min(g.problem.fuel_limits as f64);
                meets.push(Meeting{node:at_node[pos],start:lo as u16,end:t as u16,car:i,need:(need+8.0)*noise});
            }
            t+=1;
        }
    }}
    let mut result=Vec::new();let mut done=0u16;
    for &i in suppliers {
        let init=Tour{node:net.node(g.agents[i].pos),t:0,mask:done,value:0.,stops:Vec::new()};
        let mut best=init.clone();let mut beam=vec![init];
        for _ in 0..g.agents.len() {
            let mut next=Vec::new();
            for st in &beam {for m in &meets {
                if st.mask&(1<<m.car)!=0 {continue}
                let travel=net.path(st.node,m.node,0).time;let arrival=(st.t+travel).max(m.start).max(st.t+1);
                if arrival>m.end {continue}
                let mut stops=st.stops.clone();stops.push(Stop {node:m.node,wait:arrival-st.t-travel});
                let nt=Tour{node:m.node,t:arrival,mask:st.mask|(1<<m.car),value:st.value+m.need,stops};
                if nt.value>best.value {best=nt.clone();}next.push(nt);
            }}
            next.sort_unstable_by(|a,b|(b.value-b.t as f64*0.1).total_cmp(&(a.value-a.t as f64*0.1)));
            let mut unique=HashMap::new();
            beam=next.into_iter().filter(|s|{let c=unique.entry((s.node,s.mask)).or_insert(0);*c+=1;*c<=2}).take(60).collect();
            if beam.is_empty(){break}
        }
        done=best.mask;
        if best.stops.is_empty(){best.stops.push(Stop {node:net.node(g.agents[i].pos),wait:steps});}
        result.push(best.stops);
    }
    result
}

fn make_schedule(g:&Game,net:&Network,suppliers:&[usize],support:&[Vec<Stop>])->(Schedule,Vec<Route>) {
    let steps=g.problem.day_steps[g.day] as u16;let len=g.problem.map.len();
    let mut schedule=Schedule {present:vec![false;(steps as usize+1)*len],windows:Vec::new(),len,steps,ends:Vec::new()};
    let mut routes=Vec::new();
    for (si,&i) in suppliers.iter().enumerate() {
        let mut at=net.node(g.agents[i].pos);let mut pos=net.nodes[at];let mut t=0;let mut actions=Vec::new();
        for stop in &support[si] {
            let path=net.path(at,stop.node,0);
            if t+path.time>steps {break}
            for (j,&dir) in path.dirs.iter().enumerate(){
                let dur=net.duration[pos];
                for dt in 1..=dur {let cp=if dt==dur {path.cells[j+1]} else {pos};schedule.present[(t+dt) as usize*len+cp]=true;}
                t+=dur;pos=path.cells[j+1];actions.push(dir);
            }
            at=stop.node;
            let wait=stop.wait.min(steps-t);
            if wait>0 {actions.push(-i64::from(wait));for dt in 1..=wait {schedule.present[(t+dt) as usize*len+pos]=true;}t+=wait;}
            if t==steps {break}
        }
        if t<steps {actions.push(-i64::from(steps-t));for tt in t+1..=steps {schedule.present[tt as usize*len+pos]=true;}}
        schedule.ends.push(at);
        routes.push(Route{actions,vis:0,fuel:g.problem.fuel_limits as u16,end:at});
    }
    for (node,&pos) in net.nodes.iter().enumerate() {
        let mut t=1;
        while t<=steps {
            if !schedule.has(pos,t) {t+=1;continue}
            let start=t;while t<steps&&schedule.has(pos,t+1){t+=1;}
            schedule.windows.push((node,start,t));t+=1;
        }
    }
    (schedule,routes)
}

fn visit(pos:usize,net:&Network,counts:&[u8],p:&Problem,vis:&mut u32) {
    let s=net.spot[pos];
    if s>=0&&counts[s as usize]<(p.spots[s as usize].stocks as u8){*vis|=1u32<<(s as usize);}
}

fn travel(st:State,path:&Path,wait:u16,net:&Network,schedule:&Schedule,counts:&[u8],p:&Problem)->Option<(u16,u16,u32)> {
    let mut t=st.t;let mut fuel=st.fuel;let mut vis=st.vis;let mut pos=net.nodes[st.node];
    if t+path.time+wait>schedule.steps {return None}
    for j in 0..path.dirs.len() {
        let dur=net.duration[pos];let cost=net.cost[pos];
        for dt in 1..=dur {
            t+=1;
            if dt==dur {if fuel<cost{return None}fuel-=cost;pos=path.cells[j+1];}
            visit(pos,net,counts,p,&mut vis);
            if schedule.has(pos,t){fuel=p.fuel_limits as u16;}
        }
    }
    for _ in 0..wait {t+=1;visit(pos,net,counts,p,&mut vis);if schedule.has(pos,t){fuel=p.fuel_limits as u16;}}
    Some((t,fuel,vis))
}

fn gains(vis:u32,net:&Network,other:u32,seen:u32,noise:&[f64])->f64 {
    let brands=net.brand_mask(vis);let mut bits=brands&!other;let mut daily=0.;
    while bits!=0 {let b=bits.trailing_zeros() as usize;bits&=bits-1;daily+=noise[b];}
    let balls=vis.count_ones() as f64;
    let mut bits=brands&!seen&!other;let mut unique=0.;
    while bits!=0 {let b=bits.trailing_zeros() as usize;bits&=bits-1;unique+=noise[b];}
    unique*1000000.0+daily*100000.0+balls*1000.0
}

fn terminal(st:State,net:&Network,sched:&Schedule,counts:&[u8],g:&Game,other:u32,seen:u32,noise:&[f64])->(f64,u16,u32) {
    let mut vis=st.vis;let pos=net.nodes[st.node];let mut fuel=st.fuel;
    if st.t<sched.steps {visit(pos,net,counts,&g.problem,&mut vis);for t in st.t+1..=sched.steps {if sched.has(pos,t){fuel=g.problem.fuel_limits as u16;break}}}
    let future=if g.day+1<g.problem.day_steps.len(){1.0}else{0.0};
    let cap=g.problem.fuel_limits as f64;
    let fuel_factor=if noise[31]==1.0 {1.0}else{(0.3+4.0*(noise[31]-0.84)).clamp(0.1,2.0)};
    let mut value=gains(vis,net,other,seen,noise)+future*(fuel_factor*reserve_value(g,fuel,!sched.ends.is_empty())+net.density[st.node]*0.9);
    if future>0.0 {
        value-=rescue_penalty(g,net,st.node,fuel,sched.ends.iter().map(|&j|net.path(j,st.node,0).time).min().unwrap_or(u16::MAX));
        let need=sched.ends.iter().map(|&s|net.path(st.node,s,2).fuel).min().unwrap_or(0);
        if need>fuel {value-=400.0*(need-fuel) as f64/cap;}
    }
    (value,fuel,vis)
}

fn reserve_value(g:&Game,fuel:u16,supplied:bool)->f64 {
    if supplied {let next=g.problem.day_steps[(g.day+1).min(g.problem.day_steps.len()-1)] as f64;let useful=(g.problem.fuel_limits as f64).min(next*0.90).max(1.);let weight=if g.agents.len()>5{2500.}else{1000.};return weight*(fuel as f64/useful).min(1.0).sqrt();}
    let remaining=(g.problem.day_steps.len()-g.day) as f64;
    let average=g.agents.iter().map(|a|a.fuel as f64).sum::<f64>()/g.agents.len() as f64;
    let target=average*(remaining-1.0)/remaining;
    -180.0*(target-fuel as f64).max(0.0)
}

fn rescue_penalty(g:&Game,net:&Network,node:usize,fuel:u16,supply_time:u16)->f64 {
    if g.agents.len()>5{let need=(net.exit_need[node] as f64).max(g.problem.fuel_limits as f64*0.35);let days=(g.problem.day_steps.len()-g.day-1) as f64;return 5000.*days*(1.-fuel as f64/need).max(0.);}
    let need=(net.exit_need[node] as f64).max((g.problem.day_steps[(g.day+1).min(g.problem.day_steps.len()-1)] as f64*0.72).min(g.problem.fuel_limits as f64));
    let days=(g.problem.day_steps.len()-g.day-1) as f64;
    let proximity=if supply_time==u16::MAX||days==0. {1.}else{
        let eta=(f64::from(supply_time)-f64::from(fuel)*0.65-2.).max(0.);
        (eta/(g.problem.day_steps[g.day+1] as f64*0.45)).min(1.)
    };
    5000.0*days.min(2.)*(1.0-fuel as f64/need).max(0.0)*proximity
}

fn patrol_route(g:&Game,id:usize,net:&Network,sched:&Schedule,counts:&[u8],other:u32,seen_spots:u32,width:usize,noise:&[f64],deadline:Instant,mut pool:Option<&mut Vec<Route>>)->Route {
    let seen=net.brand_mask(seen_spots);let p=&g.problem;let init=State {node:net.node(g.agents[id].pos),t:0,fuel:g.agents[id].fuel as u16,vis:0,gain:0.,parent:usize::MAX,variant:0,wait:0};
    let mut arena=vec![init];let mut beam=vec![0usize];let mut best=0;let (mut bestv,mut bestfuel,mut bestvis)=terminal(init,net,sched,counts,g,other,seen,noise);
    let mut finals=vec![(bestv,0usize,bestfuel,bestvis)];
    for depth in 0..36 {
        let mut next=Vec::new();
        for &bi in &beam {
            let st=arena[bi];
            for to in 0..p.spots.len() {
                if st.vis&(1u32<<to)!=0||counts[to]>=p.spots[to].stocks as u8 {continue}
                for v in 0..net.variants {
                    let path=net.path(st.node,to,v);
                    if v>0 {let prev=net.path(st.node,to,v-1);if prev.time==path.time&&prev.fuel==path.fuel {continue}}
                    let wait=if path.time==0 {1}else{0};
                    if let Some((t,fuel,vis))=travel(st,path,wait,net,sched,counts,p) {
                        if vis==st.vis {continue}
                        let gain=gains(vis,net,other,seen,noise);
                        let s=State {node:to,t,fuel,vis,gain,parent:bi,variant:v as u8,wait};
                        arena.push(s);next.push(arena.len()-1);
                    }
                }
            }
            if depth<32 && st.fuel<(p.fuel_limits as u16)*9/10 {
                for &(to,lo,hi) in &sched.windows {
                    for v in 0..2 {
                        let path=net.path(st.node,to,v);let arrival=st.t+path.time;
                        if arrival>hi||hi<=st.t {continue}
                        let wait=lo.max(arrival).max(st.t+1)-arrival;
                        if let Some((t,fuel,vis))=travel(st,path,wait,net,sched,counts,p) {
                            if fuel<=st.fuel+3&&vis==st.vis {continue}
                            let gain=gains(vis,net,other,seen,noise);
                            let s=State {node:to,t,fuel,vis,gain,parent:bi,variant:v as u8,wait};
                            arena.push(s);next.push(arena.len()-1);
                        }
                    }
                }
            }
        }
        if next.is_empty(){break}
        for &i in &next {
            let(v,fuel,vis)=terminal(arena[i],net,sched,counts,g,other,seen,noise);if v>bestv {bestv=v;best=i;bestfuel=fuel;bestvis=vis;}
            if pool.is_some(){finals.push((v,i,fuel,vis));}
            let st=arena[i];
            if g.day+1<g.problem.day_steps.len()&&st.t+6<sched.steps&&st.gain+40000.0>=arena[best].gain {
                let mut targets:Vec<usize>=(0..net.n).filter(|&to|to!=st.node&&(to>=p.spots.len()||st.vis&(1u32<<to)!=0)&&net.density[to]>net.density[st.node]+30.0&&net.path(st.node,to,0).time<=sched.steps-st.t).collect();
                targets.sort_unstable_by(|&a,&b|net.density[b].total_cmp(&net.density[a]));
                for to in targets.into_iter().take(3) {
                    for variant in 0..2 {
                        if let Some((t,fuel,vis))=travel(st,net.path(st.node,to,variant),0,net,sched,counts,p) {
                            let s=State {node:to,t,fuel,vis,gain:gains(vis,net,other,seen,noise),parent:i,variant:variant as u8,wait:0};
                            let (v,fuel,vis)=terminal(s,net,sched,counts,g,other,seen,noise);
                            if v>bestv {arena.push(s);best=arena.len()-1;bestv=v;bestfuel=fuel;bestvis=vis;}
                        }
                    }
                }
            }
        }
        next.sort_unstable_by(|&a,&b|{
            let score=|i:usize|{let s=arena[i];if g.agents.len()<=5||width>=90 {let balls=s.vis.count_ones() as f64*1000.;(s.gain-balls)*0.03+balls-s.t as f64*70.+s.fuel as f64*2.} else {s.gain-s.t as f64*9.0+s.fuel as f64*1.5}};
            score(b).total_cmp(&score(a))
        });
        let mut fronts:HashMap<(u32,usize),Vec<(u16,u16)>>=HashMap::new();
        beam=next.into_iter().filter(|&i|{
            let s=arena[i];let f=fronts.entry((s.vis,s.node)).or_default();
            if f.iter().any(|&(t,fuel)|t<=s.t&&fuel>=s.fuel)||f.len()>=4 {false}else{f.push((s.t,s.fuel));true}
        }).take(width).collect();
        if depth>=1&&Instant::now()>=deadline {break}
    }
    if let Some(out)=pool.as_mut() {
        finals.sort_unstable_by(|a,b|b.0.total_cmp(&a.0));
        let mut masks=std::collections::HashSet::new();
        for (_,i,fuel,vis) in finals {if masks.insert(vis) {
            out.push(build_route(&arena,i,fuel,vis,net,sched.steps));if out.len()>=if g.agents.len()<=4{500}else{180}{break}
        }}
        out.push(build_route(&arena,best,bestfuel,bestvis,net,sched.steps));
    }
    build_route(&arena,best,bestfuel,bestvis,net,sched.steps)
}

fn build_route(arena:&[State],best:usize,bestfuel:u16,bestvis:u32,net:&Network,steps:u16)->Route {
    let mut chain=Vec::new();let mut at=best;
    while arena[at].parent!=usize::MAX {chain.push(at);at=arena[at].parent;}
    chain.reverse();let mut actions=Vec::new();
    for i in chain {let s=arena[i];let prev=arena[s.parent];actions.extend_from_slice(&net.path(prev.node,s.node,s.variant as usize).dirs);if s.wait>0 {actions.push(-i64::from(s.wait));}}
    if arena[best].t<steps {actions.push(-i64::from(steps-arena[best].t));}
    Route {actions,vis:bestvis,fuel:bestfuel,end:arena[best].node}
}

fn polish_route(g:&Game,id:usize,net:&Network,sched:&Schedule,counts:&[u8],other:u32,seen:u32,original:&Route,rng:&mut Rng,deadline:Instant)->Route {
    #[derive(Clone,Copy)]
    struct Point {node:usize,until:u16,mode:usize}
    let mut points=Vec::<Point>::new();let mut pos=g.agents[id].pos;let mut time=0;
    for &a in &original.actions {
        if a<0 {
            time+=(-a) as u16;
            if time<sched.steps {
                let node=net.node(pos);
                if points.last().is_none_or(|p|p.node!=node){points.push(Point{node,until:time,mode:0});}else{points.last_mut().unwrap().until=time;}
            }
        }else{
            time+=net.duration[pos];pos=g.problem.map.neighbor(pos,a as usize).unwrap();
            if net.spot[pos]>=0 {let node=net.spot[pos] as usize;if points.last().is_none_or(|p|p.node!=node){points.push(Point{node,until:0,mode:0});}}
        }
    }
    let end=net.node(pos);if points.last().is_none_or(|p|p.node!=end){points.push(Point{node:end,until:0,mode:0});}
    let old=net.brand_mask(seen);let noise=vec![1.;32];
    let evaluate=|points:&[Point]|->Option<(Route,f64)>{
        let mut st=State{node:net.node(g.agents[id].pos),t:0,fuel:g.agents[id].fuel as u16,vis:0,gain:0.,parent:usize::MAX,variant:0,wait:0};
        let mut actions=Vec::new();
        for point in points {
            let path=net.path(st.node,point.node,point.mode);
            let wait=point.until.saturating_sub(st.t+path.time);
            let (t,fuel,vis)=travel(st,path,wait,net,sched,counts,&g.problem)?;
            actions.extend_from_slice(&path.dirs);if wait>0{actions.push(-i64::from(wait));}
            st=State{node:point.node,t,fuel,vis,..st};
        }
        let (score,fuel,vis)=terminal(st,net,sched,counts,g,other,old,&noise);
        let score=score+f64::from(sched.steps-st.t)*0.8;
        if st.t<sched.steps{actions.push(-i64::from(sched.steps-st.t));}
        Some((Route{actions,vis,fuel,end:st.node},score))
    };
    let os=State{node:original.end,t:sched.steps,fuel:original.fuel,vis:original.vis,gain:0.,parent:usize::MAX,variant:0,wait:0};
    let (mut best_value,_,_)=terminal(os,net,sched,counts,g,other,old,&noise);
    best_value+=original.actions.last().filter(|&&a|a<0).map_or(0.0,|&a|(-a) as f64*0.8);
    let mut best=original.clone();
    let mut current=f64::NEG_INFINITY;
    for mode in 0..net.variants {let mut proposal=points.clone();for p in &mut proposal{p.mode=mode;}if let Some((route,value))=evaluate(&proposal){if value>current{current=value;points=proposal;}if value>best_value{best_value=value;best=route;}}}
    for it in 0..350 {
        if it%64==0&&Instant::now()+Duration::from_millis(3)>=deadline {break}
        let mut trial=points.clone();
        match rng.at(7) {
            0|1=>{
                if trial.len()>40{continue}
                let node=rng.at(g.problem.spots.len());
                if trial.iter().any(|p|p.node==node){continue}
                let mut slots:Vec<(u16,usize)>=(0..=trial.len()).map(|k|{
                    let from=if k==0{net.node(g.agents[id].pos)}else{trial[k-1].node};
                    let mut cost=net.path(from,node,0).time;
                    if k<trial.len(){cost=(cost+net.path(node,trial[k].node,0).time).saturating_sub(net.path(from,trial[k].node,0).time);}
                    (cost,k)
                }).collect();slots.sort_unstable();
                let k=slots[rng.at(slots.len().min(3))].1;
                trial.insert(k,Point{node,until:0,mode:0});
            }
            2=>{if trial.len()<2{continue}let a=rng.at(trial.len());let b=rng.at(trial.len());let lo=a.min(b);let hi=a.max(b);trial[lo..=hi].reverse();}
            3=>{if trial.len()<2{continue}let a=rng.at(trial.len());let item=trial.remove(a);let b=rng.at(trial.len()+1);trial.insert(b,item);}
            4=>{if trial.is_empty(){continue}let k=rng.at(trial.len());trial.remove(k);}
            5=>{if trial.is_empty(){continue}let k=rng.at(trial.len());trial[k].mode=rng.at(net.variants);}
            _=>{if trial.is_empty(){continue}let k=rng.at(trial.len());if trial[k].until>0{trial[k].until=trial[k].until.saturating_sub(rng.at(5) as u16);}else{trial[k].node=rng.at(g.problem.spots.len());}}
        }
        if let Some((route,value))=evaluate(&trial){
            let temp=800.0*(1.0-it as f64/350.0)+50.0;
            if value>current||rng.unit()<((value-current)/temp).exp(){current=value;points=trial;}
            if value>best_value{best_value=value;best=route;}
        }
    }
    best
}

fn pack_routes(g:&Game,net:&Network,sched:&Schedule,patrols:&[usize],suppliers:&[usize],initial:&[Route],seen:u32,width:usize,rng:&mut Rng,deadline:Instant)->Vec<Route> {
    #[derive(Clone)]
    struct Choice {counts:[u8;32],brands:u32,value:f64,route:[usize;8]}
    let mut pools=Vec::new();let empty=vec![0;g.problem.spots.len()];let old=net.brand_mask(seen);
    for &i in patrols {
        let mut pool=vec![initial[i].clone()];let mut noise=vec![1.;32];
        for x in &mut noise{*x=0.72+rng.unit()*0.56;}
        patrol_route(g,i,net,sched,&empty,u32::MAX,seen,width*2,&noise,deadline,Some(&mut pool));pools.push(pool);
    }
    let mut beam=vec![Choice {counts:[0;32],brands:0,value:0.,route:[0;8]}];
    for (j,pool) in pools.iter().enumerate() {
        let mut next=Vec::new();
        for st in &beam {for(k,r)in pool.iter().enumerate(){
            let mut s=st.clone();let brands=net.brand_mask(r.vis);let added=brands&!s.brands;
            s.value+=(added&!old).count_ones() as f64*1000000.0+added.count_ones() as f64*100000.0;
            s.brands|=brands;s.route[j]=k;
            let mut bits=r.vis;while bits!=0 {let z=bits.trailing_zeros() as usize;bits&=bits-1;if s.counts[z]<g.problem.spots[z].stocks as u8{s.value+=1000.;}s.counts[z]+=1;}
            if g.day+1<g.problem.day_steps.len(){s.value+=reserve_value(g,r.fuel,!suppliers.is_empty())+net.density[r.end]*0.9;}
            s.value-=rescue_penalty(g,net,r.end,r.fuel,sched.ends.iter().map(|&j|net.path(j,r.end,0).time).min().unwrap_or(u16::MAX));
            next.push(s);
        }}
        next.sort_unstable_by(|a,b|b.value.total_cmp(&a.value));
        let mut masks=std::collections::HashSet::new();
        beam=next.into_iter().filter(|s|masks.insert((s.counts,s.route[j]))).take(512).collect();
    }
    let mut best=initial.to_vec();let mut value=solution_value(g,net,&best,seen,suppliers);
    for s in beam.iter().take(25) {
        let mut routes=initial.to_vec();for(j,&i)in patrols.iter().enumerate(){routes[i]=pools[j][s.route[j]].clone();}
        let v=solution_value(g,net,&routes,seen,suppliers);if v>value{value=v;best=routes;}
    }
    best
}

fn solution_value(g:&Game,net:&Network,routes:&[Route],seen:u32,suppliers:&[usize])->f64 {
    let mut vis=0;let mut counts=vec![0u32;g.problem.spots.len()];let mut fuel=0.;
    let future=if g.day+1<g.problem.day_steps.len(){1.0}else{0.0};
    for (i,r) in routes.iter().enumerate() {if g.agents[i].kind==0 {
        vis|=r.vis;for(s,c)in counts.iter_mut().enumerate(){if r.vis&(1u32<<s)!=0{*c+=1;}}
        fuel+=reserve_value(g,r.fuel,!suppliers.is_empty())+net.density[r.end]*0.9;
        fuel-=rescue_penalty(g,net,r.end,r.fuel,suppliers.iter().map(|&j|net.path(routes[j].end,r.end,0).time).min().unwrap_or(u16::MAX));
        let need=suppliers.iter().map(|&j|net.path(r.end,routes[j].end,2).fuel).min().unwrap_or(0);
        if need>r.fuel {fuel-=400.0*(need-r.fuel) as f64/g.problem.fuel_limits as f64;}
    }}
    let brands=net.brand_mask(vis);let old=net.brand_mask(seen);
    let udon:u32=counts.iter().enumerate().map(|(s,&c)|c.min(g.problem.spots[s].stocks)).sum();
    let position=if future>0.0 {placement_value(g,net,routes,suppliers)}else{0.};
    (brands&!old).count_ones() as f64*1000000.0+brands.count_ones() as f64*100000.0+udon as f64*1000.0+future*(fuel+position)
}

#[derive(Clone,Copy,PartialEq,Eq)]
struct Waypoint {node:usize,until:u16,mode:usize}

fn route_points(g:&Game,id:usize,net:&Network,route:&Route)->Vec<Waypoint> {
    let mut result=Vec::new();let mut pos=g.agents[id].pos;let mut time=0;
    let steps=g.problem.day_steps[g.day] as u16;
    let mut from=net.node(pos);let mut segment=Vec::new();
    for &a in &route.actions {
        if a<0 {
            time+=(-a) as u16;
            if time<steps&&net.nodes.contains(&pos) {
                let node=net.node(pos);
                if result.last().is_none_or(|p:&Waypoint|p.node!=node) {let mode=(0..net.variants).find(|&v|net.path(from,node,v).dirs==segment).unwrap_or(0);result.push(Waypoint{node,until:time,mode});from=node;segment.clear();}
                else {result.last_mut().unwrap().until=time;}
            }
        }else{
            time+=net.duration[pos];pos=g.problem.map.neighbor(pos,a as usize).unwrap();segment.push(a);
            if net.spot[pos]>=0 {let node=net.spot[pos] as usize;if result.last().is_none_or(|p|p.node!=node){let mode=(0..net.variants).find(|&v|net.path(from,node,v).dirs==segment).unwrap_or(0);result.push(Waypoint{node,until:0,mode});from=node;segment.clear();}}
        }
    }
    let end=net.node(pos);if result.last().is_none_or(|p|p.node!=end){let mode=(0..net.variants).find(|&v|net.path(from,end,v).dirs==segment).unwrap_or(0);result.push(Waypoint{node:end,until:0,mode});}
    result
}

fn faithful_points(g:&Game,id:usize,net:&Network,route:&Route)->Vec<Waypoint> {
    fn flush(g:&Game,net:&Network,start:usize,dirs:&[i64],out:&mut Vec<Waypoint>){
        let mut cells=vec![start];for &dir in dirs{cells.push(g.problem.map.neighbor(*cells.last().unwrap(),dir as usize).unwrap());}
        let mut k=0;while k<dirs.len(){
            let from=net.node(cells[k]);let mut choice=None;
            for j in (k+1..=dirs.len()).rev(){if let Some(node)=net.find_node(cells[j]){for mode in 0..net.variants{if net.path(from,node,mode).dirs==dirs[k..j]{choice=Some((j,node,mode));break}}if choice.is_some(){break}}}
            let (j,node,mode)=choice.expect("raw path anchors must preserve every segment");out.push(Waypoint{node,until:0,mode});k=j;
        }
    }
    let mut out=Vec::new();let mut dirs=Vec::new();let mut start=g.agents[id].pos;let mut pos=start;let mut time=0;
    for &a in &route.actions {
        if a<0 {
            flush(g,net,start,&dirs,&mut out);dirs.clear();start=pos;time+=(-a) as u16;
            if time<g.problem.day_steps[g.day] as u16{let node=net.node(pos);if out.last().is_some_and(|p|p.node==node){out.last_mut().unwrap().until=time;}else{out.push(Waypoint{node,until:time,mode:0});}}
        }else{time+=net.duration[pos];pos=g.problem.map.neighbor(pos,a as usize).unwrap();dirs.push(a);
            if net.spot[pos]>=0{flush(g,net,start,&dirs,&mut out);dirs.clear();start=pos;}
        }
    }
    flush(g,net,start,&dirs,&mut out);out
}

fn route_from_points(g:&Game,id:usize,net:&Network,sched:&Schedule,points:&[Waypoint])->Option<Route> {
    let p=&g.problem;let counts=[0u8;32];
    let mut st=State{node:net.node(g.agents[id].pos),t:0,fuel:g.agents[id].fuel as u16,vis:0,gain:0.,parent:usize::MAX,variant:0,wait:0};
    let mut actions=Vec::new();
    for point in points {
        let path=net.path(st.node,point.node,point.mode);
        let wait=point.until.saturating_sub(st.t+path.time).max(if path.time==0 {1}else{0});
        let(t,fuel,vis)=travel(st,path,wait,net,sched,&counts,p)?;
        actions.extend_from_slice(&path.dirs);if wait>0{actions.push(-i64::from(wait));}
        st=State{node:point.node,t,fuel,vis,..st};
    }
    let(_,fuel,vis)=terminal(st,net,sched,&counts,g,u32::MAX,u32::MAX,&[1.;32]);
    if st.t<sched.steps{actions.push(-i64::from(sched.steps-st.t));}
    Some(Route{actions,vis,fuel,end:st.node})
}

fn fleet_score(g:&Game,net:&Network,routes:&[Route],seen:u32,suppliers:&[usize],brand_weight:f64)->f64 {
    let mut counts=[0u8;32];let mut mask=0u32;let mut score=0.;
    let future=g.day+1<g.problem.day_steps.len();
    for (i,r) in routes.iter().enumerate(){if g.agents[i].kind==0 {
        let mut bits=r.vis;mask|=net.brand_mask(bits);while bits!=0{let z=bits.trailing_zeros() as usize;bits&=bits-1;counts[z]+=1;}
        if future {
            score+=reserve_value(g,r.fuel,!suppliers.is_empty())+net.density[r.end]*0.9-rescue_penalty(g,net,r.end,r.fuel,suppliers.iter().map(|&j|net.path(routes[j].end,r.end,0).time).min().unwrap_or(u16::MAX));
        }
        score+=r.actions.last().filter(|&&a|a<0).map_or(0.,|&a|-a as f64*1.5);
    }}
    for(s,&c)in counts.iter().enumerate().take(g.problem.spots.len()){score+=f64::from(c.min(g.problem.spots[s].stocks as u8))*1000.;}
    if future {score+=placement_value(g,net,routes,suppliers);}
    score+mask.count_ones() as f64*brand_weight+(mask&!net.brand_mask(seen)).count_ones() as f64*brand_weight*if brand_weight>50000.{10.}else{5.}
}

fn polish_fleet(g:&Game,net:&Network,sched:&Schedule,initial:&[Route],seen:u32,suppliers:&[usize],rng:&mut Rng,deadline:Instant,advanced:bool,mut alternatives:Option<&mut Vec<Solution>>)->Vec<Route> {
    let ids:Vec<_>=(0..g.agents.len()).filter(|&i|g.agents[i].kind==0).collect();
    let mut points:Vec<_>=(0..g.agents.len()).map(|i|route_points(g,i,net,&initial[i])).collect();
    let mut current=initial.to_vec();
    let mut sched=sched.clone();
    let mut best=current.clone();let mut bestvalue=solution_value(g,net,&best,seen,suppliers);
    for &i in &ids {
        let mut value=f64::NEG_INFINITY;let mut chosen=None;
        for mode in 0..=net.variants {
            let mut ps=points[i].clone();if mode<net.variants{for p in &mut ps{p.mode=mode;}}
            if let Some(route)=route_from_points(g,i,net,&sched,&ps){
                let v=route.vis.count_ones() as f64*1000.+reserve_value(g,route.fuel,!suppliers.is_empty());
                if v>value{value=v;chosen=Some((route,ps));}
            }
        }
        if let Some((route,ps))=chosen{current[i]=route;points[i]=ps;}
    }
    let began=Instant::now();let total=deadline.saturating_duration_since(began).as_secs_f64().max(0.001);
    let mut it=0;let mut accepted=0;
    while Instant::now()<deadline {
        let cycle=if g.agents.len()>5{(began.elapsed().as_secs_f64()/total*4.).fract()}else{(it%(ids.len()*10000)) as f64/(ids.len()*10000) as f64};
        let brand_weight=7000.+cycle*10000.;
        let temp=1800.*(1.-cycle).powi(2)+40.;
        if !suppliers.is_empty()&&rng.at(16)==0 {
            let id=suppliers[rng.at(suppliers.len())];let mut ps=points[id].clone();
            if ps.is_empty(){ps.push(Waypoint{node:net.node(g.agents[id].pos),until:0,mode:0});}
            match rng.at(6){
                0=>{let k=rng.at(ps.len());ps[k].node=rng.at(net.n);}
                1=>{if ps.len()<16{let k=rng.at(ps.len()+1);ps.insert(k,Waypoint{node:rng.at(net.n),until:0,mode:0});}}
                2=>{let k=rng.at(ps.len());ps.remove(k);}
                3=>{let k=rng.at(ps.len());ps[k].until=ps[k].until.saturating_add(rng.at(17) as u16).saturating_sub(8);}
                4=>{let k=rng.at(ps.len());let j=rng.at(ps.len());ps.swap(k,j);}
                _=>{let k=rng.at(ps.len());let car=ids[rng.at(ids.len())];ps[k].node=current[car].end;ps[k].until=0;}
            }
            let Some(actions)=supply_points(g,id,net,&ps)else{continue};
            let actions:Vec<_>=suppliers.iter().map(|&j|if j==id{actions.clone()}else{current[j].actions.clone()}).collect();
            let (proposed,rs)=raw_schedule(g,net,suppliers,&actions);let mut routes=current.clone();
            for (&j,r)in suppliers.iter().zip(rs){routes[j]=r;}
            let mut valid=true;for &j in &ids{if let Some(r)=reschedule_route(g,j,net,&proposed,&routes[j]){routes[j]=r;}else{valid=false;break}}
            if valid {
                let delta=fleet_score(g,net,&routes,seen,suppliers,brand_weight)-fleet_score(g,net,&current,seen,suppliers,brand_weight);
                if delta>=0.||rng.unit()<(delta/temp).exp(){
                    current=routes;sched=proposed;points[id]=ps;
                    let value=solution_value(g,net,&current,seen,suppliers);if value>bestvalue{bestvalue=value;best=current.clone();}
                }
            }
            continue
        }
        let a=ids[rng.at(ids.len())];let b=ids[rng.at(ids.len())];
        let mut pa=points[a].clone();let mut pb=points[b].clone();let mut changed_b=false;
        match rng.at(12) {
            0|1|2=>{
                if pa.len()>40{continue}
                let node=rng.at(g.problem.spots.len());
                if pa.iter().any(|p|p.node==node){continue}
                let mut slots:Vec<_>=(0..=pa.len()).map(|k|{
                    let from=if k==0{net.node(g.agents[a].pos)}else{pa[k-1].node};
                    let cost=if k==pa.len(){net.path(from,node,0).time}else{(net.path(from,node,0).time+net.path(node,pa[k].node,0).time).saturating_sub(net.path(from,pa[k].node,0).time)};
                    (cost,k)
                }).collect();slots.sort_unstable();
                let k=if rng.at(8)==0{rng.at(pa.len()+1)}else{slots[rng.at(slots.len().min(3))].1};
                pa.insert(k,Waypoint{node,until:0,mode:0});
            }
            3=>{if pa.is_empty(){continue}let k=rng.at(pa.len());pa.remove(k);}
            4=>{if pa.len()<2{continue}let k=rng.at(pa.len());let item=pa.remove(k);let j=rng.at(pa.len()+1);pa.insert(j,item);}
            5=>{if pa.len()<2{continue}let k=rng.at(pa.len());let j=rng.at(pa.len());pa[k.min(j)..=k.max(j)].reverse();}
            6=>{if pa.is_empty(){continue}let k=rng.at(pa.len());pa[k].mode=rng.at(net.variants);}
            7=>{if pa.is_empty(){continue}let k=rng.at(pa.len());pa[k].node=rng.at(g.problem.spots.len());pa[k].until=0;}
            8=>{if a==b||pa.is_empty()||pb.is_empty(){continue}let k=rng.at(pa.len());let j=rng.at(pb.len());std::mem::swap(&mut pa[k],&mut pb[j]);changed_b=true;}
            9=>{if a==b||pa.is_empty(){continue}let k=rng.at(pa.len());let point=pa.remove(k);let j=rng.at(pb.len()+1);pb.insert(j,point);changed_b=true;}
            10=>{if a==b||pa.is_empty()||pb.is_empty(){continue}let k=rng.at(pa.len()+1);let j=rng.at(pb.len()+1);let taila=pa.split_off(k);let tailb=pb.split_off(j);pa.extend(tailb);pb.extend(taila);changed_b=true;}
            _=>{if pa.is_empty(){continue}let k=rng.at(pa.len());if pa[k].until>0{pa[k].until=pa[k].until.saturating_add(rng.at(11) as u16).saturating_sub(5);}else{let j=rng.at(pa.len());pa.swap(k,j);}}
        }
        let Some(ra)=route_from_points(g,a,net,&sched,&pa)else{continue};
        let rb=if changed_b{let Some(r)=route_from_points(g,b,net,&sched,&pb)else{continue};Some(r)}else{None};
        let old=fleet_score(g,net,&current,seen,suppliers,brand_weight);
        let olda=std::mem::replace(&mut current[a],ra);
        let oldb=if let Some(rb)=rb{Some(std::mem::replace(&mut current[b],rb))}else{None};
        let new=fleet_score(g,net,&current,seen,suppliers,brand_weight);
        if new>=old||rng.unit()<((new-old)/temp).exp(){
            points[a]=pa;if changed_b{points[b]=pb;}accepted+=1;
            // The inexpensive score is a bound with the tiny idle preference removed.
            let strict=fleet_score(g,net,&current,seen,suppliers,100000.);
            let beststrict=fleet_score(g,net,&best,seen,suppliers,100000.);
            if strict>beststrict-3000. {
                let v=solution_value(g,net,&current,seen,suppliers);
                if let Some(pool)=alternatives.as_deref_mut(){save_alternative(pool,&current,v,bestvalue);}
                if v>bestvalue{bestvalue=v;best=current.clone();}
            }
        }else{current[a]=olda;if let Some(rb)=oldb{current[b]=rb;}}
        it+=1;
        if advanced&&it%25000==0&&Instant::now()+Duration::from_millis(20)<deadline {
            let actions:Vec<_>=suppliers.iter().map(|&i|best[i].actions.clone()).collect();
            let b_sched=raw_schedule(g,net,suppliers,&actions).0;
            let end=(Instant::now()+Duration::from_millis(90)).min(deadline);
            if let Some((candidate,ps))=rebuild_neighborhood(g,net,&b_sched,&best,seen,suppliers,rng,end){
                let value=solution_value(g,net,&candidate,seen,suppliers);
                if value>bestvalue{bestvalue=value;best=candidate.clone();}
                let delta=fleet_score(g,net,&candidate,seen,suppliers,brand_weight)-fleet_score(g,net,&current,seen,suppliers,brand_weight);
                if delta>=0.||rng.unit()<(delta/temp).exp(){current=candidate;points=ps;sched=b_sched;}
            }
        }
        if it%3000==0&&rng.at(2)==0 {
            current=best.clone();for i in 0..g.agents.len(){points[i]=route_points(g,i,net,&current[i]);}
            let actions:Vec<_>=suppliers.iter().map(|&i|current[i].actions.clone()).collect();sched=raw_schedule(g,net,suppliers,&actions).0;
        }
    }
    if total>0.5 {eprintln!("fleet trials={it} accept={accepted} value={bestvalue:.1}");}
    best
}


// Infeasible intermediate tours permit exchanges that need several edits. Only
// tours with no excess time and no borrowed fuel can replace the valid result.
fn relaxed_route_from_points(g:&Game,id:usize,net:&Network,sched:&Schedule,points:&[Waypoint])->(Route,[f64;2]) {
    let mut node=net.node(g.agents[id].pos);let mut pos=g.agents[id].pos;
    let mut t=0u16;let mut fuel=g.agents[id].fuel as u16;let mut debt=0u16;let mut vis=0u32;
    let mut actions=Vec::new();let counts=[0u8;32];
    for point in points {
        let path=net.path(node,point.node,point.mode);
        for &dir in &path.dirs {
            let duration=net.duration[pos];let cost=net.cost[pos];
            let dest=g.problem.map.neighbor(pos,dir as usize).unwrap();
            for dt in 1..=duration {
                t+=1;
                if dt==duration{debt+=cost.saturating_sub(fuel);fuel=fuel.saturating_sub(cost);pos=dest;}
                visit(pos,net,&counts,&g.problem,&mut vis);
                if t<=sched.steps&&sched.has(pos,t){fuel=g.problem.fuel_limits as u16;}
            }
            actions.push(dir);
        }
        let wait=point.until.saturating_sub(t).max(if path.time==0{1}else{0});
        for _ in 0..wait {t+=1;visit(pos,net,&counts,&g.problem,&mut vis);if t<=sched.steps&&sched.has(pos,t){fuel=g.problem.fuel_limits as u16;}}
        if wait>0{actions.push(-i64::from(wait));}node=point.node;
    }
    let over=t.saturating_sub(sched.steps);
    if t<sched.steps {
        actions.push(-i64::from(sched.steps-t));
        while t<sched.steps{t+=1;visit(pos,net,&counts,&g.problem,&mut vis);if sched.has(pos,t){fuel=g.problem.fuel_limits as u16;}}
    }
    (Route{actions,vis,fuel,end:node},[f64::from(over),f64::from(debt)])
}

fn soft_polish_fleet(g:&Game,net:&Network,sched:&Schedule,initial:&[Route],seen:u32,suppliers:&[usize],rng:&mut Rng,deadline:Instant,advanced:bool,mut alternatives:Option<&mut Vec<Solution>>,hint:Option<&[Vec<Waypoint>]>,exact_points:bool,trial_limit:Option<usize>)->Vec<Route> {
    let ids:Vec<_>=(0..g.agents.len()).filter(|&i|g.agents[i].kind==0).collect();
    let decode=|i:usize,r:&Route|if exact_points{faithful_points(g,i,net,r)}else{route_points(g,i,net,r)};
    let mut points:Vec<_>=(0..g.agents.len()).map(|i|decode(i,&initial[i])).collect();
    if exact_points {
        let (rebuilt,excess)=flexible_fleet(g,net,suppliers,&points);
        let canonical=|r:&Route|->Vec<i64>{r.actions.iter().flat_map(|&a|if a<0{vec![-1;(-a) as usize]}else{vec![a]}).collect()};
        let identical=rebuilt.iter().zip(initial).all(|(a,b)|canonical(a)==canonical(b));
        eprintln!("single faithful initial identical={identical} excess={excess:?}");
    }
    let mut current=initial.to_vec();
    let mut sched=sched.clone();
    let mut best=current.clone();let mut bestvalue=solution_value(g,net,&best,seen,suppliers);
    for &i in &ids {
        let mut value=f64::NEG_INFINITY;let mut chosen=None;
        for mode in 0..=net.variants {
            let mut ps=points[i].clone();if mode<net.variants{for p in &mut ps{p.mode=mode;}}
            if let Some(route)=route_from_points(g,i,net,&sched,&ps){
                let v=route.vis.count_ones() as f64*1000.+reserve_value(g,route.fuel,!suppliers.is_empty());
                if v>value{value=v;chosen=Some((route,ps));}
            }
        }
        if let Some((route,ps))=chosen{current[i]=route;points[i]=ps;}
    }
    if let Some(hint)=hint {
        points=hint.to_vec();
        let mut actions=Vec::new();
        for &i in suppliers {
            loop{if let Some(plan)=supply_points(g,i,net,&points[i]){actions.push(plan);break}points[i].pop();}
        }
        let (new_sched,rs)=raw_schedule(g,net,suppliers,&actions);sched=new_sched;
        for(&i,r)in suppliers.iter().zip(rs){current[i]=r;}
    }
    let mut excess=vec![[0.0f64;2];g.agents.len()];
    for &i in &ids {let (r,e)=relaxed_route_from_points(g,i,net,&sched,&points[i]);current[i]=r;excess[i]=e;}
    let mut penalties=[700.0f64,1000.0];let mut feasible=[0usize;2];let mut observations=0;
    let began=Instant::now();let total=deadline.saturating_duration_since(began).as_secs_f64().max(0.001);
    let mut it=0;let mut accepted=0;let mut attempts=0usize;
    while Instant::now()<deadline {
        if trial_limit.is_some_and(|limit|attempts>=limit){break}attempts+=1;
        let cycle=if let Some(limit)=trial_limit{attempts as f64/limit as f64}else if g.agents.len()>5{(began.elapsed().as_secs_f64()/total*4.).fract()}else{(it%(ids.len()*10000)) as f64/(ids.len()*10000) as f64};
        let brand_weight=7000.+cycle*10000.;
        let temp=1400.*(1.-cycle).powi(2)+20.;
        observations+=1;for k in 0..2{if excess.iter().all(|x|x[k]==0.){feasible[k]+=1;}}
        if observations>=400{for k in 0..2{penalties[k]=(penalties[k]*if feasible[k]<220{1.12}else{0.9}).clamp(150.,8000.);}observations=0;feasible=[0;2];}
        let penalty=|xs:&[[f64;2]]|->f64{xs.iter().map(|x|x[0]*penalties[0]+x[1]*penalties[1]).sum()};
        if !suppliers.is_empty()&&rng.at(12)==0 {
            let car=ids[rng.at(ids.len())];let supply=suppliers[rng.at(suppliers.len())];
            let mut cp=points[car].clone();let mut sp=points[supply].clone();
            if cp.is_empty()||rng.at(5)==0 {let start=net.node(g.agents[car].pos);if cp.first().is_none_or(|p|p.node!=start){cp.insert(0,Waypoint{node:start,until:0,mode:0});}}
            let mut arrival=Vec::new();let mut t=0u16;let mut node=net.node(g.agents[car].pos);
            for point in &cp{let path=net.path(node,point.node,point.mode);t=(t+path.time).max(point.until).max(t+1);arrival.push(t);node=point.node;}
            let k=rng.at(cp.len());let meeting=cp[k].node;
            if arrival[k]>sched.steps*4/5{continue}
            if sp.len()>=16||(sp.len()>1&&rng.at(2)==0) {let j=rng.at(sp.len());sp.remove(j);}
            let mut choices=Vec::new();let mut t=0u16;let mut node=net.node(g.agents[supply].pos);
            for j in 0..=sp.len(){
                let meet_time=(t+net.path(node,meeting,0).time).max(arrival[k]);
                let extra=if j==sp.len(){meet_time-t}else{(meet_time+net.path(meeting,sp[j].node,0).time).saturating_sub(t+net.path(node,sp[j].node,0).time)};
                choices.push((extra,j,meet_time));
                if j<sp.len(){t=(t+net.path(node,sp[j].node,0).time).max(sp[j].until);node=sp[j].node;}
            }
            choices.sort_unstable();let (_,j,meet_time)=choices[rng.at(choices.len().min(3))];
            if meet_time>=sched.steps{continue}
            cp[k].until=meet_time;sp.insert(j,Waypoint{node:meeting,until:meet_time,mode:0});
            let Some(actions)=supply_points(g,supply,net,&sp)else{continue};
            let actions:Vec<_>=suppliers.iter().map(|&j|if j==supply{actions.clone()}else{current[j].actions.clone()}).collect();
            let (proposed,rs)=raw_schedule(g,net,suppliers,&actions);let mut routes=current.clone();let mut proposed_excess=excess.clone();
            for (&j,r)in suppliers.iter().zip(rs){routes[j]=r;}
            for &j in &ids{let (r,e)=relaxed_route_from_points(g,j,net,&proposed,if j==car{&cp}else{&points[j]});routes[j]=r;proposed_excess[j]=e;}
            let delta=fleet_score(g,net,&routes,seen,suppliers,brand_weight)-fleet_score(g,net,&current,seen,suppliers,brand_weight)-penalty(&proposed_excess)+penalty(&excess);
            if delta>=0.||rng.unit()<(delta/temp).exp(){
                current=routes;sched=proposed;points[supply]=sp;points[car]=cp;excess=proposed_excess;
                if excess.iter().all(|x|x[0]==0.&&x[1]==0.){let value=solution_value(g,net,&current,seen,suppliers);if value>bestvalue{bestvalue=value;best=current.clone();}}
            }
            continue
        }
        if !suppliers.is_empty()&&rng.at(16)==0 {
            let id=suppliers[rng.at(suppliers.len())];let mut ps=points[id].clone();
            if ps.is_empty(){ps.push(Waypoint{node:net.node(g.agents[id].pos),until:0,mode:0});}
            match rng.at(6){
                0=>{let k=rng.at(ps.len());ps[k].node=rng.at(net.n);}
                1=>{if ps.len()<16{let k=rng.at(ps.len()+1);ps.insert(k,Waypoint{node:rng.at(net.n),until:0,mode:0});}}
                2=>{let k=rng.at(ps.len());ps.remove(k);}
                3=>{let k=rng.at(ps.len());ps[k].until=ps[k].until.saturating_add(rng.at(17) as u16).saturating_sub(8);}
                4=>{let k=rng.at(ps.len());let j=rng.at(ps.len());ps.swap(k,j);}
                _=>{let k=rng.at(ps.len());let car=ids[rng.at(ids.len())];ps[k].node=current[car].end;ps[k].until=0;}
            }
            let Some(actions)=supply_points(g,id,net,&ps)else{continue};
            let actions:Vec<_>=suppliers.iter().map(|&j|if j==id{actions.clone()}else{current[j].actions.clone()}).collect();
            let (proposed,rs)=raw_schedule(g,net,suppliers,&actions);let mut routes=current.clone();
            for (&j,r)in suppliers.iter().zip(rs){routes[j]=r;}
            let mut proposed_excess=excess.clone();for &j in &ids{let (r,e)=relaxed_route_from_points(g,j,net,&proposed,&points[j]);routes[j]=r;proposed_excess[j]=e;}
            {
                let delta=fleet_score(g,net,&routes,seen,suppliers,brand_weight)-fleet_score(g,net,&current,seen,suppliers,brand_weight)-penalty(&proposed_excess)+penalty(&excess);
                if delta>=0.||rng.unit()<(delta/temp).exp(){
                    current=routes;sched=proposed;points[id]=ps;excess=proposed_excess;
                    if excess.iter().all(|x|x[0]==0.&&x[1]==0.){let value=solution_value(g,net,&current,seen,suppliers);if value>bestvalue{bestvalue=value;best=current.clone();}}
                }
            }
            continue
        }
        let a=ids[rng.at(ids.len())];let b=ids[rng.at(ids.len())];
        let mut pa=points[a].clone();let mut pb=points[b].clone();let mut changed_b=false;
        match rng.at(if g.agents.len()>5{12}else{15}) {
            0|1|2=>{
                if pa.len()>40{continue}
                let node=rng.at(g.problem.spots.len());
                if pa.iter().any(|p|p.node==node){continue}
                let mut slots:Vec<_>=(0..=pa.len()).map(|k|{
                    let from=if k==0{net.node(g.agents[a].pos)}else{pa[k-1].node};
                    let cost=if k==pa.len(){net.path(from,node,0).time}else{(net.path(from,node,0).time+net.path(node,pa[k].node,0).time).saturating_sub(net.path(from,pa[k].node,0).time)};
                    (cost,k)
                }).collect();slots.sort_unstable();
                let k=if rng.at(8)==0{rng.at(pa.len()+1)}else{slots[rng.at(slots.len().min(3))].1};
                pa.insert(k,Waypoint{node,until:0,mode:0});
            }
            3=>{if pa.is_empty(){continue}let k=rng.at(pa.len());pa.remove(k);}
            4=>{if pa.len()<2{continue}let k=rng.at(pa.len());let item=pa.remove(k);let j=rng.at(pa.len()+1);pa.insert(j,item);}
            5=>{if pa.len()<2{continue}let k=rng.at(pa.len());let j=rng.at(pa.len());pa[k.min(j)..=k.max(j)].reverse();}
            6=>{if pa.is_empty(){continue}let k=rng.at(pa.len());pa[k].mode=rng.at(net.variants);}
            7=>{if pa.is_empty(){continue}let k=rng.at(pa.len());pa[k].node=rng.at(g.problem.spots.len());pa[k].until=0;}
            8=>{if a==b||pa.is_empty()||pb.is_empty(){continue}let k=rng.at(pa.len());let j=rng.at(pb.len());std::mem::swap(&mut pa[k],&mut pb[j]);changed_b=true;}
            9=>{if a==b||pa.is_empty(){continue}let k=rng.at(pa.len());let point=pa.remove(k);let j=rng.at(pb.len()+1);pb.insert(j,point);changed_b=true;}
            10=>{if a==b||pa.is_empty()||pb.is_empty(){continue}let k=rng.at(pa.len()+1);let j=rng.at(pb.len()+1);let taila=pa.split_off(k);let tailb=pb.split_off(j);pa.extend(tailb);pb.extend(taila);changed_b=true;}
            12=>{if a==b||pa.is_empty(){continue}let k=rng.at(pa.len());let length=(1+rng.at(4)).min(pa.len()-k);let block:Vec<_>=pa.drain(k..k+length).collect();let j=rng.at(pb.len()+1);pb.splice(j..j,block);changed_b=true;}
            13=>{if a==b{continue}std::mem::swap(&mut pa,&mut pb);changed_b=true;}
            14=>{if a==b||pa.is_empty()||pb.is_empty(){continue}let k=rng.at(pa.len());let j=rng.at(pb.len());let la=(1+rng.at(3)).min(pa.len()-k);let lb=(1+rng.at(3)).min(pb.len()-j);let ba:Vec<_>=pa.drain(k..k+la).collect();let bb:Vec<_>=pb.drain(j..j+lb).collect();pa.splice(k..k,bb);pb.splice(j..j,ba);changed_b=true;}
            _=>{if pa.is_empty(){continue}let k=rng.at(pa.len());if pa[k].until>0{pa[k].until=pa[k].until.saturating_add(rng.at(11) as u16).saturating_sub(5);}else{let j=rng.at(pa.len());pa.swap(k,j);}}
        }
        let (ra,ea)=relaxed_route_from_points(g,a,net,&sched,&pa);
        let rb=if changed_b{Some(relaxed_route_from_points(g,b,net,&sched,&pb))}else{None};
        let old=fleet_score(g,net,&current,seen,suppliers,brand_weight)-penalty(&excess);
        let old_ea=std::mem::replace(&mut excess[a],ea);
        let old_eb=if let Some((_,eb))=&rb{Some(std::mem::replace(&mut excess[b],*eb))}else{None};
        let olda=std::mem::replace(&mut current[a],ra);
        let oldb=if let Some((rb,_))=rb{Some(std::mem::replace(&mut current[b],rb))}else{None};
        let new=fleet_score(g,net,&current,seen,suppliers,brand_weight)-penalty(&excess);
        if new>=old||rng.unit()<((new-old)/temp).exp(){
            points[a]=pa;if changed_b{points[b]=pb;}accepted+=1;
            // The inexpensive score is a bound with the tiny idle preference removed.
            let strict=fleet_score(g,net,&current,seen,suppliers,100000.);
            let beststrict=fleet_score(g,net,&best,seen,suppliers,100000.);
            if strict>beststrict-3000.&&excess.iter().all(|x|x[0]==0.&&x[1]==0.) {
                let v=solution_value(g,net,&current,seen,suppliers);
                if let Some(pool)=alternatives.as_deref_mut(){save_alternative(pool,&current,v,bestvalue);}
                if v>bestvalue{bestvalue=v;best=current.clone();}
            }
        }else{current[a]=olda;excess[a]=old_ea;if let Some(rb)=oldb{current[b]=rb;excess[b]=old_eb.unwrap();}}
        it+=1;
        if advanced&&it%25000==0&&Instant::now()+Duration::from_millis(20)<deadline {
            let actions:Vec<_>=suppliers.iter().map(|&i|best[i].actions.clone()).collect();
            let b_sched=raw_schedule(g,net,suppliers,&actions).0;
            let end=(Instant::now()+Duration::from_millis(90)).min(deadline);
            if let Some((candidate,ps))=rebuild_neighborhood(g,net,&b_sched,&best,seen,suppliers,rng,end){
                let value=solution_value(g,net,&candidate,seen,suppliers);
                if value>bestvalue{bestvalue=value;best=candidate.clone();}
                let delta=fleet_score(g,net,&candidate,seen,suppliers,brand_weight)-fleet_score(g,net,&current,seen,suppliers,brand_weight)+penalty(&excess);
                if delta>=0.||rng.unit()<(delta/temp).exp(){current=candidate;points=ps;sched=b_sched;excess.fill([0.;2]);}
            }
        }
        if if g.agents.len()>5{it%20000==0&&rng.at(2)==0}else{it%(ids.len()*10000)==0} {
            excess.fill([0.;2]);current=best.clone();for i in 0..g.agents.len(){points[i]=decode(i,&current[i]);}
            let actions:Vec<_>=suppliers.iter().map(|&i|current[i].actions.clone()).collect();sched=raw_schedule(g,net,suppliers,&actions).0;
            for &i in &ids{let(r,e)=relaxed_route_from_points(g,i,net,&sched,&points[i]);current[i]=r;excess[i]=e;}
        }
    }
    if total>0.5 {eprintln!("soft fleet trials={it} accept={accepted} value={bestvalue:.1}");}
    best
}

fn flexible_supply(g:&Game,net:&Network,ids:&[usize],points:&[Vec<Waypoint>])->(Schedule,Vec<Route>,u32) {
    let steps=g.problem.day_steps[g.day] as u16;let len=g.problem.map.len();
    let mut sched=Schedule{present:vec![false;(steps as usize+1)*len],windows:Vec::new(),len,steps,ends:Vec::new()};
    let mut routes=Vec::new();let mut excess=0;
    for &id in ids {
        let mut node=net.node(g.agents[id].pos);let mut pos=g.agents[id].pos;let mut t=0u16;let mut actions=Vec::new();
        for point in &points[id] {
            let path=net.path(node,point.node,point.mode);
            for &dir in &path.dirs {
                let duration=net.duration[pos];let dest=g.problem.map.neighbor(pos,dir as usize).unwrap();
                for dt in 1..=duration{if t+dt<=steps{sched.present[(t+dt) as usize*len+if dt==duration{dest}else{pos}]=true;}}
                t+=duration;pos=dest;actions.push(dir);
            }
            let wait=point.until.saturating_sub(t);
            for dt in 1..=wait{if t+dt<=steps{sched.present[(t+dt) as usize*len+pos]=true;}}
            if wait>0{actions.push(-i64::from(wait));t+=wait;}node=point.node;
        }
        excess+=u32::from(t.saturating_sub(steps));
        if t<steps{for tt in t+1..=steps{sched.present[tt as usize*len+pos]=true;}actions.push(-i64::from(steps-t));}
        sched.ends.push(node);routes.push(Route{actions,vis:0,fuel:g.problem.fuel_limits as u16,end:node});
    }
    (sched,routes,excess)
}

fn flexible_fleet(g:&Game,net:&Network,ids:&[usize],points:&[Vec<Waypoint>])->(Vec<Route>,[f64;2]) {
    let (sched,supply,over)=flexible_supply(g,net,ids,points);
    let mut result=vec![Route{actions:Vec::new(),vis:0,fuel:0,end:0};g.agents.len()];let mut excess=[over as f64,0.];
    for(&i,r)in ids.iter().zip(supply){result[i]=r;}
    for i in 0..g.agents.len(){if g.agents[i].kind==0{let(r,e)=relaxed_route_from_points(g,i,net,&sched,&points[i]);result[i]=r;excess[0]+=e[0];excess[1]+=e[1];}}
    (result,excess)
}

// A search keeps traffic and day length fixed. Reuse a route only while
// its start state, waypoints, and supplier timetable are unchanged.
#[derive(Clone)]
struct CachedFleet {
    routes: Vec<Route>,
    debts: Vec<[f64;2]>,
    supply_over: u32,
    schedule: std::rc::Rc<Schedule>,
    starts: Vec<(usize,u32)>,
}
impl CachedFleet {
    fn excess(&self)->[f64;2] {
        let mut total=[self.supply_over as f64,0.];
        for debt in &self.debts{total[0]+=debt[0];total[1]+=debt[1];}total
    }
}
fn cached_fleet(g:&Game,net:&Network,ids:&[usize],points:&[Vec<Waypoint>],old:Option<(&CachedFleet,&[Vec<Waypoint>])>)->CachedFleet {
    let supply_changed=old.is_none_or(|(previous,before)|ids.iter().any(|&i|previous.starts[i].0!=g.agents[i].pos||before[i]!=points[i]));
    let starts=g.agents.iter().map(|a|(a.pos,a.fuel)).collect();
    let (schedule,supply,supply_over)=if supply_changed {
        let (schedule,routes,over)=flexible_supply(g,net,ids,points);(std::rc::Rc::new(schedule),routes,over)
    }else{let previous=old.unwrap().0;(previous.schedule.clone(),ids.iter().map(|&i|previous.routes[i].clone()).collect(),previous.supply_over)};
    let mut routes=old.map_or_else(||vec![Route{actions:Vec::new(),vis:0,fuel:0,end:0};g.agents.len()],|(previous,_)|previous.routes.clone());
    let mut debts=old.map_or_else(||vec![[0.;2];g.agents.len()],|(previous,_)|previous.debts.clone());
    for (&i,r) in ids.iter().zip(supply){routes[i]=r;}
    for i in 0..g.agents.len(){if g.agents[i].kind==0&&
        (supply_changed||old.is_none_or(|(previous,before)|previous.starts[i]!=(g.agents[i].pos,g.agents[i].fuel)||before[i]!=points[i])) {
        let(r,e)=relaxed_route_from_points(g,i,net,&schedule,&points[i]);routes[i]=r;debts[i]=e;
    }}
    CachedFleet{routes,debts,supply_over,schedule,starts}
}

fn set_future_agents(g:&mut Game,net:&Network,routes:&[Route]) {
    for(i,r)in routes.iter().enumerate(){g.agents[i].pos=net.nodes[r.end];g.agents[i].fuel=u32::from(r.fuel);}
}

fn mutate_tours(g:&Game,net:&Network,points:&mut [Vec<Waypoint>],rng:&mut Rng) {
    let ids:Vec<_>=(0..g.agents.len()).filter(|&i|g.agents[i].kind==0).collect();
    let supplies:Vec<_>=(0..g.agents.len()).filter(|&i|g.agents[i].kind==1).collect();
    if !supplies.is_empty()&&rng.at(10)==0 {
        let a=supplies[rng.at(supplies.len())];let ps=&mut points[a];
        if ps.is_empty(){ps.push(Waypoint{node:net.node(g.agents[a].pos),until:0,mode:0});}
        let k=rng.at(ps.len());
        match rng.at(5){
            0=>ps[k].node=rng.at(net.n),
            1=>{if ps.len()<16{ps.insert(k,Waypoint{node:rng.at(net.n),until:0,mode:0});}},
            2=>{ps.remove(k);},
            3=>{let mut t=0u16;let mut node=net.node(g.agents[a].pos);for p in &ps[..=k]{t=(t+net.path(node,p.node,0).time).max(p.until);node=p.node;}ps[k].until=t.saturating_add(rng.at(9) as u16).saturating_sub(4);},
            _=>{let j=rng.at(ps.len());ps.swap(k,j);}
        }
        return
    }
    if !supplies.is_empty()&&rng.at(12)==0 {
        let a=ids[rng.at(ids.len())];let b=supplies[rng.at(supplies.len())];let mut pa=points[a].clone();let mut pb=points[b].clone();
        if pa.is_empty()||rng.at(5)==0{let node=net.node(g.agents[a].pos);if pa.first().is_none_or(|p|p.node!=node){pa.insert(0,Waypoint{node,until:0,mode:0});}}
        let k=rng.at(pa.len());let mut t=0u16;let mut node=net.node(g.agents[a].pos);
        for p in &pa[..=k]{let path=net.path(node,p.node,p.mode);t=(t+path.time).max(p.until).max(t+1);node=p.node;}
        if pb.len()>=16||(pb.len()>1&&rng.at(2)==0){let j=rng.at(pb.len());pb.remove(j);}
        let mut choices=Vec::new();let mut bt=0u16;let mut from=net.node(g.agents[b].pos);
        for j in 0..=pb.len(){let at=(bt+net.path(from,node,0).time).max(t);let cost=if j==pb.len(){at-bt}else{(at+net.path(node,pb[j].node,0).time).saturating_sub(bt+net.path(from,pb[j].node,0).time)};choices.push((cost,j,at));if j<pb.len(){bt=(bt+net.path(from,pb[j].node,0).time).max(pb[j].until);from=pb[j].node;}}
        choices.sort_unstable();let(_,j,at)=choices[rng.at(choices.len().min(3))];
        if at>=g.problem.day_steps[g.day] as u16{return}pa[k].until=at;pb.insert(j,Waypoint{node,until:at,mode:0});points[a]=pa;points[b]=pb;return
    }
    let a=ids[rng.at(ids.len())];let b=ids[rng.at(ids.len())];let mut pa=points[a].clone();let mut pb=points[b].clone();let mut change_b=false;
    match rng.at(15){
        0|1|2=>{
            if pa.len()>40{return}let node=rng.at(g.problem.spots.len());if pa.iter().any(|p|p.node==node){return}
            let mut slots:Vec<_>=(0..=pa.len()).map(|k|{let from=if k==0{net.node(g.agents[a].pos)}else{pa[k-1].node};let cost=if k==pa.len(){net.path(from,node,0).time}else{(net.path(from,node,0).time+net.path(node,pa[k].node,0).time).saturating_sub(net.path(from,pa[k].node,0).time)};(cost,k)}).collect();slots.sort_unstable();let k=slots[rng.at(slots.len().min(3))].1;pa.insert(k,Waypoint{node,until:0,mode:0});
        }
        3=>{if pa.is_empty(){return}let k=rng.at(pa.len());pa.remove(k);}
        4=>{if pa.len()<2{return}let k=rng.at(pa.len());let p=pa.remove(k);let j=rng.at(pa.len()+1);pa.insert(j,p);}
        5=>{if pa.len()<2{return}let k=rng.at(pa.len());let j=rng.at(pa.len());pa[k.min(j)..=k.max(j)].reverse();}
        6=>{if pa.is_empty(){return}let k=rng.at(pa.len());pa[k].mode=rng.at(net.variants);}
        7=>{if pa.is_empty(){return}let k=rng.at(pa.len());pa[k].node=rng.at(g.problem.spots.len());pa[k].until=0;}
        8=>{if a==b||pa.is_empty()||pb.is_empty(){return}let k=rng.at(pa.len());let j=rng.at(pb.len());std::mem::swap(&mut pa[k],&mut pb[j]);change_b=true;}
        9|12=>{if a==b||pa.is_empty(){return}let k=rng.at(pa.len());let length=(1+rng.at(4)).min(pa.len()-k);let block:Vec<_>=pa.drain(k..k+length).collect();let j=rng.at(pb.len()+1);pb.splice(j..j,block);change_b=true;}
        10=>{if a==b{return}let k=rng.at(pa.len()+1);let j=rng.at(pb.len()+1);let taila=pa.split_off(k);let tailb=pb.split_off(j);pa.extend(tailb);pb.extend(taila);change_b=true;}
        13=>{if a==b{return}std::mem::swap(&mut pa,&mut pb);change_b=true;}
        14=>{if a==b||pa.is_empty()||pb.is_empty(){return}let k=rng.at(pa.len());let j=rng.at(pb.len());let la=(1+rng.at(3)).min(pa.len()-k);let lb=(1+rng.at(3)).min(pb.len()-j);let ba:Vec<_>=pa.drain(k..k+la).collect();let bb:Vec<_>=pb.drain(j..j+lb).collect();pa.splice(k..k,bb);pb.splice(j..j,ba);change_b=true;}
        _=>{if pa.is_empty(){return}let k=rng.at(pa.len());if pa[k].until>0{pa[k].until=pa[k].until.saturating_add(rng.at(11) as u16).saturating_sub(5);}else{let j=rng.at(pa.len());pa.swap(k,j);}}
    }
    points[a]=pa;if change_b{points[b]=pb;}
}

fn two_day_polish(g:&Game,next:&mut Game,net:&Network,next_net:&Network,initial:&[Route],initial_next:&[Route],seen:u32,rng:&mut Rng,deadline:Instant)->(Vec<Route>,Vec<Vec<Waypoint>>) {
    let supplies:Vec<_>=(0..g.agents.len()).filter(|&i|g.agents[i].kind==1).collect();
    let mut score_game=g.clone();score_game.problem.day_steps.truncate(g.day+1);
    let pair_score=|next:&Game,a:&[Route],b:&[Route],weight:f64|->f64{
        let mut visits=seen;for(i,r)in a.iter().enumerate(){if g.agents[i].kind==0{visits|=r.vis;}}
        fleet_score(&score_game,net,a,seen,&supplies,weight)+fleet_score(next,next_net,b,visits,&supplies,weight)
    };
    let mut points=vec![(0..g.agents.len()).map(|i|faithful_points(g,i,net,&initial[i])).collect::<Vec<_>>(),(0..g.agents.len()).map(|i|faithful_points(next,i,next_net,&initial_next[i])).collect::<Vec<_>>()];
    let current_brands=|rs:&[Route]|->u32{rs.iter().enumerate().filter(|(i,_)|g.agents[*i].kind==0).fold(0,|mask,(_,r)|mask|net.brand_mask(r.vis))};
    let required=current_brands(initial).count_ones();let required_new=(current_brands(initial)&!net.brand_mask(seen)).count_ones();
    let mut best=initial.to_vec();let mut best_next=initial_next.to_vec();
    for(i,r)in best.iter_mut().enumerate(){let mut pos=g.agents[i].pos;for &a in &r.actions{if a>=0{pos=g.problem.map.neighbor(pos,a as usize).unwrap();}}r.end=net.node(pos);}
    for(i,r)in best_next.iter_mut().enumerate(){let mut pos=next.agents[i].pos;for &a in &r.actions{if a>=0{pos=next.problem.map.neighbor(pos,a as usize).unwrap();}}r.end=next_net.node(pos);}
    let mut bestvalue=pair_score(next,&best,&best_next,100000.);let mut best_points=points.clone();
    let mut cache0=cached_fleet(g,net,&supplies,&points[0],None);
    let(mut current,mut e0)=(cache0.routes.clone(),cache0.excess());set_future_agents(next,net,&current);
    let mut cache1=cached_fleet(next,next_net,&supplies,&points[1],None);
    let(mut future,mut e1)=(cache1.routes.clone(),cache1.excess());
    let canonical=|r:&Route|->Vec<i64>{r.actions.iter().flat_map(|&a|if a<0{vec![-1;(-a) as usize]}else{vec![a]}).collect()};
    let identical=current.iter().zip(initial).chain(future.iter().zip(initial_next)).all(|(a,b)|canonical(a)==canonical(b));
    eprintln!("faithful initial identical={identical} e0={e0:?} e1={e1:?}");
    let period=(g.agents.len()-supplies.len())*12000;let mut penalties=[700.0f64,1200.0];let mut feasible=[0usize;2];let mut it=0usize;let mut accepted=0;
    while Instant::now()<deadline{
        let cycle=(it%period) as f64/period as f64;let temp=1700.*(1.-cycle).powi(2)+30.;let bw=9000.+cycle*14000.;
        for k in 0..2{if e0[k]+e1[k]==0.{feasible[k]+=1;}}
        if it%500==499{for k in 0..2{penalties[k]=(penalties[k]*if feasible[k]<250{1.12}else{0.9}).clamp(150.,10000.);}feasible=[0;2];}
        let old=pair_score(next,&current,&future,bw)-(e0[0]+e1[0])*penalties[0]-(e0[1]+e1[1])*penalties[1];
        let day=usize::from(rng.at(10)>=6);let mut trial=points.clone();
        let cross=rng.at(8)==0;
        if cross {
            let a=rng.at(g.agents.len());
            match rng.at(3){
                0=>{if !trial[0][a].is_empty(){let n=(1+rng.at(3)).min(trial[0][a].len());let k=trial[0][a].len()-n;let moved:Vec<_>=trial[0][a].drain(k..).map(|p|Waypoint{node:next_net.node(net.nodes[p.node]),until:0,mode:p.mode}).collect();trial[1][a].splice(0..0,moved);}},
                1=>{if !trial[1][a].is_empty(){let n=(1+rng.at(3)).min(trial[1][a].len());let moved:Vec<_>=trial[1][a].drain(..n).filter_map(|p|net.find_node(next_net.nodes[p.node]).map(|node|Waypoint{node,until:0,mode:p.mode})).collect();trial[0][a].extend(moved);}},
                _=>{let b=rng.at(g.agents.len());if g.agents[a].kind==g.agents[b].kind{trial[0].swap(a,b);trial[1].swap(a,b);}}
            }
        }else{mutate_tours(if day==0{g}else{next},if day==0{net}else{next_net},&mut trial[day],rng);}
        let proposed0=if day==0||cross{Some(cached_fleet(g,net,&supplies,&trial[0],Some((&cache0,&points[0]))))}else{None};
        let (r0,x0)=proposed0.as_ref().map_or_else(||(current.clone(),e0),|f|(f.routes.clone(),f.excess()));set_future_agents(next,net,&r0);
        let proposed1=cached_fleet(next,next_net,&supplies,&trial[1],Some((&cache1,&points[1])));
        let (r1,x1)=(proposed1.routes.clone(),proposed1.excess());
        let value=pair_score(next,&r0,&r1,bw)-(x0[0]+x1[0])*penalties[0]-(x0[1]+x1[1])*penalties[1];
        if value>=old||rng.unit()<((value-old)/temp).exp(){
            points=trial;current=r0;future=r1;e0=x0;e1=x1;if let Some(c)=proposed0{cache0=c;}cache1=proposed1;accepted+=1;
            if x0[0]+x0[1]+x1[0]+x1[1]==0.&&current_brands(&current).count_ones()>=required&&(current_brands(&current)&!net.brand_mask(seen)).count_ones()>=required_new{let value=pair_score(next,&current,&future,100000.);if value>bestvalue{bestvalue=value;best=current.clone();best_next=future.clone();best_points=points.clone();}}
        }else{set_future_agents(next,net,&current);}
        it+=1;
        if it%period==0{points=best_points.clone();current=best.clone();future=best_next.clone();e0=[0.;2];e1=[0.;2];set_future_agents(next,net,&current);cache0=cached_fleet(g,net,&supplies,&points[0],None);cache1=cached_fleet(next,next_net,&supplies,&points[1],None);}
    }
    set_future_agents(next,net,&best);
    eprintln!("two-day trials={it} accepted={accepted} value={bestvalue:.1}");(best,best_points.remove(1))
}

fn reschedule_route(g:&Game,id:usize,net:&Network,sched:&Schedule,route:&Route)->Option<Route> {
    let mut pos=g.agents[id].pos;let mut t=0u16;let mut fuel=g.agents[id].fuel as u16;let mut vis=0u32;let counts=[0u8;32];
    for &a in &route.actions {
        let dur=if a<0{(-a) as u16}else{net.duration[pos]};
        let cost=if a<0{0}else{net.cost[pos]};
        let dest=if a<0{pos}else{g.problem.map.neighbor(pos,a as usize)?};
        for dt in 1..=dur {
            t+=1;if dt==dur{if fuel<cost{return None}fuel-=cost;pos=dest;}
            visit(pos,net,&counts,&g.problem,&mut vis);
            if sched.has(pos,t){fuel=g.problem.fuel_limits as u16;}
        }
    }
    Some(Route{actions:route.actions.clone(),vis,fuel,end:net.node(pos)})
}

fn supply_points(g:&Game,id:usize,net:&Network,points:&[Waypoint])->Option<Vec<i64>> {
    let steps=g.problem.day_steps[g.day] as u16;let mut t=0;let mut node=net.node(g.agents[id].pos);let mut result=Vec::new();
    for point in points {
        let path=net.path(node,point.node,if g.agents.len()<=5{point.mode}else{0});let wait=point.until.saturating_sub(t+path.time);
        if t+path.time+wait>steps{return None}
        result.extend_from_slice(&path.dirs);if wait>0{result.push(-i64::from(wait));}
        t+=path.time+wait;node=point.node;
    }
    if t<steps{result.push(-i64::from(steps-t));}Some(result)
}

fn placement_value(g:&Game,net:&Network,routes:&[Route],suppliers:&[usize])->f64 {
    let mut distances=[u16::MAX;32];
    for (i,r)in routes.iter().enumerate(){if g.agents[i].kind==0{
        let support=suppliers.iter().map(|&j|net.path(routes[j].end,r.end,0).time).min().unwrap_or(999);
        for spot in 0..g.problem.spots.len(){
            let path=net.path(r.end,spot,0);let cheap=net.path(r.end,spot,2);
            let time=if path.fuel<=r.fuel{path.time}else if cheap.fuel<=r.fuel{cheap.time}else{support+path.time};
            let b=net.brands[spot].trailing_zeros() as usize;distances[b]=distances[b].min(time);
        }
    }}
    let next=g.problem.day_steps[g.day+1] as f64;
    distances.iter().filter(|&&x|x<u16::MAX).map(|&x|{-8.*f64::from(x)-200.*(f64::from(x)-next*0.65).max(0.)}).sum()
}


// Remove several visits together and reinsert them where their marginal value
// per travel time is highest. This changes territories between collectors.
fn rebuild_neighborhood(g:&Game,net:&Network,sched:&Schedule,initial:&[Route],seen:u32,suppliers:&[usize],rng:&mut Rng,deadline:Instant)->Option<(Vec<Route>,Vec<Vec<Waypoint>>)> {
    let mut routes=initial.to_vec();let mut points:Vec<_>=(0..g.agents.len()).map(|i|route_points(g,i,net,&routes[i])).collect();
    let mut ids:Vec<_>=(0..g.agents.len()).filter(|&i|g.agents[i].kind==0).collect();rng.shuffle(&mut ids);
    ids.truncate((2+rng.at(2)).min(ids.len()));
    for &i in &ids {
        let mut trial=points[i].clone();
        let drops=(trial.len()/3).max(1).min(7);
        for _ in 0..drops {if trial.is_empty(){break}let k=rng.at(trial.len());if trial[k].until==0{trial.remove(k);}}
        if let Some(route)=route_from_points(g,i,net,sched,&trial){routes[i]=route;points[i]=trial;}
    }
    let mut noise=vec![1.;g.problem.spots.len()];for x in &mut noise{*x=0.65+0.7*rng.unit();}
    for _ in 0..28 {
        if Instant::now()+Duration::from_millis(1)>=deadline{break}
        let before=fleet_score(g,net,&routes,seen,suppliers,100000.);
        let mut choice=None;let mut best=f64::NEG_INFINITY;
        for &i in &ids {
            for node in 0..g.problem.spots.len(){
                if points[i].iter().any(|p|p.node==node){continue}
                let mut slots:Vec<_>=(0..=points[i].len()).map(|k|{
                    let from=if k==0{net.node(g.agents[i].pos)}else{points[i][k-1].node};
                    let cost=if k==points[i].len(){net.path(from,node,0).time}else{(net.path(from,node,0).time+net.path(node,points[i][k].node,0).time).saturating_sub(net.path(from,points[i][k].node,0).time)};
                    (cost,k)
                }).collect();slots.sort_unstable();
                for &(cost,k) in slots.iter().take(2) {
                    let mut trial=points[i].clone();trial.insert(k,Waypoint{node,until:0,mode:0});
                    let Some(route)=route_from_points(g,i,net,sched,&trial)else{continue};
                    let old=std::mem::replace(&mut routes[i],route);
                    let delta=fleet_score(g,net,&routes,seen,suppliers,100000.)-before;
                    let value=delta*noise[node]/(f64::from(cost)+4.).powf(0.72);
                    let route=std::mem::replace(&mut routes[i],old);
                    if delta>50.&&value>best{best=value;choice=Some((i,trial,route));}
                }
            }
            if Instant::now()>=deadline{break}
        }
        if let Some((i,ps,route))=choice{points[i]=ps;routes[i]=route;}else{break}
    }
    Some((routes,points))
}


fn save_alternative(pool:&mut Vec<Solution>,routes:&[Route],value:f64,best:f64) {
    if value<best-3500. {return}
    if let Some(old)=pool.iter_mut().find(|x|x.routes.iter().zip(routes).all(|(a,b)|a.end==b.end)) {
        if value>old.value{old.routes=routes.to_vec();old.value=value;}return
    }
    pool.push(Solution{routes:routes.to_vec(),support:Vec::new(),raw:None,value});
    if pool.len()>40 {
        pool.sort_by(|a,b|b.value.total_cmp(&a.value));
        let mut levels=HashMap::<i64,usize>::new();pool.retain(|x|{if x.value<best-3500.{return false}let c=levels.entry((x.value/700.).floor() as i64).or_default();*c+=1;*c<=8});pool.truncate(32);
    }
}
