use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    process::{Command, Output},
};
use tester::{
    model::MatchFormat,
    runner::{CommandSpec, RunConfig},
};

fn cli(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tester"))
        .args(args)
        .current_dir(root)
        .output()
        .unwrap()
}
fn config(root: &Path) -> RunConfig {
    RunConfig {
        count: Some(3),
        jobs: Some(3),
        build: None,
        label: "CLIの確認".into(),
        solver: CommandSpec {
            program: "python3".into(),
            args: vec![
                "-u".into(),
                "-c".into(),
                include_str!("fixtures/solver.py").into(),
                "archive".into(),
            ],
            cwd: root.into(),
            env: BTreeMap::new(),
        },
        ..RunConfig::default()
    }
}
fn save_config(root: &Path, config: &RunConfig) {
    fs::write(
        root.join("config.json"),
        serde_json::to_vec(config).unwrap(),
    )
    .unwrap();
}

#[test]
fn cli_run_and_compare_work_with_saved_and_imported_runs() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    save_config(root, &config(root));
    let run = cli(root, &["run", "--config", "config.json"]);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8(run.stdout).unwrap();
    for label in [
        "スコア合計・判定件数",
        "全体",
        "1回戦",
        "準決勝",
        "決勝",
        "CE=0",
    ] {
        assert!(stdout.contains(label), "{stdout}");
    }
    let runs = root.join(".tester/runs");
    let source = fs::read_dir(&runs)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name()
        .into_string()
        .unwrap();
    for (seed, sample) in [
        include_str!("../../../official/sample_maps/sample_16.json"),
        include_str!("../../../official/sample_maps/sample_24.json"),
        include_str!("../../../official/sample_maps/sample_32.json"),
    ]
    .iter()
    .enumerate()
    {
        let saved = fs::read(
            runs.join(&source)
                .join(seed.to_string())
                .join("problem.json"),
        )
        .unwrap();
        let problem: serde_json::Value = serde_json::from_slice(&saved).unwrap();
        let mut expected: serde_json::Value = serde_json::from_str(sample).unwrap();
        expected["players"] = 1.into();
        assert_eq!(problem, expected);
        let generated = cli(
            root,
            &[
                "generate",
                "--seed",
                &seed.to_string(),
                "--format",
                ["first", "semifinal", "final"][seed],
            ],
        );
        assert!(generated.status.success());
        let explicit: serde_json::Value = serde_json::from_slice(&generated.stdout).unwrap();
        assert_eq!(explicit, problem);
    }
    assert!(
        cli(root, &["export", &source, "--output", "run.zip"])
            .status
            .success()
    );
    assert!(
        cli(root, &["import", "run.zip", "--label", "別端末"])
            .status
            .success()
    );
    let imported = fs::read_dir(&runs)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .find(|id| id != &source)
        .unwrap();
    let compared = cli(root, &["compare", &source, &imported, "--json"]);
    assert!(compared.status.success());
    let json: serde_json::Value = serde_json::from_slice(&compared.stdout).unwrap();
    assert_eq!(json["baseline"], source);
    assert_eq!(json["scopes"].as_array().unwrap().len(), 4);
    assert_eq!(json["scopes"][0]["accepted"], 3);
    assert_eq!(json["scopes"][0]["results"][1]["outcomes"]["draws"], 3);
    assert_eq!(json["runs"][1]["importedFrom"], source);
    let filtered = cli(
        root,
        &[
            "compare",
            &source,
            &imported,
            "--baseline",
            &imported,
            "--format",
            "final",
        ],
    );
    assert!(filtered.status.success());
    let text = String::from_utf8(filtered.stdout).unwrap();
    assert!(text.contains("決勝: 1seed"));
    assert!(!text.contains("1回戦:"));
    assert!(text.contains("玉数合計"));
    for args in [
        vec!["compare", &source],
        vec!["compare", &source, &source],
        vec!["compare", &source, &imported, "--baseline", "0-0"],
        vec!["compare", &source, &imported, "--format"],
        vec!["compare", &source, &imported, "--format", "invalid"],
    ] {
        assert!(!cli(root, &args).status.success(), "{args:?}");
    }
}

#[test]
fn cli_prints_summary_before_returning_nonzero_for_re_and_ce() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mut cfg = config(root);
    cfg.count = Some(1);
    cfg.format = Some(MatchFormat::First);
    cfg.solver.program = root.join("missing-solver").to_string_lossy().into_owned();
    save_config(root, &cfg);
    let run = cli(root, &["run", "--config", "config.json"]);
    assert!(!run.status.success());
    let text = String::from_utf8(run.stdout).unwrap();
    assert!(text.contains("スコア合計・判定件数"));
    assert!(text.contains("完了 1 / 1件"));
    cfg.build = Some(cfg.solver.clone());
    save_config(root, &cfg);
    let run = cli(root, &["run", "--config", "config.json"]);
    assert!(!run.status.success());
    let text = String::from_utf8(run.stdout).unwrap();
    assert!(text.contains("CE=1"));
    assert!(text.contains("完了 0 / 1件"));
    assert!(text.contains("玉数"));
}

#[cfg(unix)]
#[test]
fn cli_interrupt_prints_the_saved_partial_summary() {
    use std::{
        process::Stdio,
        time::{Duration, Instant},
    };
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mut cfg = config(root);
    cfg.count = Some(1);
    *cfg.solver.args.last_mut().unwrap() = "cancel".into();
    save_config(root, &cfg);
    let mut child = Command::new(env!("CARGO_BIN_EXE_tester"))
        .args(["run", "--config", "config.json"])
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !root.join("started.json").exists() {
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("solver did not start");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    unsafe {
        assert_eq!(libc::kill(child.id() as i32, libc::SIGINT), 0);
    }
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("状態: 中断 / 完了 0 / 1件"), "{text}");
    assert!(text.contains("スコア合計・判定件数"));
    let path = fs::read_dir(root.join(".tester/runs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(path.join("run.json")).unwrap()).unwrap();
    assert_eq!(record["status"], "cancelled");
}
