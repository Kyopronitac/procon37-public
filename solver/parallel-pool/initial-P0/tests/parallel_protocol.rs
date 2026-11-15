use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};
use std::time::{Duration, Instant};
use tester::{engine::Game, model::{Plan, MatchFormat}};

#[test]
fn json_lines_parallel_multiday_rejection_and_thread_bound() {
    let mut maximum = 0;
    for (seed, format, long) in [(0, MatchFormat::First, true), (1, MatchFormat::Semifinal, false), (2, MatchFormat::Final, false)] {
        if let Ok(filter) = std::env::var("P0_PROTOCOL_SEED") {
            if filter.parse::<u64>().unwrap() != seed { continue; }
        }
        let long = long || std::env::var_os("P0_PROTOCOL_LONG").is_some();
        let p = tester::generator::generate_for_players(seed, Some(format), 6).unwrap().problem;
        let log = std::fs::File::create(format!("solver/docs/p0-protocol-seed{seed}.log")).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_solver")).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(log).spawn().unwrap();
        let pid = child.id();
        let stopped = Arc::new(AtomicBool::new(false));
        let thread_max = Arc::new(AtomicUsize::new(0));
        let watch = {
            let stopped = stopped.clone(); let thread_max = thread_max.clone();
            std::thread::spawn(move || {
                while !stopped.load(Ordering::Relaxed) {
                    if let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) {
                        if let Some(n) = status.lines().find_map(|line|line.strip_prefix("Threads:").and_then(|v|v.trim().parse::<usize>().ok())) {
                            thread_max.fetch_max(n, Ordering::Relaxed);
                        }
                    }
                    std::thread::sleep(Duration::from_millis(4));
                }
            })
        };
        let mut input = child.stdin.take().unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let mut exchange = |request: serde_json::Value| {
            let start = Instant::now();
            writeln!(input, "{request}").unwrap();input.flush().unwrap();
            let mut line = String::new(); assert!(output.read_line(&mut line).unwrap() > 0);
            let result: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(result["requestId"], request["requestId"]);
            let limit = request["timeLimitMs"].as_u64().unwrap();
            assert!(start.elapsed() < Duration::from_millis(limit + 300), "request took {:?}, limit={limit}", start.elapsed());
            eprintln!("seed={seed} phase={} id={} elapsed_ms={}", request["phase"], request["requestId"], start.elapsed().as_millis());
            result["answer"].clone()
        };
        let select_limit = if long { 14000 } else { 3000 };
        let answer = exchange(serde_json::json!({"protocolVersion":1,"requestId":0,"phase":"select","timeLimitMs":select_limit,"problem":p}));
        let kinds: Vec<u8> = serde_json::from_value(answer).unwrap();
        assert_eq!(kinds.len(), p.agents.len());assert_eq!(kinds.iter().filter(|&&k|k==1).count(),1);
        let mut game = Game::new(p.clone(), &kinds);
        for day in 0..3 {
            let limit = if day == 2 { 1 } else if long { 12000 } else { 3000 };
            let mut obs = game.observation(0);
            // Observed traffic, rather than own-only forecasts, is authoritative.
            for traffic in &mut obs.traffics { traffic.status = ((traffic.pos + day) % 3) as u8; }
            obs.others = (1..6).map(|id|serde_json::json!({"id":id,"agents":obs.agents})).collect();
            game.set_observed_traffic(obs.traffics.clone());
            let answer = exchange(serde_json::json!({"protocolVersion":1,"requestId":day+1,"phase":"plan","timeLimitMs":limit,"state":obs,"steps":p.day_steps[day],"previousAccepted":day!=1}));
            let plan: Plan = serde_json::from_value(answer).unwrap();
            let mut verified = game.clone();verified.play_day(plan).expect("submitted plan must be valid in the public engine");
            if day == 0 { game.play_day(game.waiting_plan()).unwrap(); } else { game = verified; }
        }
        drop(exchange);drop(input);
        assert!(child.wait().unwrap().success());
        stopped.store(true, Ordering::Relaxed);watch.join().unwrap();
        let count = thread_max.load(Ordering::Relaxed);maximum = maximum.max(count);
        assert_eq!(count,15,"expected fourteen workers plus controller");
        eprintln!("seed={seed} peak_threads={count} score={:?}",game.score);
    }
    assert_eq!(maximum,15);
}
