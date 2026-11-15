//! Run real processes and TCP sockets, including reconnection and a rejected day.
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tester::engine::Game;
use tester::model::{Plan, Request, Response};

struct Process {
    child: Child,
    record: PathBuf,
}
impl Process {
    fn start(root: &Path, label: &str, token: &Path, args: &[String]) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_distributed-solver"))
            .args(args)
            .env("PROCON37_DISTRIBUTED_TOKEN_FILE", token)
            .env("PROCON37_G007_FUTURE_WEIGHT", "0")
            .env("PROCON37_G007_RESERVE_WEIGHT", "15")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(File::create(root.join(format!("{label}.log"))).unwrap())
            .spawn()
            .unwrap();
        let record = root.join(format!("{label}.process.json"));
        fs::write(
            &record,
            serde_json::to_string_pretty(
                &serde_json::json!({"pid":child.id(),"args":args,"stopped":false}),
            )
            .unwrap(),
        )
        .unwrap();
        Self { child, record }
    }
    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        fs::write(
            &self.record,
            serde_json::to_string_pretty(
                &serde_json::json!({"pid":self.child.id(),"stopped":true}),
            )
            .unwrap(),
        )
        .unwrap();
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.stop();
    }
}
fn port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
fn wait_listen(port: u16) {
    let start = Instant::now();
    loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(15));
}
fn worker(root: &Path, label: &str, token: &Path, port: u16, offset: usize) -> Process {
    let process = Process::start(
        root,
        label,
        token,
        &[
            "--mode".into(),
            "worker".into(),
            "--threads".into(),
            "3".into(),
            "--worker-offset".into(),
            offset.to_string(),
            "--listen".into(),
            format!("127.0.0.1:{port}"),
        ],
    );
    wait_listen(port);
    process
}
fn exchange(
    process: &mut Process,
    reader: &mut BufReader<std::process::ChildStdout>,
    request: Request,
) -> serde_json::Value {
    let start = Instant::now();
    writeln!(
        process.child.stdin.as_mut().unwrap(),
        "{}",
        serde_json::to_string(&request).unwrap()
    )
    .unwrap();
    process.child.stdin.as_mut().unwrap().flush().unwrap();
    let mut line = String::new();
    assert!(reader.read_line(&mut line).unwrap() > 0);
    let response: Response = serde_json::from_str(&line).unwrap();
    assert_eq!(response.request_id, request.request_id);
    assert!(
        start.elapsed() < Duration::from_millis(request.time_limit_ms + 700),
        "deadline {:?}",
        start.elapsed()
    );
    response.answer
}
#[test]
fn real_cluster_reconnects_and_keeps_adopted_state() {
    let root = std::env::temp_dir().join(format!(
        "procon37-dist-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    fs::create_dir_all(&root).unwrap();
    let token = root.join("token");
    fs::write(
        &token,
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
    .unwrap();
    fs::set_permissions(&token, fs::Permissions::from_mode(0o600)).unwrap();
    let port_a = port();
    let mut port_b = port();
    while port_b == port_a {
        port_b = port();
    }
    let mut a = worker(&root, "worker-a", &token, port_a, 1);
    let b = worker(&root, "worker-b", &token, port_b, 3);
    // Authentication failure closes only this connection; the daemon stays alive.
    let mut bad = TcpStream::connect(("127.0.0.1", port_a)).unwrap();
    bad.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let message =
        serde_json::to_vec(&serde_json::json!({"Authenticate":{"version":1,"token":"wrong"}}))
            .unwrap();
    bad.write_all(&(message.len() as u32).to_be_bytes())
        .unwrap();
    bad.write_all(&message).unwrap();
    let mut byte = [0];
    assert_eq!(bad.read(&mut byte).unwrap(), 0);
    drop(bad);
    let mut coordinator = Process::start(
        &root,
        "coordinator",
        &token,
        &[
            "--mode".into(),
            "coordinator".into(),
            "--threads".into(),
            "2".into(),
            "--peers".into(),
            format!("127.0.0.1:{port_a}/3/1,127.0.0.1:{port_b}/3/3"),
        ],
    );
    let mut reader = BufReader::new(coordinator.child.stdout.take().unwrap());
    let problem = tester::generator::generate(0, None).unwrap().problem;
    // CPU1 validation serializes five eager-Graph search workers; keep enough request time.
    let request = Request {
        protocol_version: 1,
        request_id: 1,
        phase: "select".into(),
        time_limit_ms: 1800,
        problem: Some(problem.clone()),
        state: None,
        steps: None,
        previous_accepted: None,
    };
    let kinds = serde_json::from_value::<Vec<u8>>(exchange(&mut coordinator, &mut reader, request))
        .unwrap();
    let mut game = Game::new(problem.clone(), &kinds);
    // Confirm the configured thread cap counts the main thread on each process.
    for (process, threads) in [(&coordinator, 2), (&a, 3), (&b, 3)] {
        assert_eq!(
            fs::read_dir(format!("/proc/{}/task", process.child.id()))
                .unwrap()
                .count(),
            threads
        );
    }
    let mut previous_accepted = true;
    for day in 0..problem.day_steps.len() {
        if day == 1 {
            a.stop();
            // One complete request while peer A is absent, followed by restart.
        }
        if day == 2 {
            a = worker(&root, "worker-a-restarted", &token, port_a, 1);
        }
        let request = Request {
            protocol_version: 1,
            request_id: 2 + day as u64,
            phase: "plan".into(),
            time_limit_ms: 1800,
            problem: None,
            state: Some(game.observation(0)),
            steps: Some(problem.day_steps[day]),
            previous_accepted: Some(previous_accepted),
        };
        let plan: Plan =
            serde_json::from_value(exchange(&mut coordinator, &mut reader, request)).unwrap();
        let mut check = game.clone();
        check.play_day(plan.clone()).unwrap();
        previous_accepted = day != 0;
        if previous_accepted {
            game.play_day(plan).unwrap();
        } else {
            game.play_day(game.waiting_plan()).unwrap();
        }
    }
    coordinator.stop();
    a.stop();
    drop(b);
    fs::remove_file(token).unwrap();
    let log = fs::read_to_string(root.join("coordinator.log")).unwrap();
    assert!(
        log.contains("remote_results=2/2"),
        "no complete remote result: {log}"
    );
    assert!(
        log.contains("remote_results=1/2"),
        "missing degraded result: {log}"
    );
    assert!(!log.contains("rejected_configuration"));
    assert!(!log.contains("rejected_worker_id"));
    assert!(!log.contains("ignored_stale_or_mismatched_result"));
    let restarted = fs::read_to_string(root.join("worker-a-restarted.log")).unwrap();
    assert!(restarted.contains("phase=plan"));
    assert!(!restarted.contains("request_rejected"));
    eprintln!("process records and logs: {}", root.display());
}

#[test]
fn comparison_mode_uses_sixteen_total_threads() {
    let root =
        std::env::temp_dir().join(format!("procon37-parallel16-test-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let mut process = Process::start(
        &root,
        "parallel16",
        &root.join("unused-token"),
        &[
            "--mode".into(),
            "parallel".into(),
            "--threads".into(),
            "16".into(),
            "--seed".into(),
            "0".into(),
        ],
    );
    let mut reader = BufReader::new(process.child.stdout.take().unwrap());
    let p = tester::generator::generate(0, None).unwrap().problem;
    let kinds: Vec<u8> = serde_json::from_value(exchange(
        &mut process,
        &mut reader,
        Request {
            protocol_version: 1,
            request_id: 1,
            phase: "select".into(),
            time_limit_ms: 100,
            problem: Some(p.clone()),
            state: None,
            steps: None,
            previous_accepted: None,
        },
    ))
    .unwrap();
    assert_eq!(
        fs::read_dir(format!("/proc/{}/task", process.child.id()))
            .unwrap()
            .count(),
        16
    );
    let mut game = Game::new(p.clone(), &kinds);
    let plan: Plan = serde_json::from_value(exchange(
        &mut process,
        &mut reader,
        Request {
            protocol_version: 1,
            request_id: 2,
            phase: "plan".into(),
            time_limit_ms: 100,
            problem: None,
            state: Some(game.observation(0)),
            steps: Some(p.day_steps[0]),
            previous_accepted: Some(true),
        },
    ))
    .unwrap();
    game.play_day(plan).unwrap();
    process.stop();
    eprintln!("comparison process record: {}", root.display());
}
