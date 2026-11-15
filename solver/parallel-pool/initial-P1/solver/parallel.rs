//! A bounded portfolio of independent searches, followed by parallel next-day probes.
//! Only owned state crosses threads; route graphs and their RefCells stay local.
use super::*;
use std::thread;

const MAX_WORKERS: usize = 14;

#[derive(Clone)]
struct Snapshot {
    p: Problem,
    opponent_stays: Option<(usize, Vec<f64>)>,
    forecast_other: Vec<f64>,
    supplier: usize,
    game: Option<Game>,
    last: Option<Plan>,
    rng: u64,
    cache: Option<[Solution; 2]>,
    cache_positions: Vec<usize>,
    cache_day: usize,
    cache_complete: bool,
    seen_brands: u64,
    coverage_weight: f64,
    beam_time_weight: f64,
    archive_weight: f64,
    future_weight: f64,
    reserve_weight: f64,
    last_value: f64,
}

impl Snapshot {
    fn take(s: &Solver) -> Self {
        Self {
            p: s.p.clone(), opponent_stays: s.opponent_stays.clone(),
            forecast_other: s.forecast_other.clone(), supplier: s.supplier,
            game: s.game.clone(), last: s.last.clone(), rng: s.rng.0,
            cache: s.cache.clone(), cache_positions: s.cache_positions.clone(),
            cache_day: s.cache_day, cache_complete: s.cache_complete,
            seen_brands: s.seen_brands, coverage_weight: s.coverage_weight,
            beam_time_weight: s.beam_time_weight, archive_weight: s.archive_weight,
            future_weight: s.future_weight, reserve_weight: s.reserve_weight,
            last_value: s.last_value,
        }
    }

    fn solver(self) -> Solver {
        Solver {
            p: self.p, selection_graph: None, opponent_stays: self.opponent_stays,
            forecast_other: self.forecast_other, supplier: self.supplier,
            game: self.game, last: self.last, rng: Rng(self.rng), cache: self.cache,
            cache_positions: self.cache_positions, cache_day: self.cache_day,
            cache_complete: self.cache_complete, selecting: false,
            seen_brands: self.seen_brands, coverage_weight: self.coverage_weight,
            beam_time_weight: self.beam_time_weight, archive_weight: self.archive_weight,
            future_weight: self.future_weight, reserve_weight: self.reserve_weight,
            last_value: self.last_value,
        }
    }

    fn diversify(&mut self, worker: usize, round: u64) {
        self.rng ^= (worker as u64 + 1).wrapping_mul(0x9e3779b97f4a7c15)
            ^ round.wrapping_mul(0xd1b54a32d192ed03);
        self.rng |= 1;
        // This parameter orders beam prefixes only; terminal scores stay comparable.
        const TIME_WEIGHTS: [f64; MAX_WORKERS] = [2.0, 0.0, 0.5, 1.0, 1.5, 3.0, 4.0,
            0.25, 0.75, 1.25, 2.5, 3.5, 5.0, 6.0];
        self.beam_time_weight = TIME_WEIGHTS[worker % MAX_WORKERS];
        // The portfolio owns the next-day comparison and never nests parallel work.
        self.future_weight = 0.0;
        self.archive_weight = 0.0;
    }
}

#[derive(Clone)]
struct Candidate {
    worker: usize,
    state: Snapshot,
    plan: Plan,
    value: f64,
    score: tester::model::Score,
    selection_value: f64,
    future_brands: usize,
}

fn remaining(deadline: Instant) -> u64 {
    deadline.saturating_duration_since(Instant::now()).as_millis() as u64
}

fn candidate(worker: usize, solver: &Solver, plan: Plan) -> Option<Candidate> {
    let mut actual = solver.game.as_ref()?.clone();
    actual.play_day(plan.clone()).ok()?;
    Some(Candidate {
        worker, state: Snapshot::take(solver), plan, value: solver.last_value,
        score: actual.score, selection_value: solver.last_value, future_brands: 0,
    })
}

// All handles are joined before returning or starting the next batch. The main
// thread does not create a runtime or a nested pool: 1 control + <=14 workers.
fn join_all(handles: Vec<thread::JoinHandle<Option<Candidate>>>) -> Vec<Candidate> {
    handles.into_iter().filter_map(|h| h.join().ok().flatten()).collect()
}

pub struct ParallelSolver {
    solver: Solver,
    workers: usize,
    round: u64,
}

impl ParallelSolver {
    pub fn new(p: Problem, seed: u64, coverage: f64, beam: f64, archive: f64) -> Self {
        let threads = std::env::var("PROCON37_P1_THREADS").ok()
            .and_then(|v| v.parse::<usize>().ok()).unwrap_or(15).clamp(1, 15);
        Self { solver: Solver::new(p, seed, coverage, beam, archive),
            workers: threads - 1, round: 0 }
    }

