use crate::planner::{SharedSearch, Snapshot, Solver};
use std::io;
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tester::model::{Observation, Plan, Problem};

// The caller is the fifteenth thread. No worker creates threads or runtimes.
const WORKERS: usize = 14;
const TEMPERATURES: [f64; WORKERS] = [
    0.55, 0.75, 1.0, 1.3, 1.7, 2.2, 0.9, 0.65, 1.15, 1.5, 2.0, 2.6, 0.8, 1.8,
];
const BEAM_FACTORS: [f64; WORKERS] = [
    0.0, 0.35, 0.65, 1.0, 1.4, 2.0, 0.5, 1.0, 1.7, 0.25, 0.8, 1.2, 2.5, 0.5,
];

enum Search {
    Select {
        supplier: usize,
    },
    Plan {
        observation: Observation,
        steps: u32,
        state: Arc<Snapshot>,
        shared: Arc<SharedSearch>,
    },
}
struct Job {
    ticket: u64,
    deadline: Instant,
    search: Search,
}
enum Answer {
    Kinds(Vec<u8>),
    Plan(Plan),
}
struct Candidate {
    score: f64,
    state: Snapshot,
    answer: Answer,
}
struct Completion {
    ticket: u64,
    worker: usize,
    candidate: Option<Candidate>,
}
struct Pool {
    senders: Vec<mpsc::Sender<Job>>,
    receiver: mpsc::Receiver<Completion>,
    handles: Vec<JoinHandle<()>>,
}
impl Pool {
    fn new(
        problem: &Problem,
        seed: u64,
        coverage: f64,
        beam: f64,
        archive: f64,
    ) -> io::Result<Self> {
        let (reply, receiver) = mpsc::channel();
        let mut pool = Self {
            senders: vec![],
            receiver,
            handles: vec![],
        };
        for worker in 0..WORKERS {
            let problem = problem.clone();
            let replies = reply.clone();
            let (sender, commands) = mpsc::channel::<Job>();
            let handle = thread::Builder::new()
                .name(format!("p2-search-{worker:02}"))
                .spawn(move || {
                    while let Ok(job) = commands.recv() {
                        let ticket = job.ticket;
                        let candidate =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                let limit = job
                                    .deadline
                                    .saturating_duration_since(Instant::now())
                                    .as_millis() as u64;
                                if limit <= 110 {
                                    return None;
                                }
                                let worker_seed = seed.wrapping_add(
                                    (worker as u64 + 1).wrapping_mul(0x9e3779b185ebca87),
                                ) ^ ticket.wrapping_mul(0xd1b54a32d192ed03);
                                let mut solver = Solver::new(
                                    problem.clone(),
                                    worker_seed,
                                    coverage,
                                    beam * BEAM_FACTORS[worker],
                                    archive,
                                );
                                match job.search {
                                    Search::Select { supplier } => {
                                        solver.configure_parallel(TEMPERATURES[worker], None);
                                        let answer = solver.select_fixed(supplier, limit);
                                        Some(Candidate {
                                            score: solver.selection_value(),
                                            state: solver.snapshot(),
                                            answer: Answer::Kinds(answer),
                                        })
                                    }
                                    Search::Plan {
                                        observation,
                                        steps,
                                        state,
                                        shared,
                                    } => {
                                        solver
                                            .configure_parallel(TEMPERATURES[worker], Some(shared));
                                        solver.restore(&state);
                                        let answer = solver.plan(&observation, steps, limit, true);
                                        Some(Candidate {
                                            score: solver.comparison_value(),
                                            state: solver.snapshot(),
                                            answer: Answer::Plan(answer),
                                        })
                                    }
                                }
                            }))
                            .unwrap_or_else(|_| {
                                eprintln!("p2 worker={worker} ticket={ticket} search panicked");
                                None
                            });
                        if replies
                            .send(Completion {
                                ticket,
                                worker,
                                candidate,
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                })?;
            pool.senders.push(sender);
            pool.handles.push(handle);
        }
        Ok(pool)
    }
    fn collect(&self, ticket: u64, deadline: Instant, expected: usize) -> Option<Candidate> {
        let mut completed = 0;
        let mut best: Option<(usize, Candidate)> = None;
        while completed < expected {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let Ok(result) = self.receiver.recv_timeout(remaining) else {
                break;
            };
            if result.ticket != ticket {
                continue;
            }
            completed += 1;
            if let Some(candidate) = result.candidate {
                if best.as_ref().is_none_or(|(id, old)| {
                    candidate.score > old.score
                        || (candidate.score == old.score && result.worker < *id)
                }) {
                    best = Some((result.worker, candidate));
                }
            }
        }
        if let Some((worker, candidate)) = best {
            eprintln!("p2 portfolio ticket={ticket} completed={completed}/{expected} winner={worker} value={:.3}",candidate.score);
            Some(candidate)
        } else {
            eprintln!(
                "p2 portfolio ticket={ticket} completed={completed}/{expected} fallback=waiting"
            );
            None
        }
    }
}
impl Drop for Pool {
    fn drop(&mut self) {
        // Closing the command channels ends the existing workers before a new
        // problem can create another pool, so repeated select never doubles it.
        self.senders.clear();
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
    }
}

