//! C011: primary-only, team-level terminal placement guidance.
//! The graph is retained by Rc so a pointer cannot be reused during this table's
//! lifetime. No node indices cross restore, request, or graph boundaries.
use super::*;

pub(super) struct TerminalReach {
    graph: Rc<Graph>,
    day: usize,
    steps: i32,
    supplier: usize,
    agents: usize,
    brands: u64,
    near: Vec<u64>,
    full: Vec<u64>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct ReachStats {
    pub(super) far: u32,
    pub(super) thin: u32,
    pub(super) penalty: f64,
}
pub(super) struct BeamReach<'a> {
    table: &'a TerminalReach,
    near: u64,
    full: u64,
    pub(super) original: f64,
}
impl BeamReach<'_> {
    pub(super) fn at(&self, node: usize) -> f64 {
        self.table.from_masks(self.near | self.table.near[node], self.full | self.table.full[node]).penalty
    }
}
impl TerminalReach {
    pub(super) fn build(p: &Problem, obs: &Observation, steps: i32, graph: Rc<Graph>) -> Option<Self> {
        let tomorrow = *p.day_steps.get(obs.day + 1)?;
        if tomorrow == 0 || steps <= 0 || obs.agents.len() != p.agents.len()
            || obs.agents.iter().filter(|a| a.kind == 1).count() != 1
            || obs.agents.iter().any(|a| a.kind > 1) { return None; }
        let supplier = obs.agents.iter().position(|a| a.kind == 1)?;
        let (brands,near,full) = Self::masks(p, &graph.potential, tomorrow);
        Some(Self { graph, day:obs.day, steps, supplier, agents:obs.agents.len(), brands, near, full })
    }
    fn masks(p: &Problem, potential: &[Vec<(i32,i32)>], tomorrow: u32) -> (u64,Vec<u64>,Vec<u64>) {
        let mut brands = 0;
        for spot in &p.spots { if spot.stocks > 0 { brands |= 1u64 << spot.brand; } }
        let mut near = vec![0;potential.len()]; let mut full = vec![0;potential.len()];
        if tomorrow > 0 {
            for (node,row) in potential.iter().enumerate() {
                for (i,spot) in p.spots.iter().enumerate() {
                    if spot.stocks == 0 { continue; }
                    let distance = row[i].0;
                    if distance >= 0 && distance as u32 <= tomorrow { full[node] |= 1u64 << spot.brand; }
                    if distance >= 0 && distance as u32 <= tomorrow/2 { near[node] |= 1u64 << spot.brand; }
                }
            }
        }
        (brands,near,full)
    }
    pub(super) fn matches(&self, graph: &Graph, obs: &Observation, steps: i32, supplier: usize) -> bool {
        std::ptr::eq(self.graph.as_ref(),graph) && self.day == obs.day && self.steps == steps
            && self.supplier == supplier && self.agents == obs.agents.len()
    }
    fn unions(&self, nodes: &[usize;MAX_AGENTS], skip: Option<usize>) -> (u64,u64) {
        let mut near = 0; let mut full = 0;
        for (car,&node) in nodes.iter().enumerate().take(self.agents) {
            if car != self.supplier && Some(car) != skip { near |= self.near[node]; full |= self.full[node]; }
        }
        (near,full)
    }
    fn from_masks(&self, near: u64, full: u64) -> ReachStats {
        let far = (self.brands & !full).count_ones();
        let thin = (self.brands & full & !near).count_ones();
        ReachStats { far, thin, penalty: -f64::from((4*far+thin).min(12)) }
    }
    pub(super) fn stats(&self, nodes: &[usize;MAX_AGENTS]) -> ReachStats {
        let (near,full) = self.unions(nodes,None); self.from_masks(near,full)
    }
    pub(super) fn beam(&self, nodes: &[usize;MAX_AGENTS], car: usize) -> BeamReach<'_> {
        let (near,full) = self.unions(nodes,Some(car));
        BeamReach { table:self, near, full, original:self.stats(nodes).penalty }
    }
}
impl Solver {
    pub(super) fn terminal_for(&self, graph: &Graph, obs: &Observation, steps: i32) -> Option<&TerminalReach> {
        self.terminal_reach.as_ref().filter(|table| table.matches(graph,obs,steps,self.supplier))
    }
    pub(super) fn prepare_terminal(&mut self, graph: Rc<Graph>, obs: &Observation, steps: i32, primary: bool) {
        self.terminal_reach = None;
        if primary && self.terminal_reach_enabled && !self.selecting && self.coverage_weight == 0.0
            && self.future_weight == 0.0 && self.archive_weight == 0.0 {
            self.terminal_reach = TerminalReach::build(&self.p,obs,steps,graph);
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use tester::model::{Map, Spot, MatchFormat};
    fn setup() -> (Solver,Rc<Graph>,Observation) {
        let p=Problem { starts_at:0,day_seconds:vec![60;3],day_steps:vec![10,6,12],
            map:Map{height:3,width:9,cells:vec![vec![0;9];3]},
            spots:vec![Spot{brand:0,pos:0,stocks:2},Spot{brand:1,pos:8,stocks:2},
                Spot{brand:2,pos:18,stocks:2},Spot{brand:0,pos:26,stocks:1},Spot{brand:3,pos:13,stocks:0}],
            agents:vec![0,1,10,26],fuel_limits:60,players:4,busy_threshold:2,jammed_threshold:5 };
        let game=Game::new(p.clone(),&[1,0,0,0]);let obs=game.observation(0);
        let graph=Rc::new(Graph::new(&p,&obs,&[]));
        let mut solver=Solver::new(p,0,0.0,2.0,0.0);solver.game=Some(game);
        solver.future_weight=0.0;solver.reserve_weight=15.0;solver.terminal_reach_enabled=true;
        (solver,graph,obs)
    }
    /// Independent slow reference deliberately loops over brands, collectors and
    /// all stores, rather than reusing precomputed masks or their union helpers.
    fn reference(s:&Solver,g:&Graph,obs:&Observation,nodes:&[usize;MAX_AGENTS])->ReachStats {
        let tomorrow=s.p.day_steps[obs.day+1] as i32;
        let mut far=0;let mut thin=0;
        for brand in 0..64 {
            if !s.p.spots.iter().any(|shop|shop.brand==brand&&shop.stocks>0){continue;}
            let mut distance=i32::MAX;
            for car in 0..obs.agents.len(){if car==s.supplier{continue;}
                for (i,shop) in s.p.spots.iter().enumerate(){
                    if shop.brand==brand&&shop.stocks>0{distance=distance.min(g.potential[nodes[car]][i].0);}
                }
            }
            if distance>tomorrow{far+=1;}else if distance>tomorrow/2{thin+=1;}
        }
        ReachStats{far,thin,penalty:-f64::from((4*far+thin).min(12))}
    }
    #[test]
    fn inclusive_thresholds_duplicates_zero_stock_and_one_step() {
        let (mut s,_,_)=setup();
        s.p.spots=vec![Spot{brand:0,pos:0,stocks:1},Spot{brand:1,pos:1,stocks:1},
            Spot{brand:2,pos:2,stocks:1},Spot{brand:3,pos:3,stocks:1},Spot{brand:4,pos:4,stocks:1},
            Spot{brand:5,pos:5,stocks:0},Spot{brand:0,pos:6,stocks:1}];
        let potential=vec![vec![(2,99),(3,99),(4,99),(6,99),(7,99),(0,0),(100,0)]];
        let (brands,near,full)=TerminalReach::masks(&s.p,&potential,6);
        assert_eq!(brands,0b11111);assert_eq!(near[0],0b00011);assert_eq!(full[0],0b01111);
        let single=vec![vec![(0,99),(1,99),(2,99),(3,99),(4,99),(0,0),(100,0)]];
        let (_,near,full)=TerminalReach::masks(&s.p,&single,1);
        assert_eq!(near[0],1);assert_eq!(full[0],3);
        let (_,near,full)=TerminalReach::masks(&s.p,&single,0);
        assert_eq!(near[0],0);assert_eq!(full[0],0);
    }
    #[test]
    fn masks_match_independent_team_scan_and_ignore_supplier_and_seen_brands() {
        let (mut s,g,obs)=setup();s.prepare_terminal(g.clone(),&obs,10,true);
        let table=s.terminal_reach.as_ref().unwrap();
        let mut nodes=[0;MAX_AGENTS];
        for a in 0..g.positions.len(){for b in 0..g.positions.len(){for c in 0..g.positions.len(){
            nodes[1]=a;nodes[2]=b;nodes[3]=c;
            let stats=table.stats(&nodes);assert_eq!(stats,reference(&s,&g,&obs,&nodes));
            assert!((-12.0..=0.0).contains(&stats.penalty));
            nodes[0]=(a+b+c)%g.positions.len();assert_eq!(table.stats(&nodes),stats);
            let beam=table.beam(&nodes,2);
            for replacement in [0,g.positions.len()/2,g.positions.len()-1]{
                let mut changed=nodes;changed[2]=replacement;
                assert_eq!(beam.at(replacement),table.stats(&changed).penalty);
            }
        }}}
        let before=table.stats(&nodes);s.seen_brands=u64::MAX;
        assert_eq!(s.terminal_reach.as_ref().unwrap().stats(&nodes),before);
        // Moving a collector onto another brand's shop cannot remove any bits
        // contributed by the remaining collectors (the new node may add more).
        let table=s.terminal_reach.as_ref().unwrap();
        let saturated=table.from_masks(0,0);assert!(saturated.penalty>=-12.0);
    }
    #[test]
    fn graph_identity_phase_restore_roles_and_fallback_clear_tables() {
        let (mut s,g,obs)=setup();s.prepare_terminal(g.clone(),&obs,10,true);
        assert!(s.terminal_for(&g,&obs,10).is_some());
        let different=Graph::new(&s.p,&obs,&[]);
        assert!(s.terminal_for(&different,&obs,10).is_none());
        let mut next=obs.clone();next.day+=1;assert!(s.terminal_for(&g,&next,10).is_none());
        assert!(s.terminal_for(&g,&obs,9).is_none());
        let snapshot=s.snapshot();s.restore(snapshot);assert!(s.terminal_reach.is_none());
        s.prepare_terminal(g.clone(),&obs,10,false);assert!(s.terminal_reach.is_none());
        s.terminal_reach_enabled=false;let rng=s.rng.0;
        s.prepare_terminal(g.clone(),&obs,10,true);assert!(s.terminal_reach.is_none());assert_eq!(s.rng.0,rng);
        s.terminal_reach_enabled=true;
        let mut last=obs.clone();last.day=s.p.day_steps.len()-1;
        assert!(TerminalReach::build(&s.p,&last,10,g.clone()).is_none());
        let mut zero=s.p.clone();zero.day_steps[1]=0;
        assert!(TerminalReach::build(&zero,&obs,10,g.clone()).is_none());
        let mut bad=obs.clone();bad.agents[0].kind=0;
        assert!(TerminalReach::build(&s.p,&bad,10,g.clone()).is_none());
        s.prepare_terminal(g.clone(),&obs,10,true);
        let _=s.plan(&obs,10,1,true);assert!(s.terminal_reach.is_none());
        s.prepare_terminal(g.clone(),&obs,10,true);
        let _=s.select(1);assert!(s.terminal_reach.is_none());
    }
    #[test]
    fn full_evaluation_memo_and_beam_local_values_are_consistent() {
        let (mut s,g,obs)=setup();s.prepare_terminal(g.clone(),&obs,10,true);
        let mut solution=s.initial(&g,&obs,10,None);
        let m=s.p.spots.len();
        for car in 1..obs.agents.len(){
            solution.routes[car].clear();
            let zero=s.evaluate(&g,&obs,10,&solution,false,None);
            let supply=g.supply_cache.borrow().as_ref().unwrap().1.clone();
            let mut available=0u32;let mut other_brands=0u64;
            for i in 0..m{
                let visits=(0..obs.agents.len()).filter(|&a|a!=car&&zero.masks[a]&(1<<i)!=0).count();
                if visits<s.p.spots[i].stocks as usize{available|=1<<i;}
                if visits>0&&s.p.spots[i].stocks>0{other_brands|=1u64<<s.p.spots[i].brand;}
            }
            let marginal=|mask:u32|{
                let brands=s.p.spots.iter().enumerate().filter(|(i,shop)|mask&(1<<i)!=0&&shop.stocks>0)
                    .fold(0u64,|bits,(_,shop)|bits|(1u64<<shop.brand)) & !other_brands;
                20.0*(mask&available).count_ones()as f64+1500.0*brands.count_ones()as f64
            };
            let table=s.terminal_for(&g,&obs,10).unwrap();let reach=table.beam(&zero.endnodes,car);
            let fixed=zero.value-zero.route_values[car]-marginal(zero.masks[car])-reach.original;
            let start=CarPrefix{mask:zero.masks[car],node:m+car,time:0,fuel:obs.agents[car].fuel as i32,penalty:0.0,complete:true};
            for mode in 0..PATH_MODES{for token in 0..m+2*STOPS{
                let encoded=token+mode*(m+2*STOPS);
                let next=s.extend_prefix(&g,&supply,&solution,10,start,encoded);
                let local=fixed+s.prefix_value(&g,&obs,&supply,10,&next).0+marginal(next.mask)+reach.at(next.node);
                let mut candidate=solution.clone();candidate.routes[car]=vec![encoded];
                let full=s.evaluate(&g,&obs,10,&candidate,true,None);
                assert!((full.value-local).abs()<1e-7,"car={car} token={encoded} local={local} full={}",full.value);
                assert_eq!(full.endnodes[car],next.node); // includes partial paths
                let memoed=s.evaluate(&g,&obs,10,&candidate,false,None);
                assert!((memoed.value-full.value).abs()<1e-7);
                let mut game=s.game.clone().unwrap();game.play_day(full.plans).unwrap();
            }}
            solution.routes[car]=vec![car%m];
        }
        let on=s.evaluate(&g,&obs,10,&solution,false,None);
        let penalty=s.terminal_for(&g,&obs,10).unwrap().stats(&on.endnodes).penalty;
        let table=s.terminal_reach.take();
        let off=s.evaluate(&g,&obs,10,&solution,false,None); // same route memo retained
        assert_eq!(on.route_values,off.route_values);assert_eq!(on.endnodes,off.endnodes);
        assert!((on.value-off.value-penalty).abs()<1e-7);
        s.terminal_reach=table;
        assert!((s.evaluate(&g,&obs,10,&solution,false,None).value-on.value).abs()<1e-7);
    }
    #[test]
    fn primary_finishes_in_d0_comparison_units_and_forecast_stays_disabled() {
        let (mut s,_,obs)=setup();
        let plan=s.plan_inner(&obs,10,150,true,false,true);
        assert!(s.terminal_reach.is_none());
        let g=Graph::new(&s.p,&obs,&s.cache_positions[s.p.spots.len()+obs.agents.len()..]);
        // Cache node indices require the exact final ordering, so reconstruct by
        // the recorded graph positions only if the deterministic order agrees.
        assert_eq!(g.positions,s.cache_positions);
        let selected=&s.cache.as_ref().unwrap()[0];
        let old=s.reserve_weight;s.reserve_weight=15.0;
        let canonical=s.evaluate(&g,&obs,10,selected,false,None).value;s.reserve_weight=old;
        assert!((canonical-s.canonical_value).abs()<1e-7);
        let mut game=s.game.clone().unwrap();game.play_day(plan).unwrap();
        let mut tomorrow=game.observation(0);tomorrow.day=1;
        let _=s.plan_inner(&tomorrow,6,150,true,false,false);
        assert!(s.terminal_reach.is_none());
    }
    #[test]
    fn generated_three_formats_keep_engine_plans_legal() {
        for format in [MatchFormat::First,MatchFormat::Semifinal,MatchFormat::Final]{
            let p=tester::generator::generate(0,Some(format)).unwrap().problem;
            let mut kinds=vec![0;p.agents.len()];kinds[0]=1;
            let game=Game::new(p.clone(),&kinds);let obs=game.observation(0);
            let graph=Rc::new(Graph::new(&p,&obs,&[]));
            let mut s=Solver::new(p.clone(),0,0.0,2.0,0.0);s.game=Some(game.clone());s.future_weight=0.0;
            s.prepare_terminal(graph.clone(),&obs,p.day_steps[0]as i32,true);
            let initial=s.initial(&graph,&obs,p.day_steps[0]as i32,None);
            let out=s.evaluate(&graph,&obs,p.day_steps[0]as i32,&initial,true,None);
            let expected=reference(&s,&graph,&obs,&out.endnodes);
            assert_eq!(s.terminal_reach.as_ref().unwrap().stats(&out.endnodes),expected);
            let mut actual=game;actual.play_day(out.plans).unwrap();
        }
    }
}
