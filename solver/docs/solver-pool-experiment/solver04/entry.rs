#[path = "../pool-generation-004/itinerary.rs"]
mod itinerary;
use std::io::{self, BufRead, Write};
use tester::model::{Request, Response};
fn main() {
    let seed = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut solver = None;
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(req) = serde_json::from_str::<Request>(&line) else {
            continue;
        };
        let answer = if req.phase == "select" {
            let mut s = itinerary::Solver::new(req.problem.unwrap(), seed);
            let kinds = s.select(req.time_limit_ms);
            solver = Some(s);
            serde_json::json!(kinds)
        } else if let (Some(s), Some(state)) = (&mut solver, req.state) {
            serde_json::json!(s.plan(
                state,
                req.previous_accepted.unwrap_or(true),
                req.time_limit_ms
            ))
        } else {
            serde_json::json!([])
        };
        if serde_json::to_writer(
            &mut out,
            &Response {
                request_id: req.request_id,
                answer,
            },
        )
        .is_err()
        {
            break;
        }
        if writeln!(out).and_then(|_| out.flush()).is_err() {
            break;
        }
    }
}