pub struct ParallelSolver {
    serial: Solver,
    problem: Problem,
    pool: Option<Pool>,
    ticket: u64,
    seed: u64,
    coverage: f64,
    beam: f64,
    archive: f64,
}
impl ParallelSolver {
    pub fn new(problem: Problem, seed: u64, coverage: f64, beam: f64, archive: f64) -> Self {
        let mut serial = Solver::new(problem.clone(), seed, coverage, beam, archive);
        serial.configure_parallel(1.0, None);
        Self {
            serial,
            problem,
            pool: None,
            ticket: 0,
            seed,
            coverage,
            beam,
            archive,
        }
    }
    fn ensure_pool(&mut self) -> bool {
        if self.pool.is_none() {
            match Pool::new(
                &self.problem,
                self.seed,
                self.coverage,
                self.beam,
                self.archive,
            ) {
                Ok(pool) => self.pool = Some(pool),
                Err(error) => {
                    eprintln!("p2 worker creation failed: {error}");
                    return false;
                }
            }
        }
        true
    }
    fn deadline(start: Instant, limit: u64) -> Instant {
        // Besides the planner's own margin, reserve time for IPC, choosing the
        // winner, dropping temporary graphs and writing the JSON response.
        start + Duration::from_millis(limit.saturating_sub((limit / 50).clamp(15, 350)))
    }
    pub fn select(&mut self, limit: u64) -> Vec<u8> {
        let began = Instant::now();
        let fallback = self.serial.select(1);
        if limit <= 110 {
            return fallback;
        }
        if !self.ensure_pool() {
            return self
                .serial
                .select(limit.saturating_sub(began.elapsed().as_millis() as u64));
        }
        self.ticket += 1;
        let ticket = self.ticket;
        let deadline = Self::deadline(began, limit);
        let pool = self.pool.as_ref().unwrap();
        let mut sent = 0;
        for (worker, sender) in pool.senders.iter().enumerate() {
            if sender
                .send(Job {
                    ticket,
                    deadline,
                    search: Search::Select {
                        supplier: worker % self.problem.agents.len(),
                    },
                })
                .is_ok()
            {
                sent += 1;
            }
        }
        if let Some(candidate) = pool.collect(ticket, deadline, sent) {
            if let Answer::Kinds(answer) = candidate.answer {
                self.serial.restore(&candidate.state);
                return answer;
            }
        }
        fallback
    }
    pub fn plan(&mut self, obs: &Observation, steps: u32, limit: u64, accepted: bool) -> Plan {
        let began = Instant::now();
        if limit <= 110 {
            return self.serial.plan(obs, steps, limit, accepted);
        }
        // Replay the one actually submitted plan exactly once on the controller.
        // Every worker receives this same state and starts with last=None.
        self.serial.prepare(obs, accepted);
        if !self.ensure_pool() {
            return self.serial.plan(
                obs,
                steps,
                limit.saturating_sub(began.elapsed().as_millis() as u64),
                true,
            );
        }
        let state = Arc::new(self.serial.snapshot());
        let shared = Arc::new(SharedSearch::default());
        self.ticket += 1;
        let ticket = self.ticket;
        let deadline = Self::deadline(began, limit);
        let pool = self.pool.as_ref().unwrap();
        let mut sent = 0;
        for sender in &pool.senders {
            let search = Search::Plan {
                observation: obs.clone(),
                steps,
                state: state.clone(),
                shared: shared.clone(),
            };
            if sender
                .send(Job {
                    ticket,
                    deadline,
                    search,
                })
                .is_ok()
            {
                sent += 1;
            }
        }
        if let Some(candidate) = pool.collect(ticket, deadline, sent) {
            if let Answer::Plan(answer) = candidate.answer {
                self.serial.restore(&candidate.state);
                return answer;
            }
        }
        self.serial.use_waiting(obs, steps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tester::engine::Game;
    use tester::model::{Map, Spot};
    #[test]
    fn persistent_pool_accepts_multiple_days_rejections_and_catch_up() {
        let p = Problem {
            starts_at: 0,
            day_seconds: vec![60; 4],
            day_steps: vec![40; 4],
            map: Map {
                height: 5,
                width: 7,
                cells: vec![vec![0; 7]; 5],
            },
            spots: [2, 5, 8, 15, 21, 27, 30, 33]
                .into_iter()
                .enumerate()
                .map(|(i, pos)| Spot {
                    brand: i / 2,
                    pos,
                    stocks: 2,
                })
                .collect(),
            agents: vec![0, 6, 28, 34],
            fuel_limits: 16,
            players: 6,
            busy_threshold: 2,
            jammed_threshold: 5,
        };
        let mut solver = ParallelSolver::new(p.clone(), 0, 0.0, 2.0, 0.0);
        let kinds = solver.select(900);
        assert_eq!(kinds.iter().filter(|&&k| k == 1).count(), 1);
        let mut actual = Game::new(p, &kinds);
        for day in 0..4 {
            let accepted = day != 2;
            let observation = actual.observation(0);
            let plan = solver.plan(&observation, 40, if day == 2 { 1 } else { 900 }, accepted);
            let mut check = actual.clone();
            check.play_day(plan.clone()).unwrap();
            if day == 1 {
                actual.play_day(actual.waiting_plan()).unwrap();
            } else {
                actual.play_day(plan).unwrap();
            }
        }
        assert!(actual.score.udon > 0);
        assert_eq!(solver.pool.as_ref().unwrap().handles.len(), WORKERS);
    }
}