    pub fn select(&mut self, limit: u64) -> Vec<u8> {
        if limit < 1500 || self.workers == 0 || self.solver.p.agents.len() < 3 {
            return self.solver.select(limit);
        }
        let began = Instant::now();
        let deadline = began + Duration::from_millis(limit.saturating_sub(200).min(limit * 96 / 100));
        let first_deadline = began + deadline.duration_since(began) * 3 / 5;
        let mut handles = Vec::new();
        let n = self.solver.p.agents.len();
        for worker in 0..self.workers {
            let mut state = Snapshot::take(&self.solver);
            state.diversify(worker, self.round);
            let role = worker % n;
            let handle = thread::Builder::new().name(format!("p1-select-{worker}"))
                .spawn(move || {
                    let mut solver = state.solver();
                    let mut kinds = vec![0; n]; kinds[role] = 1;
                    solver.supplier = role;
                    solver.game = Some(Game::new(solver.p.clone(), &kinds));
                    let obs = solver.game.as_ref().unwrap().observation(0);
                    let plan = solver.plan_inner(&obs, solver.p.day_steps[0], remaining(first_deadline), true, false);
                    let mut result = candidate(worker, &solver, plan)?;
                    let mut previous_obs = obs;
                    // Compare the same number of days for all roles at normal limits.
                    for day in 1..solver.p.day_steps.len().min(3) {
                        if remaining(deadline) < 400 { break; }
                        let before = solver.game.as_ref().unwrap();
                        let mut after = before.clone();
                        after.play_day(solver.last.as_ref()?.clone()).ok()?;
                        let mut forecast = after.observation(0);
                        forecast.traffics = solver.forecast_traffic(before.last_stays(), after.last_stays(), &previous_obs.traffics);
                        let divisions = if day == 1 && solver.p.day_steps.len() > 2 { 2 } else { 1 };
                        let budget = remaining(deadline) / divisions;
                        solver.seen_brands = u64::MAX;
                        solver.plan_inner(&forecast, solver.p.day_steps[day], budget, true, false);
                        result.selection_value += if day == 1 { 0.5 } else { 0.25 } * solver.last_value;
                        previous_obs = forecast;
                    }
                    // Selection has not played day 0. Retain its warm route only.
                    result.state.last = None;
                    result.state.seen_brands = 0;
                    Some(result)
                });
            match handle { Ok(h) => handles.push(h), Err(e) => eprintln!("p1 worker spawn failed: {e}") }
        }
        let results = join_all(handles);
        if let Some(best) = results.into_iter().max_by(|a,b| a.selection_value.total_cmp(&b.selection_value)) {
            eprintln!("p1 select worker={} supplier={} value={:.3} threads={} elapsed_ms={}", best.worker,
                best.state.supplier, best.selection_value, self.workers+1, began.elapsed().as_millis());
            self.solver = best.state.solver();
        } else {
            return self.solver.select(remaining(deadline));
        }
        self.round += 1;
        let mut kinds = vec![0; n]; kinds[self.solver.supplier] = 1; kinds
    }

