//! P3: bounded, persistent search islands with an authoritative game history.
//! The controller is the only owner of accepted history; graph caches never
//! cross threads. std::thread is the only source of solver worker threads.
use crate::planner::{Solver, WarmStart};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use tester::engine::{Game, validate_kinds};
use tester::model::{Event, Observation, Plan, Problem};

const MAX_WORKERS: usize = 14;

#[derive(Clone, Copy)]
pub struct Settings {
    pub seed: u64,
    pub coverage_weight: f64,
    pub beam_time_weight: f64,
    pub archive_weight: f64,
}

enum Work {
    Select { epoch: u64, deadline: Instant },
    Plan {
        epoch: u64,
        deadline: Instant,
        game: Arc<Game>,
        observation: Arc<Observation>,
        steps: u32,
        seen: u64,
        warm: Option<Arc<WarmStart>>,
    },
}

enum Answer {
    Select(Vec<u8>),
    Plan(Plan),
    Expired,
}

struct Reply {
    epoch: u64,
    worker: usize,
    answer: Answer,
    value: f64,
    warm: Option<WarmStart>,
}

pub struct ParallelSolver {
    problem: Problem,
    settings: Settings,
    workers: Vec<mpsc::SyncSender<Work>>,
    handles: Vec<thread::JoinHandle<()>>,
    replies: mpsc::Receiver<Reply>,
    reply_sender: mpsc::Sender<Reply>,
    started: bool,
    epoch: u64,
    game: Option<Game>,
    seen: u64,
    last: Option<Plan>,
    warm: Option<Arc<WarmStart>>,
}

impl ParallelSolver {
    pub fn new(problem: Problem, settings: Settings) -> Self {
        let (reply_sender, replies) = mpsc::channel();
        Self { problem, settings, workers: vec![], handles: vec![], replies, reply_sender,
            started: false, epoch: 0, game: None, seen: 0, last: None, warm: None }
    }

    fn ensure_workers(&mut self) {
        if self.started { return; }
        self.started = true;
        let requested = std::env::var("PROCON37_P3_WORKERS").ok()
            .and_then(|v| v.parse::<usize>().ok()).unwrap_or(MAX_WORKERS)
            .clamp(1, MAX_WORKERS);
        for index in 0..requested {
            let problem = self.problem.clone();
            let settings = self.settings;
            let output = self.reply_sender.clone();
            let (input, jobs) = mpsc::sync_channel(1);
            let worker = self.workers.len();
            let result = thread::Builder::new().name(format!("p3-search-{worker:02}"))
                .stack_size(4 * 1024 * 1024).spawn(move || {
                    let seed = settings.seed.wrapping_add((index as u64 + 1)
                        .wrapping_mul(0xd1b54a32d192ed03));
                    let mut solver = Solver::new(problem, seed, settings.coverage_weight,
                        settings.beam_time_weight, settings.archive_weight);
                    solver.island_profile(index);
                    while let Ok(work) = jobs.recv() {
                        let (epoch, deadline) = match &work {
                            Work::Select { epoch, deadline } | Work::Plan { epoch, deadline, .. } => (*epoch, *deadline),
                        };
                        let limit = deadline.saturating_duration_since(Instant::now()).as_millis() as u64;
                        if limit <= 110 {
                            let _ = output.send(Reply { epoch, worker, answer: Answer::Expired,
                                value: f64::NEG_INFINITY, warm: None });
                            continue;
                        }
                        let (answer, value) = match work {
                            Work::Select { .. } => {
                                let kinds = solver.select(limit);
                                (Answer::Select(kinds), solver.selection_value())
                            }
                            Work::Plan { game, observation, steps, seen, warm, .. } => {
                                solver.synchronize((*game).clone(), seen, warm.as_deref());
                                let plan = solver.plan(&observation, steps, limit, true);
                                (Answer::Plan(plan), solver.decision_value())
                            }
                        };
                        if output.send(Reply { epoch, worker, answer, value,
                            warm: solver.warm_start() }).is_err() { break; }
                    }
                });
            match result {
                Ok(handle) => { self.workers.push(input); self.handles.push(handle); },
                Err(error) => eprintln!("p3 worker creation failed: {error}"),
            }
        }
        eprintln!("p3 workers={} total_thread_limit={}", self.workers.len(), MAX_WORKERS + 1);
    }

    fn deadline(limit: u64) -> Instant {
        // Leave a controller margin for validation, serialization and scheduling.
        Instant::now() + Duration::from_millis(limit.saturating_sub((limit / 30).clamp(10, 600)))
    }

