#[path = "solver/planner.rs"]
mod planner;
use std::io::{self, BufRead, Write};
use tester::model::{Problem, Request};

fn main() {
    let exploration_seed = std::env::args().nth(1).and_then(|x| x.parse::<u64>().ok()).unwrap_or(0);
    let coverage_weight = std::env::args().nth(2).and_then(|x| x.parse::<f64>().ok()).filter(|x| x.is_finite() && *x >= 0.0 && *x <= 20.0).unwrap_or(0.0);
    let beam_time_weight = std::env::args().nth(3).and_then(|x| x.parse::<f64>().ok()).filter(|x| x.is_finite() && *x >= 0.0 && *x <= 20.0).unwrap_or(2.0);
    let archive_weight = std::env::args().nth(4).and_then(|x| x.parse::<f64>().ok()).filter(|x| x.is_finite() && *x >= 0.0 && *x <= 1.5).unwrap_or(0.0);
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout());
    let mut solver: Option<planner::ParallelSolver> = None;
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(req) = serde_json::from_str::<Request>(&line) else { continue };
        let answer = if req.phase == "select" {
            let Some(p): Option<Problem> = req.problem else { continue };
            let mut s = planner::ParallelSolver::new(p, exploration_seed, coverage_weight, beam_time_weight, archive_weight);
            let kinds = s.select(req.time_limit_ms);
            solver = Some(s);
            serde_json::json!(kinds)
        } else if let (Some(s), Some(obs)) = (solver.as_mut(), req.state) {
            serde_json::json!(s.plan(&obs, req.steps.unwrap_or(1), req.time_limit_ms, req.previous_accepted.unwrap_or(true)))
        } else { serde_json::json!([]) };
        let response = serde_json::json!({"requestId": req.request_id, "answer": answer});
        if writeln!(stdout, "{}", response).is_err() || stdout.flush().is_err() { break; }
    }
}
