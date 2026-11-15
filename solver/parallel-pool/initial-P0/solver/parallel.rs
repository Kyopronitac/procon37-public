//! P0: bounded independent search islands over shared immutable path tables.
//! The only thread creation lives in `run_workers`: at most fourteen workers
//! plus the JSON Lines controller. Forecasts call the serial Solver directly.
use super::*;

const WORKERS: usize = 14;

struct Candidate {
    worker: usize,
    solver: Solver,
    value: f64,
}

pub struct ParallelSolver {
    solver: Solver,
    seed: u64,
    beam_time_weight: f64,
}

impl ParallelSolver {
    pub fn new(p: Problem, seed: u64, coverage: f64, beam: f64, _archive: f64) -> Self {
        let mut solver = Solver::new(p, seed, coverage, beam, 0.0);
        // Parallel lookahead is orchestrated here; never start nested islands.
        solver.future_weight = 0.0;
        Self { solver, seed, beam_time_weight: beam }
    }

    fn fork(&self, worker: usize, day: usize) -> Solver {
        let mut solver = self.solver.clone();
        let mixed = self.seed.wrapping_add((worker as u64 + 1).wrapping_mul(0x9e3779b185ebca87))
            ^ (day as u64 + 1).wrapping_mul(0xc2b2ae3d27d4eb4f);
        solver.rng.0 = (solver.rng.u() ^ mixed) | 1;
        solver.temperature_scale = [0.65, 0.85, 1.0, 1.15, 1.4, 1.8, 2.2][worker % 7];
        // Beam ordering changes exploration but not the comparable final objective.
        solver.beam_time_weight = self.beam_time_weight * [0.5, 1.0, 1.5, 2.0][worker % 4];
        solver
    }