    pub fn plan(&mut self, obs: &Observation, steps: u32, limit: u64, accepted: bool) -> Plan {
        if limit < 1500 || self.workers == 0 || obs.agents.iter().filter(|a|a.kind==0).count() < 2 {
            return self.solver.plan(obs, steps, limit, accepted);
        }
        let began = Instant::now();
        let deadline = began + Duration::from_millis(limit.saturating_sub(250).min(limit * 96 / 100));
        let forecast_enabled = obs.day+1 < self.solver.p.day_steps.len() && limit >= 6000;
        let first_deadline = if forecast_enabled { began + deadline.duration_since(began) * 3 / 4 } else { deadline };
        let mut handles = Vec::new();
        for worker in 0..self.workers {
            let mut state = Snapshot::take(&self.solver);
            state.diversify(worker, self.round);
            let obs = obs.clone();
            let handle = thread::Builder::new().name(format!("p1-search-{worker}"))
                .spawn(move || {
                    let mut solver = state.solver();
                    let plan = solver.plan_with_archive(&obs, steps, remaining(first_deadline), accepted, false);
                    candidate(worker, &solver, plan)
                });
            match handle { Ok(h) => handles.push(h), Err(e) => eprintln!("p1 worker spawn failed: {e}") }
        }
        let mut results = join_all(handles);
        if results.is_empty() { return self.solver.plan(obs, steps, remaining(deadline), accepted); }
        results.sort_by(|a,b| b.value.total_cmp(&a.value));
        let fallback = results[0].clone();
        let current_best = results[0].value;
        if forecast_enabled && remaining(deadline) >= 600 {
            // Avoid trading away today's coverage just for an optimistic forecast.
            results.retain(|c| c.value >= current_best - 120.0);
            let mut handles = Vec::new();
            for mut result in results {
                let mut state = result.state.clone();
                state.diversify(result.worker, self.round+0x10000);
                let obs = obs.clone();
                let handle = thread::Builder::new().name(format!("p1-forecast-{}",result.worker))
                    .spawn(move || {
                        let mut solver = state.solver();
                        let before = solver.game.as_ref()?;
                        let mut after = before.clone();
                        after.play_day(result.plan.clone()).ok()?;
                        let mut forecast = after.observation(0);
                        forecast.traffics = solver.forecast_traffic(before.last_stays(), after.last_stays(), &obs.traffics);
                        let tomorrow = solver.p.day_steps[forecast.day];
                        let next_plan = solver.plan_inner(&forecast, tomorrow, remaining(deadline), true, false);
                        let mut next = solver.game.as_ref()?.clone();
                        next.play_day(next_plan).ok()?;
                        result.future_brands = next.score.daily_brands - result.score.daily_brands;
                        let new_unique = next.score.unique_brands - result.score.unique_brands;
                        result.selection_value = result.value + 1000.0 * result.future_brands as f64
                            + 100000.0 * new_unique as f64;
                        // Preserve the forecast route using coordinates; graph node IDs
                        // may differ after observed traffic and cache targets change.
                        if let (Some(old), Some(future)) = (result.state.cache.as_mut(), solver.cache.as_ref()) {
                            let mut next_route = future[0].clone();
                            for hub in &mut next_route.hubs {
                                let pos = solver.cache_positions[*hub];
                                *hub = result.state.cache_positions.iter().position(|&p|p==pos)
                                    .expect("every passable cell has a graph node");
                            }
                            old[1] = next_route;
                        }
                        Some(result)
                    });
                match handle { Ok(h) => handles.push(h), Err(e) => eprintln!("p1 worker spawn failed: {e}") }
            }
            results = join_all(handles);
            if results.is_empty() { results.push(fallback); }
        }
        let final_day = obs.day+1 == self.solver.p.day_steps.len();
        let best = results.into_iter().max_by(|a,b| {
            if final_day {
                (a.score.unique_brands,a.score.daily_brands,a.score.udon)
                    .cmp(&(b.score.unique_brands,b.score.daily_brands,b.score.udon))
                    .then_with(||a.value.total_cmp(&b.value))
            } else { a.selection_value.total_cmp(&b.selection_value) }
        });
        if let Some(best) = best {
            eprintln!("p1 plan day={} worker={} value={:.3} forecast_brands={} compared={:.3} threads={} elapsed_ms={}",
                obs.day,best.worker,best.value,best.future_brands,best.selection_value,self.workers+1,began.elapsed().as_millis());
            let plan = best.plan;
            self.solver = best.state.solver();
            self.round += 1;
            plan
        } else {
            self.solver.plan(obs, steps, remaining(deadline), accepted)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tester::model::{Map, Spot};

    fn problem() -> Problem {
        Problem { starts_at: 0, day_seconds: vec![60;4], day_steps: vec![40;4],
            map: Map { height: 5, width: 7, cells: vec![vec![0;7];5] },
            spots: [2,8,15,21,27,33].into_iter().enumerate()
                .map(|(i,pos)| Spot { brand: i/2, pos, stocks: 2 }).collect(),
            agents: vec![0,10,24,34], fuel_limits: 16, players: 6,
            busy_threshold: 2, jammed_threshold: 5 }
    }

    #[test]
    fn selected_history_is_shared_after_acceptance_and_rejection() {
        let mut solver = ParallelSolver::new(problem(), 17, 0.0, 2.0, 0.0);
        let kinds = solver.select(1600);
        assert_eq!(kinds.iter().filter(|&&k|k==1).count(),1);
        assert!(solver.solver.last.is_none());
        let mut game = Game::new(problem(), &kinds);
        let mut obs = game.observation(0);
        obs.others = vec![serde_json::json!({"id":4,"agents":[{"pos":6},{"pos":28}]})];
        let first = solver.plan(&obs,40,2000,true);
        game.play_day(first).unwrap();
        let second = solver.plan(&game.observation(0),40,2000,true);
        assert_eq!(solver.solver.game.as_ref().unwrap().score,game.score);
        assert_eq!(solver.solver.game.as_ref().unwrap().agents,game.agents);
        assert_eq!(solver.solver.last.as_ref().unwrap(),&second);
        // The second submitted plan is rejected: all workers must replay a wait.
        game.play_day(game.waiting_plan()).unwrap();
        let third = solver.plan(&game.observation(0),40,2000,false);
        assert_eq!(solver.solver.game.as_ref().unwrap().score,game.score);
        assert_eq!(solver.solver.game.as_ref().unwrap().agents,game.agents);
        game.play_day(third).unwrap();
        let caught_up = solver.plan(&game.observation(0),40,1,true);
        assert_eq!(solver.solver.game.as_ref().unwrap().score,game.score);
        assert_eq!(caught_up,game.waiting_plan());
    }

    #[test]
    fn forecast_keeps_actual_game_and_coordinate_warm_start() {
        let mut solver = ParallelSolver::new(problem(), 31, 0.0, 2.0, 0.0);
        let kinds = solver.select(1);
        let mut game = Game::new(problem(), &kinds);
        let plan = solver.plan(&game.observation(0),40,6200,true);
        assert_eq!(solver.solver.game.as_ref().unwrap().day,0);
        assert_eq!(solver.solver.game.as_ref().unwrap().score,game.score);
        game.play_day(plan).unwrap();
        let next = solver.plan(&game.observation(0),40,1800,true);
        assert_eq!(solver.solver.game.as_ref().unwrap().score,game.score);
        game.play_day(next).unwrap();
        assert!(game.score.daily_brands > 0);
    }
}
