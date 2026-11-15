#[path = "../pool-generation-003/rendezvous.rs"]
mod rendezvous;

use std::io::{self, BufRead, Write};
use tester::model::{Problem, Request};

fn main() {
    let seed = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut solver: Option<rendezvous::Solver> = None;
    let stdin = io::stdin();
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(req) = serde_json::from_str::<Request>(&line) else {
            continue;
        };
        let answer = if req.phase == "select" {
            let p: Problem = req.problem.unwrap();
            let s = rendezvous::Solver::new(p, seed, req.time_limit_ms);
            let kinds = s.kinds();
            solver = Some(s);
            serde_json::json!(kinds)
        } else if let (Some(s), Some(obs)) = (solver.as_mut(), req.state) {
            serde_json::json!(s.plan(
                obs,
                req.time_limit_ms,
                req.previous_accepted.unwrap_or(true)
            ))
        } else {
            serde_json::json!([])
        };
        writeln!(
            out,
            "{}",
            serde_json::json!({"requestId":req.request_id,"answer":answer})
        )
        .unwrap();
        out.flush().unwrap();
    }
}