    fn run_workers<F>(jobs: Vec<(usize, Solver)>, work: F) -> Vec<Candidate>
    where
        F: Fn(usize, Solver) -> Candidate + Sync,
    {
        assert!(jobs.len() <= WORKERS);
        std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(jobs.len());
            for (id, solver) in jobs {
                let work = &work;
                match std::thread::Builder::new().name(format!("p0-search-{id}")).spawn_scoped(scope, move || work(id, solver)) {
                    Ok(handle) => handles.push(handle),
                    Err(error) => eprintln!("p0 worker_spawn_failed worker={id} error={error}"),
                }
            }
            handles.into_iter().filter_map(|handle| match handle.join() {
                Ok(candidate) if candidate.value.is_finite() => Some(candidate),
                Ok(_) => None,
                Err(_) => { eprintln!("p0 worker_panicked"); None }
            }).collect()
        })
    }

    fn remaining(deadline: Instant) -> u64 {
        deadline.saturating_duration_since(Instant::now()).as_millis() as u64
    }

    pub fn select(&mut self, limit: u64) -> Vec<u8> {
        if limit < 2000 || self.solver.p.agents.len() < 3 {
            return self.solver.select(limit);
        }
        let started = Instant::now();
        let deadline = started + Duration::from_millis(limit.saturating_sub(180).min(limit * 94 / 100));
        let n = self.solver.p.agents.len();
        let obs = Game::new(self.solver.p.clone(), &vec![0; n]).observation(0);
        self.solver.selection_graph = Some(Graph::new(&self.solver.p, &obs, &[]));
        let available = Self::remaining(deadline);
        if available < 500 {
            return self.solver.select(available.min(110));
        }
        let days = self.solver.p.day_steps.len();
        let forecasts = if available >= 12000 { (days - 1).min(2) } else { 0 };
        let jobs = (0..WORKERS).map(|id| (id, self.fork(id, 0))).collect();
        let candidates = Self::run_workers(jobs, |id, mut solver| {
            solver.supplier = id % n;
            let mut kinds = vec![0; n];
            kinds[solver.supplier] = 1;
            solver.game = Some(Game::new(solver.p.clone(), &kinds));
            solver.last = None;
            solver.cache = None;
            solver.cache_positions.clear();
            solver.seen_brands = 0;
            let observation = solver.game.as_ref().unwrap().observation(0);
            let first_budget = if forecasts == 0 { Self::remaining(deadline) } else { Self::remaining(deadline) * 58 / 100 };
            let plan = solver.plan_inner(&observation, solver.p.day_steps[0], first_budget, true, false);
            let mut first = solver.clone();
            first.last = None;
            let mut value = first.last_value;
            let mut previous_plan = plan;
            let mut previous_obs = observation;
            let mut completed = 0;
            for day in 1..=forecasts {
                let mut game = solver.game.as_ref().unwrap().clone();
                if game.play_day(previous_plan).is_err() { break; }
                let mut next_obs = game.observation(0);
                next_obs.traffics = solver.forecast_traffic(solver.game.as_ref().unwrap().last_stays(), game.last_stays(), &previous_obs.traffics);
                let remaining = Self::remaining(deadline);
                if remaining < 600 { break; }
                solver.game = Some(game);
                solver.last = None;
                solver.selection_graph = None;
                solver.seen_brands = u64::MAX;
                let budget = remaining / (forecasts - day + 1) as u64;
                previous_plan = solver.plan_inner(&next_obs, solver.p.day_steps[day], budget, true, false);
                value += solver.last_value * if day == 1 { 0.5 } else { 0.25 };
                completed += 1;
                previous_obs = next_obs;
            }
            eprintln!("p0 role worker={id} supplier={} score={value:.3} forecast_days={completed}", first.supplier);
            Candidate { worker: id, solver: first, value }
        });
        if let Some(best) = candidates.into_iter().max_by(|a,b|a.value.total_cmp(&b.value).then_with(||b.worker.cmp(&a.worker))) {
            eprintln!("p0 select worker={} supplier={} value={:.3} elapsed_ms={} workers={WORKERS}", best.worker, best.solver.supplier, best.value, started.elapsed().as_millis());
            self.solver = best.solver;
            self.solver.temperature_scale = 1.0;
            // Keep the immutable first-day graph and first-day warm start.
            let mut kinds = vec![0; n];
            kinds[self.solver.supplier] = 1;
            kinds
        } else {
            self.solver.select(Self::remaining(deadline).min(110))
        }
    }

    pub fn plan(&mut self, obs: &Observation, steps: u32, limit: u64, accepted: bool) -> Plan {
        if limit < 2000 || obs.agents.len() < 3 || obs.agents.iter().filter(|a|a.kind == 1).count() != 1 {
            return self.solver.plan(obs, steps, limit, accepted);
        }
        let started = Instant::now();
        let deadline = started + Duration::from_millis(limit.saturating_sub(180).min(limit * 94 / 100));
        self.solver.synchronize(obs, accepted);
        let supplier = obs.agents.iter().position(|a|a.kind == 1).unwrap();
        if self.solver.supplier != supplier {
            self.solver.supplier = supplier;
            self.solver.cache = None;
            self.solver.cache_positions.clear();
            self.solver.selection_graph = None;
            self.solver.cache_complete = false;
        }
        let tree = self.solver.opponent_tree(obs, steps);
        self.solver.forecast_other = tree.clone();
        if let Some((day, previous)) = self.solver.opponent_stays.take() {
            if day + 1 == obs.day {
                for (sum, value) in self.solver.forecast_other.iter_mut().zip(previous) { *sum += value; }
            }
        }
        self.solver.opponent_stays = Some((obs.day, tree));
        let extra = self.solver.cache.as_ref().map(|cache|cache.iter().flat_map(|s|s.hubs.iter().map(|&h|self.solver.cache_positions[h])).collect::<Vec<_>>()).unwrap_or_default();
        let reusable = self.solver.selection_graph.as_ref().is_some_and(|g| {
            g.day == obs.day && obs.agents.iter().enumerate().all(|(i,a)|g.positions[self.solver.p.spots.len()+i] == a.pos)
                && obs.traffics.iter().all(|t|g.durations[t.pos] == [1,2,4][t.status as usize])
                && extra.iter().all(|&pos|g.cell_node[pos] < g.targets)
        });
        if !reusable { self.solver.selection_graph = Some(Graph::new(&self.solver.p, obs, &extra)); }
        let available = Self::remaining(deadline);
        if available < 500 { return self.solver.plan_inner(obs, steps, available.min(110), true, false); }
        let lookahead = obs.day + 1 < self.solver.p.day_steps.len() && available >= 10000;
        let primary_deadline = if lookahead { Instant::now() + Duration::from_millis(available * 77 / 100) } else { deadline };
        let jobs = (0..WORKERS).map(|id| (id, self.fork(id, obs.day + 1))).collect();
        let mut candidates = Self::run_workers(jobs, |id, mut solver| {
            solver.plan_inner(obs, steps, Self::remaining(primary_deadline), true, false);
            let value = solver.last_value;
            Candidate { worker: id, solver, value }
        });
        if candidates.is_empty() { return self.solver.plan_inner(obs, steps, Self::remaining(deadline).min(110), true, false); }
        candidates.sort_by(|a,b|b.value.total_cmp(&a.value).then_with(||a.worker.cmp(&b.worker)));
        let primary_best = candidates[0].value;
        // Only compare terminal states near today's optimum. A future estimate
        // cannot trade away a full daily brand or a newly collected global brand.
        if lookahead && Self::remaining(deadline) >= 1000 {
            candidates.retain(|entry|entry.value >= primary_best - 120.0);
            let mut unique = Vec::<Plan>::new();
            candidates.retain(|entry| {
                let plan = entry.solver.last.as_ref().unwrap();
                if unique.contains(plan) { false } else { unique.push(plan.clone()); true }
            });
            let jobs = candidates.into_iter().map(|candidate|(candidate.worker,candidate.solver)).collect();
            candidates = Self::run_workers(jobs, |id, mut today| {
                let mut value = today.last_value;
                let mut probe = today.clone();
                let current_game = today.game.as_ref().unwrap();
                let mut after = current_game.clone();
                let plan = today.last.as_ref().unwrap().clone();
                if after.play_day(plan).is_ok() {
                    let mut next_obs = after.observation(0);
                    next_obs.traffics = today.forecast_traffic(current_game.last_stays(), after.last_stays(), &obs.traffics);
                    let before = after.score.daily_brands;
                    probe.game = Some(after);
                    probe.last = None;
                    probe.selection_graph = None;
                    // Forecasts compare next-day collection, without rewarding
                    // candidates merely for having missed brands earlier.
                    probe.seen_brands = u64::MAX;
                    let next_plan = probe.plan_inner(&next_obs, probe.p.day_steps[obs.day+1], Self::remaining(deadline), true, false);
                    let mut final_game = probe.game.as_ref().unwrap().clone();
                    if final_game.play_day(next_plan).is_ok() {
                        let brands = final_game.score.daily_brands - before;
                        value += 1500.0 * brands as f64;
                        // Store the chosen forecast by coordinates, using the
                        // existing shared graph's complete cell-node mapping.
                        if let (Some(next), Some(current), Some(graph)) = (probe.cache.as_ref(), today.cache.as_mut(), today.selection_graph.as_ref()) {
                            current[1] = next[0].clone();
                            for hub in &mut current[1].hubs { *hub = graph.cell_node[probe.cache_positions[*hub]]; }
                        }
                        eprintln!("p0 forecast worker={id} day={} today={:.3} next_brands={brands} total={value:.3}", obs.day, today.last_value);
                    }
                }
                Candidate { worker: id, solver: today, value }
            });
        }
        // Worker panics are isolated, but a failed forecast must still leave a
        // valid synchronized controller available for a safe answer.
        if candidates.is_empty() { return self.solver.plan_inner(obs, steps, Self::remaining(deadline).min(110), true, false); }
        let best = candidates.into_iter().max_by(|a,b|a.value.total_cmp(&b.value).then_with(||b.worker.cmp(&a.worker))).unwrap();
        eprintln!("p0 plan day={} worker={} primary_best={primary_best:.3} primary={:.3} value={:.3} lookahead={} elapsed_ms={} workers={WORKERS}", obs.day, best.worker, best.solver.last_value, best.value, usize::from(lookahead), started.elapsed().as_millis());
        self.solver = best.solver;
        self.solver.temperature_scale = 1.0;
        // Avoid compounding the per-worker beam-order variation across days.
        self.solver.beam_time_weight = self.beam_time_weight;
        self.solver.last.as_ref().unwrap().clone()
    }
}
