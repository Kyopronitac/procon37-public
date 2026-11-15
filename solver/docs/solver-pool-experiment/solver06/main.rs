#[path = "solver/planner.rs"]
mod planner;

use std::io::{self, BufRead, Write};
use std::time::{Duration, Instant};
use tester::model::{Request, Response};

fn main() {
    let mut solver: Option<planner::Solver> = None;
    let mut previous: Option<tester::model::Plan> = None;
    let seed = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let stdin = io::stdin();
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(req) = serde_json::from_str::<Request>(&line) else { continue };
        let start = Instant::now();
        let ms = req.time_limit_ms.saturating_sub(80).max(1);
        let deadline = start + Duration::from_millis(ms * 92 / 100);
        let answer = if req.phase == "select" {
            let Some(problem) = req.problem else { continue };
            let mut s = planner::Solver::new(problem, seed);
            let kinds = s.select(deadline);
            solver = Some(s);
            serde_json::json!(kinds)
        } else {
            let Some(s) = solver.as_mut() else { continue };
            let Some(observation) = req.state else { continue };
            s.observe(&observation, previous.take(), req.previous_accepted.unwrap_or(true));
            let plan = s.plan(deadline);
            previous = Some(plan.clone());
            serde_json::json!(plan)
        };
        let response = Response { request_id: req.request_id, answer };
        if serde_json::to_writer(&mut out, &response).is_err() { break; }
        if writeln!(&mut out).is_err() || out.flush().is_err() { break; }
    }
}
