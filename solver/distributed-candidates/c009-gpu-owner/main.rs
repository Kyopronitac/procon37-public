#[path = "solver/planner.rs"]
mod planner;
mod runtime;
mod wire;
use std::io::{self, BufRead, Write};
use tester::model::{Problem, Request};

fn main() {
    if let Err(error) = run() {
        eprintln!("distributed fatal: {error}");
        std::process::exit(2);
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let options = runtime::Options::parse()?;
    if options.mode == "worker" {
        return runtime::daemon(options);
    }
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout());
    if options.mode == "parallel" {
        eprintln!(
            "distributed mode=parallel total_threads={} seed={}",
            options.threads, options.seed
        );
        let mut solver: Option<planner::PoolSolver> = None;
        for line in stdin.lock().lines() {
            let line = line?;
            let Ok(req) = serde_json::from_str::<Request>(&line) else {
                continue;
            };
            let answer = if req.phase == "select" {
                let Some(p): Option<Problem> = req.problem else {
                    continue;
                };
                drop(solver.take());
                let mut s = planner::PoolSolver::new(p, options.seed, options.threads);
                let kinds = s.select(req.time_limit_ms);
                solver = Some(s);
                kinds
            } else if let (Some(s), Some(obs)) = (solver.as_mut(), req.state) {
                s.plan(
                    &obs,
                    req.steps.unwrap_or(1),
                    req.time_limit_ms,
                    req.previous_accepted.unwrap_or(true),
                )
            } else {
                serde_json::json!([])
            };
            writeln!(
                stdout,
                "{}",
                serde_json::json!({"requestId":req.request_id,"answer":answer})
            )?;
            stdout.flush()?;
        }
    } else {
        let mut coordinator = runtime::Coordinator::new(options)?;
        for line in stdin.lock().lines() {
            let line = line?;
            let Ok(req) = serde_json::from_str::<Request>(&line) else {
                continue;
            };
            let answer = coordinator.solve(&req)?;
            writeln!(
                stdout,
                "{}",
                serde_json::json!({"requestId":req.request_id,"answer":answer})
            )?;
            stdout.flush()?;
        }
    }
    Ok(())
}
