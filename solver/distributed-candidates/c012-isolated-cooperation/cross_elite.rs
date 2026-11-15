//! C012 isolated request-scoped hints. No authoritative state or network I/O
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
            schema: 3, problem: solver.p.clone(), agents: obs.agents.clone(),
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
pub(super) const CHECK_INTERVAL: Duration = Duration::from_millis(250);
pub(super) const STAGNATION: Duration = Duration::from_secs(1);
pub(super) const COOLDOWN: Duration = Duration::from_secs(1);
pub(super) const IMPORT_RESERVE: Duration = Duration::from_millis(750);
pub(super) const RESTART_GUARD: Duration = Duration::from_millis(100);
pub(super) const REPAIR_LIMIT: Duration = Duration::from_millis(25);
const GROUPS: usize = 7;
const SLOTS: usize = 80 * GROUPS;
const BATCH_OFFERS: usize = 7;
const PROBE_OFFERS: usize = 3;
const RESERVES: [f64; GROUPS] = [4.0, 8.0, 12.0, 15.0, 20.0, 30.0, 45.0];

/// No receiver is required in small ranges. This rule does not consume RNG.
pub(super) fn receiver_worker(offset: usize, count: usize) -> Option<usize> {
    (offset..offset + count).find(|&worker| worker % 2 == 1 && (worker / 2) % GROUPS == 3)
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Offer {
    origin: usize, group: usize, worker: usize, sequence: u64,
    value: f64, solution: Solution,
}
impl Offer {
    fn slot(&self) -> usize { self.origin * GROUPS + self.group }
    pub(super) fn solution(&self) -> &Solution { &self.solution }
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct ElitePacket {
    session: u64, request: u64, digest: u64,
    context: Context, offers: Vec<Offer>,
}
/// Per-link cursor: forwarding a group never marks a different group as sent.
pub(crate) struct EliteCursor { seen: [u64; SLOTS], next: usize }
impl EliteCursor {
    pub fn new() -> Self { Self { seen: [0; SLOTS], next: 0 } }
    pub fn mark(&mut self, packet: &ElitePacket) {
        for offer in &packet.offers { self.seen[offer.slot()] = offer.sequence; }
    }
}
struct Stored { context: Arc<Context>, offer: Arc<Offer>, arrived: Instant }
struct State {
    context: Option<Arc<Context>>, offers: Vec<Stored>,
    seen: [u64; SLOTS], sequence: [u64; GROUPS],
}
impl Default for State {
    fn default() -> Self { Self { context: None, offers: Vec::new(), seen: [0; SLOTS], sequence: [0; GROUPS] } }
}
#[derive(Default)]
struct Counters {
    published: AtomicU64, received: AtomicU64, rejected: AtomicU64,
    mismatch: AtomicU64, imports: AtomicU64, improved: AtomicU64,
    sent: AtomicU64, oversized: AtomicU64, probes: AtomicU64,
    probe_ns: AtomicU64, repair_ns: AtomicU64, duplicates: AtomicU64,
    checks: AtomicU64, eligible: AtomicU64, max_stall_ms: AtomicU64,
    published_groups: [AtomicU64; GROUPS], received_groups: [AtomicU64; GROUPS], sent_groups: [AtomicU64; GROUPS],
    max_receiver_mailbox_age_ms: AtomicU64,
}
pub(crate) struct CrossBus {
    session: u64, request: u64, digest: u64,
    offset: usize, end: usize, deadline: Instant, receiver: Option<usize>,
    active: AtomicBool, state: Mutex<State>, counters: Counters,
}
impl CrossBus {
    pub fn new(session: u64, request: u64, digest: u64, offset: usize,
               search_threads: usize, deadline: Instant) -> Arc<Self> {
        Arc::new(Self { session, request, digest, offset, end: offset + search_threads,
            deadline, receiver: receiver_worker(offset, search_threads),
            active: AtomicBool::new(true), state: Mutex::new(State::default()), counters: Counters::default() })
    }
    pub fn is_receiver(&self, worker: usize) -> bool { self.receiver == Some(worker) }
    fn open(&self, reserve: Duration) -> bool {
        self.active.load(Ordering::Relaxed)
            && self.deadline.saturating_duration_since(Instant::now()) > reserve
    }
    pub fn close(&self) {
        if self.active.swap(false, Ordering::Relaxed) {
            let c = &self.counters;
            let published_groups: [u64; GROUPS] = std::array::from_fn(|i| c.published_groups[i].load(Ordering::Relaxed));
            let received_groups: [u64; GROUPS] = std::array::from_fn(|i| c.received_groups[i].load(Ordering::Relaxed));
            let sent_groups: [u64; GROUPS] = std::array::from_fn(|i| c.sent_groups[i].load(Ordering::Relaxed));
            eprintln!("cross_elite request={} origin={} receiver={:?} published={} received={} rejected={} context_mismatch={} imports={} improved={} sent={} oversized={} probes={} probe_ns={} repair_ns={} duplicates={} checks={} eligible={} max_stall_ms={} group_published={:?} group_received={:?} group_sent={:?} max_receiver_mailbox_age_ms={}",
                self.request,self.offset,self.receiver,c.published.load(Ordering::Relaxed),c.received.load(Ordering::Relaxed),
                c.rejected.load(Ordering::Relaxed),c.mismatch.load(Ordering::Relaxed),c.imports.load(Ordering::Relaxed),
                c.improved.load(Ordering::Relaxed),c.sent.load(Ordering::Relaxed),c.oversized.load(Ordering::Relaxed),
                c.probes.load(Ordering::Relaxed),c.probe_ns.load(Ordering::Relaxed),c.repair_ns.load(Ordering::Relaxed),
                c.duplicates.load(Ordering::Relaxed),c.checks.load(Ordering::Relaxed),c.eligible.load(Ordering::Relaxed),c.max_stall_ms.load(Ordering::Relaxed),published_groups,received_groups,sent_groups,c.max_receiver_mailbox_age_ms.load(Ordering::Relaxed));
        }
    }
    fn bind(self: &Arc<Self>, worker: usize, context: Context, deadline: Instant) -> Option<Binding> {
        if !self.open(ELITE_INTERVAL) || !(self.offset..self.end).contains(&worker) { return None; }
        let mut state = self.state.try_lock().ok()?;
        if let Some(expected) = &state.context {
            if expected.as_ref() != &context { self.counters.mismatch.fetch_add(1, Ordering::Relaxed); return None; }
        } else {
            state.offers.retain(|stored| {
                let valid = stored.context.as_ref() == &context;
                if !valid { self.counters.mismatch.fetch_add(1, Ordering::Relaxed); }
                valid
            });
            // A pre-bind mismatched hint must not suppress a later valid revision.
            state.seen = [0; SLOTS];
            let retained: Vec<_> = state.offers.iter().map(|v| (v.offer.slot(), v.offer.sequence)).collect();
            for (slot, revision) in retained { state.seen[slot] = revision; }
            state.context = Some(Arc::new(context));
        }
        let now = Instant::now();
        Some(Binding { bus: self.clone(), worker, deadline, seen: [0; SLOTS], cursor: 0,
            next_check: now + CHECK_INTERVAL, next_publish: now,
            last_progress: now, observed_best: f64::NEG_INFINITY, last_seed: None })
    }
    /// Invalid envelope/context or any invalid member rejects the WHOLE batch.
    /// Duplicate or old well-formed revisions are individually ignored.
    pub fn receive(&self, packet: ElitePacket, direct: Option<(usize, usize)>) {
        let valid = self.open(Duration::from_millis(250))
            && (packet.session, packet.request, packet.digest) == (self.session, self.request, self.digest)
            && !packet.offers.is_empty() && packet.offers.len() <= BATCH_OFFERS
            && serde_json::to_vec(&packet).is_ok_and(|v| v.len() + 16 <= ELITE_MAX_BYTES)
            && packet.offers.iter().all(|offer| {
                offer.origin < 80 && offer.origin != self.offset && offer.group < GROUPS
                    && offer.worker < 79 && offer.worker >= offer.origin
                    && (offer.worker / 2) % GROUPS == offer.group
                    && receiver_worker(offer.origin, 79 - offer.origin) != Some(offer.worker)
                    && offer.sequence > 0 && offer.value.is_finite()
                    && direct.is_none_or(|(offset, count)| offer.origin == offset && (offset..offset + count).contains(&offer.worker))
                    && packet.context.valid_solution(&offer.solution)
            });
        if !valid { self.counters.rejected.fetch_add(1, Ordering::Relaxed); return; }
        let mut slots = Vec::new();
        for offer in &packet.offers {
            if slots.contains(&offer.slot()) { self.counters.rejected.fetch_add(1, Ordering::Relaxed); return; }
            slots.push(offer.slot());
        }
        let Ok(mut state) = self.state.try_lock() else { return; };
        if state.context.as_ref().is_some_and(|expected| expected.as_ref() != &packet.context) {
            self.counters.mismatch.fetch_add(1, Ordering::Relaxed); return;
        }
        let mut origins: Vec<_> = state.offers.iter().map(|v| v.offer.origin).filter(|&v| v != self.offset).collect();
        origins.extend(packet.offers.iter().map(|v| v.origin)); origins.sort_unstable(); origins.dedup();
        if origins.len() > 2 { self.counters.rejected.fetch_add(1, Ordering::Relaxed); return; }
        let context = Arc::new(packet.context);
        for offer in packet.offers {
            let slot = offer.slot();
            if offer.sequence <= state.seen[slot] { self.counters.duplicates.fetch_add(1, Ordering::Relaxed); continue; }
            state.seen[slot] = offer.sequence;
            state.offers.retain(|v| v.offer.slot() != slot);
            self.counters.received_groups[offer.group].fetch_add(1, Ordering::Relaxed);
            state.offers.push(Stored { context: context.clone(), offer: Arc::new(offer), arrived: Instant::now() });
            self.counters.received.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub fn outgoing(&self, destination: Option<usize>, relay: bool,
                    cursor: &mut EliteCursor) -> Option<Arc<ElitePacket>> {
        if !self.open(ELITE_INTERVAL) { return None; }
        // Serialize outside the shared mutex: search publishers only use try_lock.
        let (context, mut offers) = {
            let state = self.state.try_lock().ok()?;
            let context = state.context.as_ref()?.clone();
            let offers: Vec<_> = state.offers.iter().filter(|v| Some(v.offer.origin) != destination
                && (relay || v.offer.origin == self.offset) && v.offer.sequence > cursor.seen[v.offer.slot()])
                .map(|v| v.offer.clone()).collect();
            (context, offers)
        };
        offers.sort_by_key(|v| (v.slot() + SLOTS - cursor.next) % SLOTS);
        let mut packet = ElitePacket { session: self.session, request: self.request, digest: self.digest,
            context: (*context).clone(), offers: Vec::new() };
        for offer in offers {
            if packet.offers.len() == BATCH_OFFERS { break; }
            packet.offers.push((*offer).clone());
            if serde_json::to_vec(&packet).map_or(true, |v| v.len() + 16 > ELITE_MAX_BYTES) {
                packet.offers.pop(); self.counters.oversized.fetch_add(1, Ordering::Relaxed);
                if !packet.offers.is_empty() { cursor.next = offer.slot(); break; }
                // This offer cannot fit even alone. Skip it without enlarging the cap.
            }
            cursor.next = (offer.slot() + 1) % SLOTS;
        }
        (!packet.offers.is_empty()).then(|| Arc::new(packet))
    }
    pub fn note_send(&self, oversized: bool, packet: &ElitePacket) {
        if oversized { self.counters.oversized.fetch_add(1, Ordering::Relaxed); }
        else {
            self.counters.sent.fetch_add(1, Ordering::Relaxed);
            for offer in &packet.offers { self.counters.sent_groups[offer.group].fetch_add(1, Ordering::Relaxed); }
        }
    }
}
pub(super) struct Binding {
    bus: Arc<CrossBus>, worker: usize, deadline: Instant,
    seen: [u64; SLOTS], cursor: usize, next_check: Instant, next_publish: Instant,
    last_progress: Instant, observed_best: f64, last_seed: Option<Instant>,
}
impl Binding {
    pub(super) fn is_receiver(&self) -> bool { self.bus.is_receiver(self.worker) }
    pub(super) fn active_for_seed(&self) -> bool { self.bus.open(IMPORT_RESERVE) && self.deadline.saturating_duration_since(Instant::now()) >= IMPORT_RESERVE }
    pub(super) fn from_solver(bus: &Arc<CrossBus>, worker: usize, solver: &Solver,
                             graph: &Graph, obs: &Observation, steps: i32, deadline: Instant) -> Option<Self> {
        if solver.selecting || solver.future_weight != 0.0 || solver.coverage_weight != 0.0
            || solver.reserve_weight != RESERVES[(worker / 2) % GROUPS] { return None; }
        bus.bind(worker, Context::new(solver, graph, obs, steps), deadline)
    }
    pub(super) fn publish(&mut self, value: f64, solution: &Solution) {
        let now = Instant::now();
        if self.bus.is_receiver(self.worker) || now < self.next_publish || !self.bus.open(ELITE_INTERVAL)
            || self.deadline.saturating_duration_since(now) < ELITE_INTERVAL || !value.is_finite() { return; }
        self.next_publish = now + ELITE_INTERVAL;
        let Ok(mut state) = self.bus.state.try_lock() else { return; };
        let Some(context) = state.context.clone() else { return; };
        let group = (self.worker / 2) % GROUPS;
        if !context.valid_solution(solution) || state.offers.iter().any(|v| v.offer.origin == self.bus.offset
            && v.offer.group == group && (v.offer.value >= value || v.offer.solution == *solution)) { return; }
        state.sequence[group] += 1;
        let offer = Offer { origin: self.bus.offset, group, worker: self.worker, sequence: state.sequence[group], value, solution: solution.clone() };
        state.offers.retain(|v| v.offer.slot() != offer.slot());
        state.offers.push(Stored { context, offer: Arc::new(offer), arrived: Instant::now() });
        self.bus.counters.published_groups[group].fetch_add(1, Ordering::Relaxed);
        self.bus.counters.published.fetch_add(1, Ordering::Relaxed);
    }
    fn ready(&mut self, now: Instant, best: f64, until_restart: Duration) -> bool {
        if !self.bus.is_receiver(self.worker) || now < self.next_check { return false; }
        self.next_check = now + CHECK_INTERVAL;
        self.bus.counters.checks.fetch_add(1, Ordering::Relaxed);
        if best > self.observed_best + 0.00001 { self.observed_best = best; self.last_progress = now; }
        let stalled = now.saturating_duration_since(self.last_progress);
        self.bus.counters.max_stall_ms.fetch_max(stalled.as_millis() as u64, Ordering::Relaxed);
        self.bus.open(IMPORT_RESERVE) && self.deadline.saturating_duration_since(now) >= IMPORT_RESERVE
            && stalled >= STAGNATION && until_restart >= RESTART_GUARD
            && self.last_seed.is_none_or(|previous| now.saturating_duration_since(previous) >= COOLDOWN)
    }
    pub(super) fn take(&mut self, best: f64, until_restart: Duration) -> Vec<Arc<Offer>> {
        if !self.ready(Instant::now(), best, until_restart) { return Vec::new(); }
        self.bus.counters.eligible.fetch_add(1, Ordering::Relaxed);
        let Ok(state) = self.bus.state.try_lock() else { return Vec::new(); };
        let mut offers: Vec<_> = state.offers.iter().filter(|v| v.offer.origin != self.bus.offset
            && v.offer.sequence > self.seen[v.offer.slot()]).map(|v| v.offer.clone()).collect();
        offers.sort_by_key(|v| (v.slot() + SLOTS - self.cursor) % SLOTS);
        offers.truncate(PROBE_OFFERS);
        for offer in &offers {
            self.seen[offer.slot()] = offer.sequence; self.cursor = (offer.slot() + 1) % SLOTS;
            if let Some(stored) = state.offers.iter().find(|v| v.offer.slot() == offer.slot()) {
                self.bus.counters.max_receiver_mailbox_age_ms.fetch_max(stored.arrived.elapsed().as_millis() as u64, Ordering::Relaxed);
            }
        }
        offers
    }
    pub(super) fn record_probe(&self, elapsed: Duration) {
        self.bus.counters.probes.fetch_add(1, Ordering::Relaxed);
        self.bus.counters.probe_ns.fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
    }
    pub(super) fn seeded(&mut self, elapsed: Duration, improved: bool) {
        self.last_seed = Some(Instant::now());
        self.bus.counters.imports.fetch_add(1, Ordering::Relaxed);
        self.bus.counters.repair_ns.fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
        if improved { self.bus.counters.improved.fetch_add(1, Ordering::Relaxed); }
    }
}
/// Receiver-only branch mutation. The incumbent and supplier schedule are untouched.
pub(super) fn destroy_quarter(s: &mut Solution, supplier: usize, rng: &mut Rng) -> Option<usize> {
    let cars: Vec<_> = (0..s.routes.len()).filter(|&a| a != supplier && !s.routes[a].is_empty()).collect();
    if cars.is_empty() { return None; }
    let car = cars[rng.n(cars.len())]; let len = s.routes[car].len();
    let remove = len.div_ceil(4); let start = rng.n(len - remove + 1);
    s.routes[car].drain(start..start + remove);
    Some(car)
}
#[cfg(test)]
#[path = "isolated_tests.rs"]
mod tests;
