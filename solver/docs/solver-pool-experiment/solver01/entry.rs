#[path = "../pool-generation-001/horizon.rs"]
mod horizon;
use std::io::{self, BufRead, Write};
use tester::model::{Request, Response};
fn main() {
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    let seed = std::env::args()
        .nth(1)
        .and_then(|x| x.parse::<u64>().ok())
        .unwrap_or(0);
    let mut solver = None;
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(req) = serde_json::from_str::<Request>(&line) else {
            continue;
        };
        let answer = if req.phase == "select" {
            let mut s = horizon::Solver::new(req.problem.unwrap(), seed);
            let kinds = s.select(req.time_limit_ms);
            solver = Some(s);
            serde_json::json!(kinds)
        } else {
            let state = req.state.unwrap();
            serde_json::json!(solver.as_mut().unwrap().plan(
                state,
                req.time_limit_ms,
                req.previous_accepted.unwrap_or(true)
            ))
        };
        let response = Response {
            request_id: req.request_id,
            answer,
        };
        serde_json::to_writer(&mut stdout, &response).unwrap();
        writeln!(stdout).unwrap();
        stdout.flush().unwrap();
    }
}
