use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};
use std::time::{Duration, Instant};
use tester::engine::{Game, validate_kinds};
use tester::model::{Map, Problem, Response, Spot};

#[test]
fn json_lines_history_role_recovery_and_thread_cap() {
    let p = Problem {
        starts_at: 0, day_seconds: vec![60; 5], day_steps: vec![40; 5],
        map: Map { height: 5, width: 7, cells: vec![vec![0; 7]; 5] },
        spots: [2, 8, 15, 21, 27, 33].into_iter().enumerate()
            .map(|(brand, pos)| Spot { brand, pos, stocks: 2 }).collect(),
        agents: vec![0, 10, 24, 34], fuel_limits: 16, players: 6,
        busy_threshold: 2, jammed_threshold: 5,
    };
    let mut process = Command::new(env!("CARGO_BIN_EXE_solver"))
        .env("PROCON37_P3_WORKERS", "999")
        .env("PROCON37_G007_FUTURE_WEIGHT", "0")
        .env("PROCON37_G007_RESERVE_WEIGHT", "15")
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
        .spawn().unwrap();
    let pid = process.id();
    let done = Arc::new(AtomicBool::new(false));
    let peak = Arc::new(AtomicUsize::new(0));
    let watch_done = done.clone(); let watch_peak = peak.clone();
    let monitor = std::thread::spawn(move || {
        while !watch_done.load(Ordering::Relaxed) {
            if let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) {
                if let Some(threads) = status.lines().find_map(|line| line.strip_prefix("Threads:")
                    .and_then(|value| value.trim().parse::<usize>().ok())) {
                    watch_peak.fetch_max(threads, Ordering::Relaxed);
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    });
    let mut input = process.stdin.take().unwrap();
    let mut output = BufReader::new(process.stdout.take().unwrap());
    let mut exchange = |request: serde_json::Value| {
        let start = Instant::now();
        writeln!(input, "{request}").unwrap(); input.flush().unwrap();
        let mut line = String::new(); output.read_line(&mut line).unwrap();
        let response: Response = serde_json::from_str(&line).unwrap();
        assert_eq!(response.request_id, request["requestId"].as_u64().unwrap());
        assert!(start.elapsed() < Duration::from_millis(1600), "response missed smoke deadline");
        response.answer
    };
    let answer = exchange(serde_json::json!({ "protocolVersion": 1, "requestId": 0,
        "phase": "select", "timeLimitMs": 1000, "problem": p }));
    let kinds = validate_kinds(&answer, p.agents.len()).unwrap();
    assert_eq!(kinds.iter().filter(|&&kind| kind == 1).count(), 1);
    let mut game = Game::new(p, &kinds);
    for day in 0..5 {
        // Day 1 rejects the preceding answer. Day 3 changes the public roles,
        // exercising recovery from a rejected type selection/reconnect.
        if day == 3 { for agent in &mut game.agents { agent.kind = 0; } }
        let mut obs = game.observation(0);
        obs.others = (0..6).filter(|&id| id != 3).map(|id|
            serde_json::json!({"id": id, "agents": obs.agents})).collect();
        let answer = exchange(serde_json::json!({ "protocolVersion": 1, "requestId": day + 1,
            "phase": "plan", "timeLimitMs": if day == 2 {1} else {1000},
            "state": obs, "steps": 40, "previousAccepted": day != 1 }));
        let plan = serde_json::from_value(answer).unwrap();
        let mut validation = game.clone(); validation.play_day(plan).unwrap();
        if day == 0 { let wait = game.waiting_plan(); game.play_day(wait).unwrap(); }
        else { game = validation; }
    }
    assert!(game.score.udon > 0);
    drop(exchange); drop(input);
    assert!(process.wait().unwrap().success());
    done.store(true, Ordering::Relaxed); monitor.join().unwrap();
    let measured = peak.load(Ordering::Relaxed);
    assert_eq!(measured, 15, "expected exactly controller + 14 workers, measured {measured}");
    eprintln!("P3 JSON Lines smoke: peak_threads={measured}, final_score={:?}", game.score);
}