    pub fn select(&mut self, limit: u64) -> Vec<u8> {
        let deadline = Self::deadline(limit);
        let mut kinds = vec![0; self.problem.agents.len()];
        kinds[0] = 1;
        if limit > 110 {
            self.ensure_workers();
            self.epoch += 1;
            let mut pending = vec![false; self.workers.len()];
            for (worker, input) in self.workers.iter().enumerate() {
                pending[worker] = input.try_send(Work::Select { epoch: self.epoch, deadline }).is_ok();
            }
            let mut best_value = f64::NEG_INFINITY;
            let mut chosen = None;
            while pending.iter().any(|&v| v) {
                let Ok(reply) = self.replies.recv_timeout(deadline.saturating_duration_since(Instant::now())) else { break; };
                if reply.epoch != self.epoch { continue; }
                pending[reply.worker] = false;
                let Answer::Select(candidate) = reply.answer else { continue; };
                if validate_kinds(&serde_json::json!(candidate), kinds.len()).is_err() { continue; }
                if chosen.is_none() || reply.value > best_value ||
                    (reply.value == best_value && Some(reply.worker) < chosen) {
                    kinds = candidate;
                    best_value = reply.value;
                    self.warm = reply.warm.map(Arc::new);
                    chosen = Some(reply.worker);
                }
            }
            eprintln!("p3 select worker={chosen:?} value={best_value:.3} unfinished={}", pending.iter().filter(|&&v| v).count());
        }
        self.game = Some(Game::new(self.problem.clone(), &kinds));
        self.last = None;
        kinds
    }

    fn synchronize(&mut self, obs: &Observation, accepted: bool) {
        let game = self.game.as_mut().expect("selection precedes plan");
        if let Some(last) = self.last.take() {
            let plan = if accepted { last } else { game.waiting_plan() };
            match game.play_day(plan) {
                Ok(day) => for frame in day.frames {
                    for event in frame.events {
                        if let Event::Collect { brand, .. } = event { self.seen |= 1u64 << brand; }
                    }
                },
                Err(error) => eprintln!("p3 history validation failed: {}", error.message),
            }
        }
        game.agents = obs.agents.clone();
        game.day = obs.day;
        game.set_observed_traffic(obs.traffics.clone());
    }

    pub fn plan(&mut self, obs: &Observation, steps: u32, limit: u64, accepted: bool) -> Plan {
        let deadline = Self::deadline(limit);
        self.synchronize(obs, accepted);
        let mut chosen_plan = vec![vec![-i64::from(steps)]; obs.agents.len()];
        if limit <= 110 {
            self.warm = None;
            self.last = Some(chosen_plan.clone());
            return chosen_plan;
        }
        self.ensure_workers();
        self.epoch += 1;
        let game = Arc::new(self.game.as_ref().unwrap().clone());
        let observation = Arc::new(obs.clone());
        let mut pending = vec![false; self.workers.len()];
        for (worker, input) in self.workers.iter().enumerate() {
            // Half the islands inherit the chosen endpoint plan; the other half
            // retain their own re-evaluated route topology for diversity.
            let warm = if worker % 2 == 0 { self.warm.clone() } else { None };
            pending[worker] = input.try_send(Work::Plan { epoch: self.epoch, deadline,
                game: game.clone(), observation: observation.clone(), steps, seen: self.seen, warm }).is_ok();
        }
        let mut chosen = None;
        let mut best_value = f64::NEG_INFINITY;
        let mut best_score = (0, 0, 0);
        let mut completed = 0;
        while pending.iter().any(|&v| v) {
            let Ok(reply) = self.replies.recv_timeout(deadline.saturating_duration_since(Instant::now())) else { break; };
            if reply.epoch != self.epoch { continue; }
            pending[reply.worker] = false;
            let Answer::Plan(candidate) = reply.answer else { continue; };
            let mut check = (*game).clone();
            if let Err(error) = check.play_day(candidate.clone()) {
                eprintln!("p3 rejected island={} reason={}", reply.worker, error.message);
                continue;
            }
            completed += 1;
            let score = (check.score.unique_brands, check.score.daily_brands, check.score.udon);
            // New lifetime brands are lexicographically dominant. Within that
            // class use the same objective, including the small-board forecast,
            // across islands; exact engine score breaks objective ties.
            let better = chosen.is_none() || score.0 > best_score.0 ||
                (score.0 == best_score.0 && (reply.value > best_value ||
                    (reply.value == best_value && (score > best_score ||
                        (score == best_score && Some(reply.worker) < chosen)))));
            if better {
                chosen_plan = candidate;
                best_value = reply.value;
                best_score = score;
                self.warm = reply.warm.map(Arc::new);
                chosen = Some(reply.worker);
            }
        }
        eprintln!("p3 day={} worker={chosen:?} completed={completed} value={best_value:.3} exact_score={best_score:?} unfinished={}",
            obs.day, pending.iter().filter(|&&v| v).count());
        self.last = Some(chosen_plan.clone());
        chosen_plan
    }
}

impl Drop for ParallelSolver {
    fn drop(&mut self) {
        // Joining before another match starts also preserves the hard cap if a
        // client reuses the process and sends a second selection request.
        self.workers.clear();
        for handle in self.handles.drain(..) { let _ = handle.join(); }
    }
}
