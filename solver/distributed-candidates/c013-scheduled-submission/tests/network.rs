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
        serde_json::to_vec(&serde_json::json!({"Authenticate":{"version":4,"token":"wrong","cross_elite":false}}))
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
    // Five search workers share CPU1 in local validation; allow eager D0 graph construction.
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


/// Each process has one donor and one isolated receiver (7 / 35). This exercises optional traffic
/// while the normal Result/state protocol remains the sole submission path.
#[test]
fn cross_elite_enabled_cluster_returns_valid_results_and_stops() {
    let root = std::env::temp_dir().join(format!("procon37-cross-test-{}-{}",std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    fs::create_dir_all(&root).unwrap();
    let token = root.join("token");
    fs::write(&token,"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef").unwrap();
    fs::set_permissions(&token,fs::Permissions::from_mode(0o600)).unwrap();
    let worker_port = port();
    let mut remote = Process::start(&root,"worker",&token,&[
        "--mode".into(),"worker".into(),"--threads".into(),"3".into(),
        "--worker-offset".into(),"34".into(),"--listen".into(),format!("127.0.0.1:{worker_port}"),
        "--cross-elite".into(),"on".into()]);
    wait_listen(worker_port);
    let mut coordinator = Process::start(&root,"coordinator",&token,&[
        "--mode".into(),"coordinator".into(),"--threads".into(),"3".into(),
        "--worker-offset".into(),"6".into(),"--peers".into(),format!("127.0.0.1:{worker_port}/3/34"),
        "--cross-elite".into(),"on".into()]);
    let mut reader = BufReader::new(coordinator.child.stdout.take().unwrap());
    let p = tester::generator::generate(0,Some(tester::model::MatchFormat::First)).unwrap().problem;
    let kinds = exchange(&mut coordinator,&mut reader,Request {
        protocol_version:1,request_id:1,phase:"select".into(),time_limit_ms:350,
        problem:Some(p.clone()),state:None,steps:None,previous_accepted:None,
    });
    let kinds:Vec<u8> = serde_json::from_value(kinds).unwrap();
    let mut game = Game::new(p.clone(),&kinds);
    let answer = exchange(&mut coordinator,&mut reader,Request {
        protocol_version:1,request_id:2,phase:"plan".into(),time_limit_ms:5000,
        problem:None,state:Some(game.observation(0)),steps:Some(p.day_steps[0]),previous_accepted:Some(true),
    });
    game.play_day(serde_json::from_value::<Plan>(answer).unwrap()).unwrap();
    coordinator.stop(); remote.stop();
    let coordinator_log = fs::read_to_string(root.join("coordinator.log")).unwrap();
    let worker_log = fs::read_to_string(root.join("worker.log")).unwrap();
    assert!(coordinator_log.contains("remote_results=1/1"),"{coordinator_log}");
    assert!(coordinator_log.contains("cross_elite request=2"),"{coordinator_log}");
    assert!(worker_log.contains("cross_elite request=2"),"{worker_log}");
    let count = |log: &str, name: &str| -> u64 {
        log.lines().filter(|line| line.starts_with("cross_elite request=2 "))
            .flat_map(|line| line.split_whitespace()).filter_map(|field|
                field.strip_prefix(&format!("{name}=")).and_then(|v| v.parse::<u64>().ok())).sum()
    };
    assert!(count(&coordinator_log,"sent") + count(&worker_log,"sent") > 0,
        "on-mode must send actual hints: {coordinator_log} {worker_log}");
    assert!(count(&coordinator_log,"received") + count(&worker_log,"received") > 0,
        "on-mode must receive actual hints: {coordinator_log} {worker_log}");
    assert!(coordinator_log.contains("receiver=Some(7)"),"{coordinator_log}");
    assert!(worker_log.contains("receiver=Some(35)"),"{worker_log}");
    // A timing-sensitive amount of improvement is not asserted. The trace records
    // imported/accepted hints for inspection; improvement can legitimately be zero.
    eprintln!("cross_network_records={}",root.display());
}

#[test]
fn old_versions_and_mixed_feature_flags_are_rejected_without_stopping_worker() {
    let root=std::env::temp_dir().join(format!("procon37-c012-auth-{}",std::process::id()));
    fs::create_dir_all(&root).unwrap();let token=root.join("token");
    let secret="0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    fs::write(&token,secret).unwrap();fs::set_permissions(&token,fs::Permissions::from_mode(0o600)).unwrap();
    let listen=port();
    let mut remote=Process::start(&root,"worker",&token,&[
        "--mode".into(),"worker".into(),"--threads".into(),"3".into(),
        "--worker-offset".into(),"34".into(),"--listen".into(),format!("127.0.0.1:{listen}"),
        "--cross-elite".into(),"on".into()]);
    wait_listen(listen);
    for (version,enabled) in [(1,true),(3,true),(4,false)] {
        let mut stream=TcpStream::connect(("127.0.0.1",listen)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let bytes=serde_json::to_vec(&serde_json::json!({"Authenticate":{"version":version,"token":secret,"cross_elite":enabled}})).unwrap();
        stream.write_all(&(bytes.len() as u32).to_be_bytes()).unwrap();stream.write_all(&bytes).unwrap();
        let mut one=[0];assert_eq!(stream.read(&mut one).unwrap(),0);
    }
    let mut stream=TcpStream::connect(("127.0.0.1",listen)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let bytes=serde_json::to_vec(&serde_json::json!({"Authenticate":{"version":4,"token":secret,"cross_elite":true}})).unwrap();
    stream.write_all(&(bytes.len() as u32).to_be_bytes()).unwrap();stream.write_all(&bytes).unwrap();
    let mut header=[0;4];stream.read_exact(&mut header).unwrap();
    let length=u32::from_be_bytes(header) as usize;assert!(length<1024);
    let mut body=vec![0;length];stream.read_exact(&mut body).unwrap();
    let ready:serde_json::Value=serde_json::from_slice(&body).unwrap();
    assert_eq!(ready["Ready"]["version"],4);assert_eq!(ready["Ready"]["cross_elite"],true);
    drop(stream);remote.stop();fs::remove_file(token).unwrap();
    let log=fs::read_to_string(root.join("worker.log")).unwrap();
    assert_eq!(log.matches("authentication_rejected").count(),3);
    eprintln!("c012_auth_records={}",root.display());
}

#[test]
fn non_reserve15_donors_reach_receiver_through_real_pool_and_tcp() {
    let root=std::env::temp_dir().join(format!("procon37-c012-group0-{}-{}",std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    fs::create_dir_all(&root).unwrap();let token=root.join("token");
    fs::write(&token,"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef").unwrap();
    fs::set_permissions(&token,fs::Permissions::from_mode(0o600)).unwrap();
    let listen=port();
    let mut remote=Process::start(&root,"worker",&token,&[
        "--mode".into(),"worker".into(),"--threads".into(),"3".into(),
        "--worker-offset".into(),"34".into(),"--listen".into(),format!("127.0.0.1:{listen}"),
        "--cross-elite".into(),"on".into()]);
    wait_listen(listen);
    // Both local workers (0,1) are reserve4 donors; there is no local receiver.
    let mut coordinator=Process::start(&root,"coordinator",&token,&[
        "--mode".into(),"coordinator".into(),"--threads".into(),"3".into(),
        "--worker-offset".into(),"0".into(),"--peers".into(),format!("127.0.0.1:{listen}/3/34"),
        "--cross-elite".into(),"on".into()]);
    let mut reader=BufReader::new(coordinator.child.stdout.take().unwrap());
    let p=tester::generator::generate(0,Some(tester::model::MatchFormat::First)).unwrap().problem;
    let kinds:Vec<u8>=serde_json::from_value(exchange(&mut coordinator,&mut reader,Request{
        protocol_version:1,request_id:1,phase:"select".into(),time_limit_ms:350,
        problem:Some(p.clone()),state:None,steps:None,previous_accepted:None,
    })).unwrap();
    let mut game=Game::new(p.clone(),&kinds);
    let answer=exchange(&mut coordinator,&mut reader,Request{
        protocol_version:1,request_id:2,phase:"plan".into(),time_limit_ms:5000,
        problem:None,state:Some(game.observation(0)),steps:Some(p.day_steps[0]),previous_accepted:Some(true),
    });
    game.play_day(serde_json::from_value::<Plan>(answer).unwrap()).unwrap();
    coordinator.stop();remote.stop();fs::remove_file(token).unwrap();
    let local=fs::read_to_string(root.join("coordinator.log")).unwrap();
    let foreign=fs::read_to_string(root.join("worker.log")).unwrap();
    let group_count=|log:&str,key:&str,group:usize|->u64 {
        log.lines().filter(|line|line.starts_with("cross_elite request=2 "))
            .filter_map(|line|line.split_once(&format!("{key}=[")))
            .filter_map(|(_,rest)|rest.split_once(']'))
            .filter_map(|(values,_)|values.split(',').nth(group)?.trim().parse::<u64>().ok()).sum()
    };
    assert!(local.contains("receiver=None"),"{local}");
    assert!(foreign.contains("receiver=Some(35)"),"{foreign}");
    assert!(local.contains("remote_results=1/1"),"{local}");
    assert!(group_count(&local,"group_published",0)>0,"no group0 donor publication: {local}");
    assert!(group_count(&foreign,"group_received",0)>0,"group0 never reached remote receiver bus: {foreign}");
    // Stagnation/import success is not forced: this proves dispatch+publish+TCP,
    // not a timing-sensitive amount of search or game improvement.
    eprintln!("c012_group0_network_records={}",root.display());
}

#[test]
fn streaming_restores_earlier_accepted_answer_across_days() {
    let root = std::env::temp_dir().join(format!("procon37-c013-stream-{}",std::process::id()));
    fs::create_dir_all(&root).unwrap(); let token = root.join("token");
    fs::write(&token,"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef").unwrap();
    let listen = port();
    let mut remote = worker(&root,"remote",&token,listen,1);
    let mut coordinator = Process::start(&root,"coordinator",&token,&[
        "--mode".into(),"coordinator".into(),"--threads".into(),"2".into(),
        "--peers".into(),format!("127.0.0.1:{listen}/3/1"),"--stream-responses".into()]);
    let mut reader = BufReader::new(coordinator.child.stdout.take().unwrap());
    let p = tester::generator::generate(0,Some(tester::model::MatchFormat::First)).unwrap().problem;
    let mut responses = |request: serde_json::Value| -> Vec<serde_json::Value> {
        writeln!(coordinator.child.stdin.as_mut().unwrap(),"{}",request).unwrap();
        coordinator.child.stdin.as_mut().unwrap().flush().unwrap();
        let start = Instant::now(); let mut out = Vec::new();
        loop {
            let mut line = String::new(); assert!(reader.read_line(&mut line).unwrap() > 0);
            let value: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(value["requestId"],request["requestId"]);
            let final_result = value["final"].as_bool().unwrap();
            out.push(value);
            if final_result { break; }
            assert!(start.elapsed() < Duration::from_secs(5));
        }
        assert_eq!(out[0]["final"],false);
        assert_eq!(out.last().unwrap()["final"],true);
        out
    };
    let selected = responses(serde_json::json!({"protocolVersion":1,"requestId":1,"phase":"select",
        "timeLimitMs":1800,"problem":p}));
    // Accept the immediate fallback, not the final selection. The next request
    // must restore this exact supplier and cache state.
    let mut accepted = selected[0]["answer"].clone();
    let kinds: Vec<u8> = serde_json::from_value(accepted.clone()).unwrap();
    let mut game = Game::new(p.clone(),&kinds);
    for day in 0..2 {
        let plans = responses(serde_json::json!({"protocolVersion":1,"requestId":day+2,"phase":"plan",
            "timeLimitMs":2500,"state":game.observation(0),"steps":p.day_steps[day],
            "previousAccepted":true,"previousAnswer":accepted}));
        for response in &plans {
            game.clone().play_day(serde_json::from_value(response["answer"].clone()).unwrap()).unwrap();
        }
        // Simulate accepting an early submission then rejecting its final update.
        accepted = plans[0]["answer"].clone();
        game.play_day(serde_json::from_value(accepted.clone()).unwrap()).unwrap();
    }
    // No solver answer was accepted: null means use previousAccepted=false to
    // synchronize the waiting outcome, not try to find a JSON null snapshot.
    let plans = responses(serde_json::json!({"protocolVersion":1,"requestId":4,"phase":"plan",
        "timeLimitMs":700,"state":game.observation(0),"steps":p.day_steps[2],
        "previousAccepted":false,"previousAnswer":null}));
    for response in plans { game.clone().play_day(serde_json::from_value(response["answer"].clone()).unwrap()).unwrap(); }
    // A server-selected all-collector fallback must also be handled after a
    // wholly rejected selection, without inventing an emitted previousAnswer.
    let _ = responses(serde_json::json!({"protocolVersion":1,"requestId":5,"phase":"select",
        "timeLimitMs":300,"problem":p}));
    let fallback_game = Game::new(p.clone(),&vec![0;p.agents.len()]);
    let plans = responses(serde_json::json!({"protocolVersion":1,"requestId":6,"phase":"plan",
        "timeLimitMs":700,"state":fallback_game.observation(0),"steps":p.day_steps[0],
        "previousAccepted":false,"previousAnswer":null}));
    for response in plans { fallback_game.clone().play_day(serde_json::from_value(response["answer"].clone()).unwrap()).unwrap(); }
    drop(responses); coordinator.stop(); remote.stop();
    let log = fs::read_to_string(root.join("coordinator.log")).unwrap();
    assert_eq!(log.matches("restored_previous_answer").count(),2,"{log}");
    assert!(log.contains("remote_results=1/1"),"{log}");
    assert!(!log.contains("fatal"),"{log}");
    fs::remove_file(token).unwrap();
}
