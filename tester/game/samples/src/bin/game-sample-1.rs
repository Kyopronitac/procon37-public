#[path = "../pool-generation-002/rendezvous.rs"]
mod rendezvous;

use rendezvous::Optimizer;
use std::io::{self, BufRead, Write};
use tester::{engine::Game, model::*};

fn main() {
    let seed = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut optimizer = Optimizer::new(seed);
    let mut game: Option<Game> = None;
    let mut previous: Option<(Game, Plan)> = None;
    let mut seen = 0u64;
    let stdin = io::stdin();
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(request) = serde_json::from_str::<Request>(&line) else {
            continue;
        };
        let answer = if request.phase == "select" {
            let p = request.problem.unwrap();
            let kinds = optimizer.select(&p, request.time_limit_ms);
            game = Some(Game::new(p, &kinds));
            serde_json::to_value(kinds).unwrap()
        } else {
            let observation = request.state.unwrap();
            if let Some((mut before, planned)) = previous.take() {
                let plan = if request.previous_accepted.unwrap_or(true) {
                    planned
                } else {
                    before.waiting_plan()
                };
                if let Ok(replay) = before.play_day(plan) {
                    for frame in &replay.frames {
                        for event in &frame.events {
                            if let Event::Collect { brand, .. } = event {
                                seen |= 1u64 << brand;
                            }
                        }
                    }
                }
                game = Some(before);
            }
            let g = game.as_mut().unwrap();
            g.agents = observation.agents;
            g.day = observation.day;
            g.set_observed_traffic(observation.traffics);
            let mut plan = optimizer.plan(g, seen, request.time_limit_ms);
            if let Err(error) = g.clone().play_day(plan.clone()) {
                eprintln!("validation failed: {}", error.message);
                plan = g.waiting_plan();
            }
            previous = Some((g.clone(), plan.clone()));
            serde_json::to_value(plan).unwrap()
        };
        let response = Response {
            request_id: request.request_id,
            answer,
        };
        writeln!(out, "{}", serde_json::to_string(&response).unwrap()).unwrap();
        out.flush().unwrap();
    }
}
