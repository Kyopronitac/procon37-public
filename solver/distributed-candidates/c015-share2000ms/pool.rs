//! Distributed scheduling around the unchanged P0 search algorithm.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Task {
    pub problem: Problem,
    pub seed: u64,
    pub operation: Operation,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) enum Operation {
    Select,
    Plan {
        state: Snapshot,
        observation: Observation,
        steps: u32,
        accepted: bool,
    },
}
impl Task {
    pub fn selection(problem: Problem, seed: u64) -> Self {
        Self {
            problem,
            seed,
            operation: Operation::Select,
        }
    }
    pub fn planning(
        problem: Problem,
        seed: u64,
        state: Snapshot,
        observation: Observation,
        steps: u32,
        accepted: bool,
    ) -> Self {
        Self {
            problem,
            seed,
            operation: Operation::Plan {
                state,
                observation,
                steps,
                accepted,
            },
        }
    }
    pub fn is_select(&self) -> bool {
        matches!(self.operation, Operation::Select)
    }
    pub fn digest(&self) -> u64 {
        digest(&serde_json::to_vec(self).expect("serializable task"))
    }
}

/// A consistency identifier, not a cryptographic authentication primitive.
pub(crate) fn digest(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Candidate {
    pub worker: usize,
    pub value: f64,
    pub answer: serde_json::Value,
    pub state: Snapshot,
}
impl Candidate {
    pub fn better_than(&self, other: &Self) -> bool {
        self.value > other.value || (self.value == other.value && self.worker < other.worker)
    }
    pub fn state_digest(&self) -> u64 {
        digest(&serde_json::to_vec(&self.state).unwrap())
    }
    /// Check the answer and the full pre-execution engine state against the
    /// authoritative previous winner. A stale worker may never submit its history.
    pub fn valid_for(&self, task: &Task, expected: &Candidate) -> bool {
        if !self.value.is_finite() {
            return false;
        }
        let Some(game) = &self.state.game else {
            return false;
        };
        if game.problem != task.problem || game.agents.len() != task.problem.agents.len() {
            return false;
        }
        if self.state.cache_day >= task.problem.day_steps.len() {
            return false;
        }
        if self
            .state
            .cache_positions
            .iter()
            .any(|&pos| pos >= task.problem.map.len())
        {
            return false;
        }
        if let Some(cache) = &self.state.cache {
            for solution in cache {
                if solution.routes.len() != task.problem.agents.len()
                    || solution
                        .hubs
                        .iter()
                        .any(|&h| h >= self.state.cache_positions.len())
                    || solution.hub_modes.iter().any(|&mode| mode >= PATH_MODES)
                    || solution
                        .routes
                        .iter()
                        .flatten()
                        .any(|&token| token >= (task.problem.spots.len() + 2 * STOPS) * PATH_MODES)
                {
                    return false;
                }
            }
        }
        match &task.operation {
            Operation::Select => {
                let Ok(kinds) =
                    tester::engine::validate_kinds(&self.answer, task.problem.agents.len())
                else {
                    return false;
                };
                game.day == 0
                    && self.state.last.is_none()
                    && self.state.seen_brands == 0
                    && game.agents == Game::new(task.problem.clone(), &kinds).agents
                    && kinds.iter().position(|&kind| kind == 1) == Some(self.state.supplier)
            }
            Operation::Plan { observation, .. } => {
                let Ok(plan) = serde_json::from_value::<Plan>(self.answer.clone()) else {
                    return false;
                };
                if self.state.last.as_ref() != Some(&plan)
                    || game.agents != observation.agents
                    || game.day != observation.day
                    || self.state.seen_brands != expected.state.seen_brands
                    || serde_json::to_value(&self.state.game).ok()
                        != serde_json::to_value(&expected.state.game).ok()
                {
                    return false;
                }
                game.clone().play_day(plan).is_ok()
            }
        }
    }
}

pub(crate) fn fallback(task: &Task) -> Candidate {
    let mut solver = Solver::new(task.problem.clone(), task.seed, 0.0, 2.0, 0.0);
    let answer = match &task.operation {
        Operation::Select => serde_json::json!(solver.select(1)),
        Operation::Plan {
            state,
            observation,
            steps,
            accepted,
        } => {
            solver.restore(state.clone());
            serde_json::json!(solver.plan(observation, *steps, 1, *accepted))
        }
    };
    Candidate {
        worker: usize::MAX,
        value: -1e300,
        answer,
        state: solver.snapshot(),
    }
}

// One coalesced slot per search thread; publishing never waits for the control loop.
#[derive(Clone)]
pub(super) struct ProgressSink {
    id: u64, worker: usize, selection: bool, day: usize,
    next: Instant, last_value: f64,
    slot: Arc<Mutex<Option<(u64, Candidate)>>>,
}
impl Solver {
    pub(super) fn publish_progress(&mut self, graph: &Graph, obs: &Observation, steps: i32,
                        best: &[Solution; 2], supplier: usize, value: f64) {
        let Some(mut sink) = self.progress.take() else { return; };
        // Rollout probes may execute future days on this same Solver. Their state
        // must never escape as a candidate for today's authoritative Task.
        if obs.day == sink.day && Instant::now() >= sink.next && value > sink.last_value {
            sink.next = Instant::now() + Duration::from_secs(1);
            let old_supplier = self.supplier;
            self.supplier = supplier;
            let outcome = self.evaluate(graph, obs, steps, &best[0], true, None);
            self.supplier = old_supplier;
            let mut state = self.snapshot();
            state.supplier = supplier;
            state.cache = Some(best.clone());
            state.cache_positions = graph.positions.clone();
            state.cache_day = obs.day;
            state.cache_complete = true;
            let answer = if sink.selection {
                let mut kinds = vec![0; self.p.agents.len()]; kinds[supplier] = 1;
                state.game = Some(Game::new(self.p.clone(), &kinds));
                state.last = None; state.seen_brands = 0;
                serde_json::json!(kinds)
            } else {
                state.last = Some(outcome.plans.clone());
                serde_json::json!(outcome.plans)
            };
            // Validate before publication without advancing the primary solver.
            let valid = sink.selection || state.game.as_ref().is_some_and(|game|
                game.day == obs.day && game.agents == obs.agents &&
                game.clone().play_day(state.last.clone().unwrap()).is_ok());
            if valid {
                if let Ok(mut slot) = sink.slot.try_lock() {
                    *slot = Some((sink.id, Candidate { worker: sink.worker, value, answer, state }));
                    sink.last_value = value;
                }
            }
        }
        self.progress = Some(sink);
    }
}

struct Job {
    id: u64,
    task: Arc<Task>,
    deadline: Instant,
    exchange: Arc<Mutex<Elite>>,
    cross: Option<Arc<CrossBus>>,
    progress: Arc<Mutex<Option<(u64, Candidate)>>>,
}
struct Completion {
    id: u64,
    worker_index: usize,
    candidate: Option<Candidate>,
}
pub(crate) struct SearchPool {
    workers: Vec<mpsc::Sender<Job>>,
    results: mpsc::Receiver<Completion>,
    busy: Vec<Option<u64>>,
    handles: Vec<thread::JoinHandle<()>>,
    offset: usize,
    progress: Vec<Arc<Mutex<Option<(u64, Candidate)>>>>,
}
impl SearchPool {
    pub fn new(search_threads: usize, offset: usize) -> Self {
        assert!((1..=79).contains(&search_threads));
        let (result_tx, results) = mpsc::channel();
        let mut workers = Vec::new();
        let mut handles = Vec::new();
        let progress = (0..search_threads).map(|_| Arc::new(Mutex::new(None))).collect();
        for worker_index in 0..search_threads {
            let worker = offset + worker_index;
            let (tx, rx) = mpsc::channel::<Job>();
            workers.push(tx);
            let result_tx = result_tx.clone();
            handles.push(
                thread::Builder::new()
                    .name(format!("dist-search-{worker}"))
                    .spawn(move || {
                        let mut solver: Option<Solver> = None;
                        while let Ok(job) = rx.recv() {
                            let candidate =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    let task = &job.task;
                                    if task.is_select()
                                        || solver.as_ref().is_none_or(|s| s.p != task.problem)
                                    {
                                        let mut fresh = Solver::new(
                                            task.problem.clone(),
                                            task.seed.wrapping_add((worker as u64 + 1) * 104729),
                                            0.0,
                                            [0.8, 1.2, 2.0, 3.0, 4.0, 1.6, 2.4][worker % 7],
                                            0.0,
                                        );
                                        fresh.temperature_scale =
                                            [0.65, 0.85, 1.0, 1.2, 1.5, 1.8, 2.2][worker % 7];
                                        solver = Some(fresh);
                                    }
                                    let solver = solver.as_mut().unwrap();
                                    solver.choice_value = f64::NEG_INFINITY;
                                    solver.progress = Some(ProgressSink {
                                        id: job.id, worker, selection: task.is_select(),
                                        day: match &task.operation { Operation::Select => 0, Operation::Plan {observation, ..} => observation.day },
                                        next: Instant::now(), last_value: f64::NEG_INFINITY,
                                        slot: job.progress.clone(),
                                    });
                                    let limit = job
                                        .deadline
                                        .saturating_duration_since(Instant::now())
                                        .as_millis()
                                        as u64;
                                    if limit == 0 {
                                        return None;
                                    }
                                    let answer = match &task.operation {
                                        Operation::Select => {
                                            serde_json::json!(solver.select_role(worker, limit))
                                        }
                                        Operation::Plan {
                                            state,
                                            observation,
                                            steps,
                                            accepted,
                                        } => {
                                            solver.restore(state.clone());
                                            let receiver = job.cross.as_ref().is_some_and(|bus| bus.is_receiver(worker));
                                            solver.exchange = if receiver { None } else { Some(job.exchange) };
                                            solver.cross = job.cross.map(|bus| (bus, worker));
                                            solver.reserve_weight =
                                                [4.0, 8.0, 12.0, 15.0, 20.0, 30.0, 45.0]
                                                    [(worker / 2) % 7];
                                            let plan = solver.plan_with_rollout(
                                                observation,
                                                *steps,
                                                limit,
                                                *accepted,
                                            );
                                            solver.exchange = None;
                                            solver.cross = None;
                                            serde_json::json!(plan)
                                        }
                                    };
                                    solver.progress = None;
                                    Some(Candidate {
                                        worker,
                                        value: if solver.choice_value.is_finite() {
                                            solver.choice_value
                                        } else {
                                            -1e290
                                        },
                                        answer,
                                        state: solver.snapshot(),
                                    })
                                }))
                                .unwrap_or_else(|_| {
                                    solver = None;
                                    None
                                });
                            if result_tx
                                .send(Completion {
                                    id: job.id,
                                    worker_index,
                                    candidate,
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                    })
                    .expect("search thread creation"),
            );
        }
        Self {
            workers,
            results,
            busy: vec![None; search_threads],
            handles,
            offset,
            progress,
        }
    }
    pub fn begin(&mut self, id: u64, task: &Task, deadline: Instant) -> usize {
        self.begin_cross(id, task, deadline, None)
    }
    pub fn begin_cross(&mut self, id: u64, task: &Task, deadline: Instant, cross: Option<Arc<CrossBus>>) -> usize {
        // Completion of an expired request releases its worker, but never becomes
        // a candidate for another request. At most one job is queued per worker.
        let _ = self.poll(id);
        let task = Arc::new(task.clone());
        let exchanges: Vec<_> = (0..7)
            .map(|_| {
                Arc::new(Mutex::new(Elite {
                    value: f64::NEG_INFINITY,
                    solution: None,
                }))
            })
            .collect();
        let mut count = 0;
        for (index, tx) in self.workers.iter().enumerate() {
            if self.busy[index].is_some() {
                continue;
            }
            let exchange = exchanges[((self.offset + index) / 2) % 7].clone();
            if tx
                .send(Job {
                    id,
                    task: task.clone(),
                    deadline,
                    exchange,
                    cross: if !task.is_select() { cross.clone() } else { None },
                    progress: self.progress[index].clone(),
                })
                .is_ok()
            {
                self.busy[index] = Some(id);
                count += 1;
            }
        }
        count
    }
    pub fn poll(&mut self, id: u64) -> Vec<Candidate> {
        let mut out = Vec::new();
        while let Ok(result) = self.results.try_recv() {
            if self.busy[result.worker_index] == Some(result.id) {
                self.busy[result.worker_index] = None;
            }
            if result.id == id {
                if let Some(candidate) = result.candidate {
                    out.push(candidate);
                }
            }
        }
        out
    }
    pub fn poll_progress(&mut self, id: u64) -> Vec<Candidate> {
        let mut out = Vec::new();
        for slot in &self.progress {
            if let Ok(mut slot) = slot.try_lock() {
                if let Some((got, candidate)) = slot.take() {
                    if got == id { out.push(candidate); }
                }
            }
        }
        out
    }
    pub fn pending(&self, id: u64) -> usize {
        self.busy.iter().filter(|busy| **busy == Some(id)).count()
    }
}
impl Drop for SearchPool {
    fn drop(&mut self) {
        self.workers.clear();
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod synchronization_tests {
    use super::*;
    fn problem() -> Problem {
        tester::generator::generate(2, None).unwrap().problem
    }
    #[test]
    fn selected_foreign_state_preserves_history_rejection_and_json_roundtrip() {
        let p = problem();
        let selection = Task::selection(p.clone(), 0);
        let chosen = fallback(&selection);
        let mut real = Game::new(
            p.clone(),
            &serde_json::from_value::<Vec<u8>>(chosen.answer).unwrap(),
        );
        let task = Task::planning(
            p.clone(),
            0,
            chosen.state,
            real.observation(0),
            p.day_steps[0],
            true,
        );
        let mut candidate = fallback(&task);
        // Force an actual non-waiting adopted plan, as if another machine won.
        let mut origin = Solver::new(p.clone(), 79, 0.0, 2.0, 0.0);
        if let Operation::Plan {
            state,
            observation,
            steps,
            ..
        } = &task.operation
        {
            origin.restore(state.clone());
            let plan = origin.plan(observation, *steps, 350, true);
            candidate.answer = serde_json::json!(plan);
            candidate.state = origin.snapshot();
        }
        assert!(candidate.valid_for(&task, &fallback(&task)));
        let adopted: Candidate =
            serde_json::from_slice(&serde_json::to_vec(&candidate).unwrap()).unwrap();
        assert_eq!(adopted.state_digest(), candidate.state_digest());
        let adopted_plan: Plan = serde_json::from_value(adopted.answer.clone()).unwrap();
        assert!(adopted_plan.iter().flatten().any(|&action| action >= 0));
        real.play_day(adopted_plan).unwrap();
        let next = Task::planning(
            p.clone(),
            0,
            adopted.state.clone(),
            real.observation(0),
            p.day_steps[1],
            true,
        );
        let synchronized = fallback(&next);
        let internal = synchronized.state.game.as_ref().unwrap();
        assert_eq!(internal.agents, real.agents);
        assert_eq!(internal.score, real.score);
        assert_eq!(internal.last_stays(), real.last_stays());
        assert!(synchronized.state.seen_brands != 0);
        // The server rejected the same proposed plan: simulate the actual wait,
        // and ensure its history, fuel, and collections are authoritative.
        let mut rejected = Game::new(
            p.clone(),
            &real.agents.iter().map(|a| a.kind).collect::<Vec<_>>(),
        );
        rejected.play_day(rejected.waiting_plan()).unwrap();
        let rejected_task = Task::planning(
            p.clone(),
            0,
            adopted.state,
            rejected.observation(0),
            p.day_steps[1],
            false,
        );
        let synchronized = fallback(&rejected_task);
        assert_eq!(
            synchronized.state.game.as_ref().unwrap().score,
            rejected.score
        );
        assert_eq!(
            synchronized.state.game.as_ref().unwrap().last_stays(),
            rejected.last_stays()
        );
        assert_eq!(synchronized.state.seen_brands, 0);
        assert!(!candidate.valid_for(&next, &fallback(&next)));
    }
    #[test]
    fn begin_cross_dispatches_all_groups_but_never_selection_or_off() {
        // Recording endpoints exercise the real begin_cross dispatch path without
        // running timed search. Direct Binding tests cannot detect a missing Job bus.
        fn dispatched(offset: usize, count: usize, task: &Task, cross: Option<Arc<CrossBus>>) -> Vec<Job> {
            let mut workers = Vec::new();
            let mut recorders = Vec::new();
            for _ in 0..count {
                let (tx, rx) = mpsc::channel();
                workers.push(tx); recorders.push(rx);
            }
            let (_result_tx, results) = mpsc::channel();
            let mut pool = SearchPool { workers, results, busy: vec![None; count], handles: Vec::new(), offset, progress: (0..count).map(|_| Arc::new(Mutex::new(None))).collect() };
            assert_eq!(pool.begin_cross(17, task, Instant::now() + Duration::from_secs(2), cross), count);
            assert_eq!(pool.pending(17), count);
            recorders.into_iter().map(|rx| rx.try_recv().unwrap()).collect()
        }
        let p = problem();
        let selection = Task::selection(p.clone(), 0);
        let safe = fallback(&selection);
        let observation = safe.state.game.as_ref().unwrap().observation(0);
        let plan = Task::planning(p.clone(), 0, safe.state, observation, p.day_steps[0], true);
        let mut all_groups = [false; 7]; let mut total = 0;
        for (offset, count, receiver) in [(0,31,Some(7)),(31,15,Some(35)),(46,31,Some(49))] {
            let cross = CrossBus::new(1,17,plan.digest(),offset,count,Instant::now()+Duration::from_secs(3));
            let jobs = dispatched(offset,count,&plan,Some(cross.clone()));
            for (index,job) in jobs.into_iter().enumerate() {
                let worker = offset + index;
                let attached = job.cross.expect("every Plan worker needs the bus, including non-reserve15 donors");
                assert!(Arc::ptr_eq(&attached,&cross));
                assert_eq!(attached.is_receiver(worker),receiver==Some(worker));
                all_groups[(worker/2)%7] = true;total += 1;
            }
            assert!(dispatched(offset,count,&selection,Some(cross)).iter().all(|job|job.cross.is_none()));
            assert!(dispatched(offset,count,&plan,None).iter().all(|job|job.cross.is_none()));
        }
        assert_eq!(total,77);assert!(all_groups.into_iter().all(|present|present));
    }
    #[test]
    fn in_flight_candidates_are_current_task_snapshots_and_rollouts_never_escape() {
        let p = problem();
        let selection = Task::selection(p.clone(), 0);
        let mut pool = SearchPool::new(1, 0);
        let mut state = fallback(&selection);
        for (id, task) in [(31, selection.clone()), (32, {
            let selected = fallback(&selection);
            Task::planning(p.clone(), 0, selected.state.clone(),
                selected.state.game.as_ref().unwrap().observation(0), p.day_steps[0], true)
        })] {
            let safe = fallback(&task);
            let began = Instant::now();
            pool.begin(id, &task, began + Duration::from_millis(4500));
            let mut published = 0;
            while pool.pending(id) > 0 {
                for candidate in pool.poll_progress(id) {
                    assert!(candidate.valid_for(&task, &safe));
                    assert_eq!(candidate.state.game.as_ref().unwrap().day, 0);
                    if task.is_select() { assert!(candidate.state.last.is_none()); }
                    else { assert_eq!(serde_json::json!(candidate.state.last), candidate.answer); }
                    published += 1;
                    state = candidate;
                }
                let _ = pool.poll(id);
                assert!(began.elapsed() < Duration::from_secs(8));
                thread::sleep(Duration::from_millis(5));
            }
            assert!(published > 0, "no useful snapshot before final result");
        }
        let first_slot = pool.progress[0].clone();
        *first_slot.lock().unwrap() = Some((32, state));
        assert!(pool.poll_progress(33).is_empty(), "stale progress crossed request boundary");
    }

    #[test]
    fn expired_results_cannot_be_used_for_later_request() {
        let mut pool = SearchPool::new(2, 31);
        let task = Task::selection(problem(), 0);
        pool.begin(101, &task, Instant::now() + Duration::from_millis(5));
        thread::sleep(Duration::from_millis(40));
        assert!(pool.poll(102).is_empty());
        assert_eq!(
            pool.begin(102, &task, Instant::now() + Duration::from_millis(5)),
            2
        );
        thread::sleep(Duration::from_millis(40));
        assert_eq!(pool.poll(102).len(), 2);
    }
}
