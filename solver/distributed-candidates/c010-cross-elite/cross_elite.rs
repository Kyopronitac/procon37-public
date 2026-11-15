//! Optional, request-scoped hints. No graph, authoritative state, or network I/O
//! enters a search worker. Every imported solution is evaluated by its receiver.
use super::*;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub(crate) const ELITE_MAX_BYTES: usize = 32 * 1024;
pub(crate) const ELITE_INTERVAL: Duration = Duration::from_millis(500);

/// Full equality, never a hash-only graph comparison. Ordered positions retain
/// duplicates and anchor order. All primary evaluate() inputs are represented.
/// Graph paths, cell_node, spot_at, mobility are deterministic from these values.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Context {
    schema: u32,
    problem: Problem,
    agents: Vec<tester::model::Agent>,
    day: usize,
    steps: i32,
    supplier: usize,
    seen_brands: u64,
    positions: Vec<usize>,
    targets: usize,
    durations: Vec<i32>,
    costs: Vec<i32>,
}
impl Context {
    fn new(solver: &Solver, graph: &Graph, obs: &Observation, steps: i32) -> Self {
        Self {
            schema: 1, problem: solver.p.clone(), agents: obs.agents.clone(),
            day: obs.day, steps, supplier: solver.supplier, seen_brands: solver.seen_brands,
            positions: graph.positions.clone(), targets: graph.targets,
            durations: graph.durations.clone(), costs: graph.costs.clone(),
        }
    }
    fn valid_solution(&self, solution: &Solution) -> bool {
        self.steps > 0 && self.steps <= MAX_STEPS as i32
            && self.agents.len() <= MAX_AGENTS && self.supplier < self.agents.len()
            && solution.routes.len() == self.agents.len()
            && solution.hubs.iter().all(|&v| v < self.targets)
            && solution.hub_modes.iter().all(|&v| v < PATH_MODES)
            && solution.departures.iter().all(|&v| v >= 0 && v <= self.steps)
            && solution.departures.windows(2).all(|v| v[0] <= v[1])
            && solution.routes.iter().all(|route| route.len() <= 64
                && route.iter().all(|&v| v < (self.problem.spots.len() + 2 * STOPS) * PATH_MODES))
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct ElitePacket {
    session: u64,
    request: u64,
    digest: u64,
    origin: usize,
    worker: usize,
    sequence: u64,
    value: f64,
    context: Context,
    solution: Solution,
}
impl ElitePacket {
    pub fn identity(&self) -> (usize, u64) { (self.origin, self.sequence) }
}
struct State {
    context: Option<Context>,
    offers: Vec<Arc<ElitePacket>>, // at most one latest offer per origin, <=3 origins
    seen: [u64; 80],
    sequence: u64,
}
impl Default for State {
    fn default() -> Self { Self { context:None, offers:Vec::new(), seen:[0;80], sequence:0 } }
}
#[derive(Default)]
struct Counters {
    published: AtomicU64, received: AtomicU64, rejected: AtomicU64,
    mismatch: AtomicU64, imports: AtomicU64, improved: AtomicU64,
    sent: AtomicU64, oversized: AtomicU64,
}
pub(crate) struct CrossBus {
    session: u64, request: u64, digest: u64,
    offset: usize, end: usize, deadline: Instant,
    active: AtomicBool, state: Mutex<State>, counters: Counters,
}
impl CrossBus {
    pub fn new(session: u64, request: u64, digest: u64, offset: usize,
               search_threads: usize, deadline: Instant) -> Arc<Self> {
        Arc::new(Self { session, request, digest, offset, end: offset + search_threads,
            deadline, active: AtomicBool::new(true), state: Mutex::new(State::default()),
            counters: Counters::default() })
    }
    fn open(&self, reserve: Duration) -> bool {
        self.active.load(Ordering::Relaxed)
            && self.deadline.saturating_duration_since(Instant::now()) > reserve
    }
    pub fn close(&self) {
        if self.active.swap(false, Ordering::Relaxed) {
            let c = &self.counters;
            eprintln!("cross_elite request={} origin={} published={} received={} rejected={} context_mismatch={} imports={} improved={} sent={} oversized={}",
                self.request, self.offset, c.published.load(Ordering::Relaxed), c.received.load(Ordering::Relaxed),
                c.rejected.load(Ordering::Relaxed), c.mismatch.load(Ordering::Relaxed), c.imports.load(Ordering::Relaxed),
                c.improved.load(Ordering::Relaxed), c.sent.load(Ordering::Relaxed), c.oversized.load(Ordering::Relaxed));
        }
    }
    fn bind(self: &Arc<Self>, worker: usize, context: Context, deadline: Instant) -> Option<Binding> {
        if !self.open(Duration::from_millis(250)) || (worker / 2) % 7 != 3
            || !(self.offset..self.end).contains(&worker) { return None; }
        let mut state = self.state.try_lock().ok()?;
        if let Some(expected) = &state.context {
            if expected != &context { self.counters.mismatch.fetch_add(1, Ordering::Relaxed); return None; }
        } else {
            state.offers.retain(|offer| {
                let valid = offer.context == context && context.valid_solution(&offer.solution);
                if !valid { self.counters.mismatch.fetch_add(1, Ordering::Relaxed); }
                valid
            });
            state.context = Some(context);
        }
        Some(Binding { bus: self.clone(), worker, deadline, seen: [0;80] })
    }
    /// The coordinator checks a directly connected peer's exact origin/range.
    /// A leaf accepts coordinator relays, preserving the original origin/sequence.
    pub fn receive(&self, packet: ElitePacket, direct: Option<(usize, usize)>) {
        let valid = self.open(Duration::from_millis(250))
            && (packet.session, packet.request, packet.digest) == (self.session, self.request, self.digest)
            && packet.origin < 80 && packet.origin != self.offset && packet.worker < 79
            && packet.worker >= packet.origin && (packet.worker / 2) % 7 == 3
            && packet.sequence > 0 && packet.value.is_finite()
            && direct.is_none_or(|(offset, count)| packet.origin == offset
                && (offset..offset + count).contains(&packet.worker))
            && packet.context.valid_solution(&packet.solution)
            && serde_json::to_vec(&packet).is_ok_and(|v| v.len() + 16 <= ELITE_MAX_BYTES);
        if !valid { self.counters.rejected.fetch_add(1, Ordering::Relaxed); return; }
        let Ok(mut state) = self.state.try_lock() else { return; };
        if state.context.as_ref().is_some_and(|expected| expected != &packet.context) {
            self.counters.mismatch.fetch_add(1, Ordering::Relaxed); return;
        }
        if packet.sequence <= state.seen[packet.origin] { return; }
        if !state.offers.iter().any(|v| v.origin == packet.origin) && state.offers.iter().filter(|v| v.origin != self.offset).count() >= 2 {
            self.counters.rejected.fetch_add(1, Ordering::Relaxed); return;
        }
        state.seen[packet.origin] = packet.sequence;
        state.offers.retain(|v| v.origin != packet.origin);
        state.offers.push(Arc::new(packet));
        self.counters.received.fetch_add(1, Ordering::Relaxed);
    }
    pub fn outgoing(&self, destination: Option<usize>, relay: bool,
                    sent: &[u64;80]) -> Option<Arc<ElitePacket>> {
        if !self.open(ELITE_INTERVAL) { return None; }
        let state = self.state.try_lock().ok()?;
        state.context.as_ref()?;
        state.offers.iter().filter(|v| Some(v.origin) != destination
            && (relay || v.origin == self.offset) && v.sequence > sent[v.origin])
            .max_by(|a,b| a.value.total_cmp(&b.value)).cloned()
    }
    pub fn note_send(&self, oversized: bool) {
        if oversized { self.counters.oversized.fetch_add(1, Ordering::Relaxed); }
        else { self.counters.sent.fetch_add(1, Ordering::Relaxed); }
    }
}
pub(super) struct Binding {
    bus: Arc<CrossBus>, worker: usize, deadline: Instant, seen: [u64;80],
}
impl Binding {
    pub(super) fn from_solver(bus: &Arc<CrossBus>, worker: usize, solver: &Solver,
                             graph: &Graph, obs: &Observation, steps: i32, deadline: Instant) -> Option<Self> {
        if solver.selecting || solver.future_weight != 0.0 || solver.coverage_weight != 0.0
            || solver.reserve_weight != 15.0 { return None; }
        bus.bind(worker, Context::new(solver, graph, obs, steps), deadline)
    }
    pub(super) fn publish(&self, value: f64, solution: &Solution) {
        if !self.bus.open(ELITE_INTERVAL) || self.deadline.saturating_duration_since(Instant::now()) < ELITE_INTERVAL
            || !value.is_finite() { return; }
        let Ok(mut state) = self.bus.state.try_lock() else { return; };
        let Some(context) = &state.context else { return; };
        if !context.valid_solution(solution) || state.offers.iter().any(|v|
            (v.origin == self.bus.offset && v.value >= value) || v.solution == *solution) { return; }
        let context = context.clone();
        state.sequence += 1;
        let packet = ElitePacket { session:self.bus.session, request:self.bus.request, digest:self.bus.digest,
            origin:self.bus.offset, worker:self.worker, sequence:state.sequence, value, context, solution:solution.clone() };
        state.offers.retain(|v| v.origin != self.bus.offset);
        state.offers.push(Arc::new(packet));
        self.bus.counters.published.fetch_add(1, Ordering::Relaxed);
    }
    pub(super) fn take(&mut self) -> Option<Arc<ElitePacket>> {
        if !self.bus.open(Duration::from_millis(250))
            || self.deadline.saturating_duration_since(Instant::now()) < Duration::from_millis(250) { return None; }
        let state = self.bus.state.try_lock().ok()?;
        let offer = state.offers.iter().filter(|v| v.origin != self.bus.offset
            && v.sequence > self.seen[v.origin]).max_by(|a,b| a.value.total_cmp(&b.value)).cloned()?;
        self.seen[offer.origin] = offer.sequence;
        self.bus.counters.imports.fetch_add(1, Ordering::Relaxed);
        Some(offer)
    }
    pub(super) fn improved(&self) { self.bus.counters.improved.fetch_add(1, Ordering::Relaxed); }
}
impl ElitePacket {
    pub(super) fn solution(&self) -> &Solution { &self.solution }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tester::model::MatchFormat;
    fn setup(format: MatchFormat) -> (Solver, Graph, Observation, Solution) {
        let p = tester::generator::generate(0, Some(format)).unwrap().problem;
        let mut kinds = vec![0; p.agents.len()]; kinds[0] = 1;
        let game = Game::new(p.clone(), &kinds);
        let obs = game.observation(0);
        let mut solver = Solver::new(p.clone(), 0, 0.0, 2.0, 0.0);
        solver.future_weight = 0.0; solver.reserve_weight = 15.0;
        solver.game = Some(game);
        let graph = Graph::new(&p, &obs, &[]);
        let solution = solver.initial(&graph, &obs, p.day_steps[0] as i32, None);
        (solver, graph, obs, solution)
    }
    fn buses() -> (Arc<CrossBus>, Arc<CrossBus>) {
        let deadline = Instant::now() + Duration::from_secs(60);
        (CrossBus::new(10, 20, 30, 0, 31, deadline), CrossBus::new(10, 20, 30, 31, 15, deadline))
    }
    fn bind(bus: &Arc<CrossBus>, worker: usize, s: &Solver, g: &Graph, obs: &Observation) -> Binding {
        Binding::from_solver(bus, worker, s, g, obs, s.p.day_steps[0] as i32,
            Instant::now() + Duration::from_secs(50)).unwrap()
    }
    #[test]
    fn generated_all_formats_fit_full_context_and_legal_max_routes() {
        for format in [MatchFormat::First, MatchFormat::Semifinal, MatchFormat::Final] {
            let (s,g,obs,solution) = setup(format);
            let (a,b) = buses();
            let a_binding = bind(&a, 6, &s,&g,&obs);
            let mut b_binding = bind(&b, 34, &s,&g,&obs);
            a_binding.publish(1.0,&solution);
            let packet = a.outgoing(Some(31),true,&[0;80]).unwrap();
            let bytes = serde_json::to_vec(&crate::wire::ClientMessage::Elite((*packet).clone())).unwrap();
            eprintln!("cross_size format={} cells={} context_and_solution_bytes={}",format.name(),s.p.map.len(),bytes.len());
            assert!(bytes.len() <= ELITE_MAX_BYTES);
            b.receive((*packet).clone(),Some((0,31)));
            let imported = b_binding.take().unwrap();
            let value = s.evaluate(&g,&obs,s.p.day_steps[0] as i32,imported.solution(),true,None);
            let mut game = s.game.clone().unwrap(); game.play_day(value.plans).unwrap();
            assert_eq!(value.value.to_bits(), s.evaluate(&g,&obs,s.p.day_steps[0] as i32,&solution,false,None).value.to_bits());
            // The largest accepted route vectors also fit without changing context.
            let mut largest = (*packet).clone();
            largest.solution.routes = vec![vec![(s.p.spots.len()+2*STOPS)*PATH_MODES-1;64];obs.agents.len()];
            assert!(largest.context.valid_solution(&largest.solution));
            let size = serde_json::to_vec(&crate::wire::ServerMessage::Elite(largest)).unwrap().len();
            eprintln!("cross_size format={} max_route_bytes={size}",format.name());
            assert!(size <= ELITE_MAX_BYTES);
        }
    }
    #[test]
    fn context_order_traffic_role_history_and_envelope_mismatches_are_rejected() {
        let (s,g,obs,solution) = setup(MatchFormat::First);
        let (a,b) = buses();
        let a_binding = bind(&a,6,&s,&g,&obs);
        let mut b_binding = bind(&b,34,&s,&g,&obs);
        a_binding.publish(10.0,&solution);
        let original = a.outgoing(Some(31),true,&[0;80]).unwrap();
        for n in 0..12 {
            let mut packet = (*original).clone(); packet.sequence += n as u64;
            match n {
                0 => packet.context.positions.swap(0,1),
                1 => packet.context.durations[0] += 1,
                2 => packet.context.supplier += 1,
                3 => packet.context.seen_brands ^= 1,
                4 => packet.context.agents[1].fuel += 1,
                5 => packet.context.day += 1,
                6 => packet.request += 1,
                7 => packet.session += 1,
                8 => packet.digest += 1,
                9 => packet.worker = 0,
                10 => packet.solution.hubs[0] = g.targets,
                11 => packet.value = f64::NAN,
                _ => unreachable!(),
            }
            b.receive(packet,Some((0,31)));
            assert!(b_binding.take().is_none(), "mutation {n}");
        }
        b.receive((*original).clone(),Some((0,31)));
        assert!(b_binding.take().is_some());
        assert!(b_binding.take().is_none());
        // Closing a request invalidates even its still-running search workers.
        b.close();
        let mut fresh = (*original).clone(); fresh.sequence += 100;
        b.receive(fresh,Some((0,31))); assert!(b_binding.take().is_none());
        a.close(); a_binding.publish(1e20,&solution);
        assert!(a.outgoing(None,true,&[0;80]).is_none());
    }
    #[test]
    fn prebind_mailbox_relay_dedup_and_sender_value_are_only_hints() {
        let (s,g,obs,solution) = setup(MatchFormat::First);
        let (a,b) = buses();
        let a_binding = bind(&a,6,&s,&g,&obs);
        a_binding.publish(1e200,&solution); // receiver must not adopt this claimed score
        let packet = a.outgoing(Some(31),true,&[0;80]).unwrap();
        b.receive((*packet).clone(),Some((0,31)));
        assert!(b.outgoing(None,true,&[0;80]).is_none()); // not verified locally yet
        let mut binding = bind(&b,34,&s,&g,&obs);
        let offer = binding.take().unwrap();
        let actual = s.evaluate(&g,&obs,s.p.day_steps[0] as i32,offer.solution(),false,None).value;
        assert!(actual < offer.value);
        let relay = b.outgoing(Some(46),true,&[0;80]).unwrap();
        assert_eq!(relay.identity(), packet.identity());
        assert!(b.outgoing(Some(0),true,&[0;80]).is_none());
        assert!(b.outgoing(None,false,&[0;80]).is_none()); // leaves never re-export imports
        a.receive((*relay).clone(),None); // reflection ignored
        assert_eq!(a.counters.received.load(Ordering::Relaxed),0);
        b.receive((*packet).clone(),Some((0,31))); assert!(binding.take().is_none());
        let mut sent = [0;80]; sent[0] = packet.sequence;
        assert!(b.outgoing(None,true,&sent).is_none());
        // Different contexts arriving before graph construction cannot escape bind.
        let c = CrossBus::new(10,20,30,46,31,Instant::now()+Duration::from_secs(60));
        let mut wrong = (*packet).clone(); wrong.context.positions.swap(0,1);
        c.receive(wrong,None); let mut c_binding = bind(&c,48,&s,&g,&obs);
        assert!(c_binding.take().is_none());
    }
    #[test]
    fn unsupported_group_objective_and_short_deadline_never_bind() {
        let (mut s,g,obs,_) = setup(MatchFormat::First);
        let (a,_) = buses();
        let far = Instant::now()+Duration::from_secs(50);
        assert!(Binding::from_solver(&a,0,&s,&g,&obs,100,far).is_none());
        s.future_weight = 0.1;
        assert!(Binding::from_solver(&a,6,&s,&g,&obs,100,far).is_none());
        s.future_weight = 0.0; s.reserve_weight = 12.0;
        assert!(Binding::from_solver(&a,6,&s,&g,&obs,100,far).is_none());
        s.reserve_weight = 15.0; s.selecting = true;
        assert!(Binding::from_solver(&a,6,&s,&g,&obs,100,far).is_none());
        s.selecting = false;
        let short = CrossBus::new(10,20,30,0,31,Instant::now());
        assert!(Binding::from_solver(&short,6,&s,&g,&obs,100,far).is_none());
    }
}
