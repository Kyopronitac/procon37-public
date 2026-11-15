#[path = "solver/search.rs"]
mod search;
#[path = "solver/network.rs"]
mod network;

use std::io::{self, BufRead, Write};
use std::time::{Duration, Instant};
use tester::{engine::Game, model::*};

fn main() {
    let seed = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut planner = search::Planner::new(seed);
    let mut game: Option<Game> = None;
    let mut previous: Option<Plan> = None;
    let mut output = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(req) = serde_json::from_str::<Request>(&line) else { continue };
        let started = Instant::now();
        let budget = req.time_limit_ms.saturating_sub(40).max(1) as f64 * 0.97;
        let deadline = started + Duration::from_millis(budget as u64);
        let answer = if req.phase == "select" {
            let Some(p) = req.problem else { continue };
            let kinds = planner.select(&p, deadline, budget as u64);
            game = Some(Game::new(p, &kinds));
            serde_json::json!(kinds)
        } else {
            let Some(g) = game.as_mut() else { continue };
            let Some(obs) = req.state else { continue };
            if g.day < obs.day {
                let plan = if req.previous_accepted == Some(false) { g.waiting_plan() } else { previous.take().unwrap_or_else(|| g.waiting_plan()) };
                match g.play_day(plan) {
                    Ok(day) => planner.remember(&day),
                    Err(_) => { if let Ok(day)=g.play_day(g.waiting_plan()) {planner.remember(&day);} }
                }
            }
            g.agents = obs.agents;
            g.day = obs.day;
            g.set_observed_traffic(obs.traffics);
            let mut plan = planner.plan(g, deadline);
            let mut check = g.clone();
            if let Err(e) = check.play_day(plan.clone()) {
                eprintln!("validation fallback: {}", e.message);
                plan = g.waiting_plan();
            }
            previous = Some(plan.clone());
            serde_json::json!(plan)
        };
        let response = Response { request_id: req.request_id, answer };
        let _ = serde_json::to_writer(&mut output, &response);
        let _ = output.write_all(b"\n");
        let _ = output.flush();
    }
}
