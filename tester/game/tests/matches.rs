use procon37_common::{
    archive, comparison, competition,
    engine::{Game, traffic_for},
    generator::{generate, generate_for_players},
    model::*,
    runner::*,
};
use std::{collections::BTreeMap, path::Path, sync::Arc};

fn config(root: &Path) -> RunConfig {
    let mut c = RunConfig::for_mode(Mode::Game);
    c.count = Some(1);
    c.jobs = Some(1);
    c.build = None;
    c.time_limit_ms = Some(2000);
    for id in 0..5 {
        let solver = CommandSpec {
            program: "python3".into(),
            args: vec![
                "-u".into(),
                "-c".into(),
                include_str!("fixture.py").into(),
                "barrier".into(),
            ],
            cwd: root.into(),
            env: BTreeMap::from([("PLAYER_ID".into(), id.to_string())]),
        };
        if id == 0 {
            c.solver = solver;
        } else {
            c.opponents[id - 1].solver = solver;
            c.opponents[id - 1].build = None;
        }
    }
    c
}
async fn run(root: &Path, c: RunConfig) -> Manifest {
    let (_tx, rx) = tokio::sync::watch::channel(false);
    run_batch(run_id(), c, root.into(), Arc::new(|_| {}), rx)
        .await
        .unwrap()
}
#[test]
fn maps_differ_only_in_player_count_and_ranking_ignores_time() {
    for seed in 0..60 {
        for format in [
            None,
            Some(MatchFormat::First),
            Some(MatchFormat::Semifinal),
            Some(MatchFormat::Final),
        ] {
            let solo = generate(seed, format).unwrap();
            let mut game = generate_for_players(seed, format, 5).unwrap();
            assert_eq!(game.problem.players, 5);
            game.problem.players = 1;
            assert_eq!(solo, game);
        }
    }
    let s = |a, b, c| Score {
        unique_brands: a,
        daily_brands: b,
        udon: c,
    };
    let scores = [
        s(3, 1, 1),
        s(2, 99, 99),
        s(3, 1, 1),
        s(2, 99, 100),
        s(2, 98, 999),
    ];
    assert_eq!(
        (0..5)
            .map(|i| competition::rank(&scores, i))
            .collect::<Vec<_>>(),
        vec![1, 4, 1, 3, 5]
    );
    let c = RunConfig::for_mode(Mode::Game);
    assert_eq!(c.execution(32).jobs, 6);
    assert_eq!(c.execution(2).jobs, 1);
    let mut c = c;
    c.opponents[2].solver_parallel = true;
    assert_eq!(c.execution(32).jobs, 1);
}
fn problem() -> Problem {
    Problem {
        starts_at: 0,
        day_seconds: vec![60; 3],
        day_steps: vec![2; 3],
        map: Map {
            width: 3,
            height: 1,
            cells: vec![vec![0, 1, 0]],
        },
        spots: vec![Spot {
            brand: 0,
            pos: 2,
            stocks: 1,
        }],
        agents: vec![0, 2],
        fuel_limits: 5,
        players: 5,
        busy_threshold: 2,
        jammed_threshold: 4,
    }
}
#[test]
fn teams_keep_stock_and_supply_separate_and_share_two_day_traffic() {
    let p = problem();
    let mut teams: Vec<_> = (0..5).map(|_| Game::new(p.clone(), &[0, 0])).collect();
    for t in &mut teams {
        t.agents[0].pos = 2;
        t.play_day_with_traffic(vec![vec![-2]; 2], vec![Traffic { pos: 1, status: 0 }])
            .unwrap();
        assert_eq!(t.score.udon, 1);
    }
    // A supply vehicle in another team cannot refuel this team's patrol.
    teams[0].agents[0].fuel = 0;
    teams[1].agents[0].kind = 1;
    teams[0]
        .play_day_with_traffic(vec![vec![-2]; 2], vec![Traffic { pos: 1, status: 0 }])
        .unwrap();
    assert_eq!(teams[0].agents[0].fuel, 0);
    for (count, status) in [(9, 0), (10, 1), (19, 1), (20, 2)] {
        assert_eq!(traffic_for(&p, &[vec![0, count, 0]], 5)[0].status, status);
    }
    assert_eq!(
        traffic_for(&p, &[vec![0, 100, 0], vec![0, 4, 0], vec![0, 5, 0]], 5)[0].status,
        0
    );
    assert_eq!(
        traffic_for(&p, &[vec![0, 6, 0], vec![0, 5, 0]], 5)[0].status,
        1
    );
}
#[tokio::test]
async fn five_requests_are_parallel_and_others_exclude_self() {
    let temp = tempfile::tempdir().unwrap();
    let m = run(temp.path(), config(temp.path())).await;
    assert_eq!(m.status, "completed");
    assert_eq!(m.cases[0].status, "AC");
    assert_eq!(m.cases[0].players.len(), 5);
    assert!(m.cases[0].players.iter().all(|p| p.rank == 1));
    let r: Replay = serde_json::from_slice(
        &std::fs::read(temp.path().join(&m.id).join("0/replay.json")).unwrap(),
    )
    .unwrap();
    competition::validate_replay(
        &r,
        &m.cases[0],
        &m.cases[0]
            .players
            .iter()
            .map(|p| p.name.clone())
            .collect::<Vec<_>>(),
    )
    .unwrap();
}
#[tokio::test]
async fn invalid_or_exited_opponent_does_not_stop_other_players() {
    for (mode, status) in [("invalid", "WA"), ("exit", "RE"), ("timeout", "TLE")] {
        let temp = tempfile::tempdir().unwrap();
        let mut c = config(temp.path());
        // No barrier in fault cases: one participant intentionally does not respond.
        *c.solver.args.last_mut().unwrap() = "valid".into();
        for p in &mut c.opponents {
            *p.solver.args.last_mut().unwrap() = "valid".into();
        }
        *c.opponents[1].solver.args.last_mut().unwrap() = mode.into();
        if mode == "timeout" {
            c.time_limit_ms = Some(150);
        }
        let m = run(temp.path(), c).await;
        assert_eq!(m.status, "completed");
        assert_eq!(m.cases[0].players[2].status, status);
        assert!(
            m.cases[0]
                .players
                .iter()
                .filter(|p| p.id != 2)
                .all(|p| p.status == "AC"),
            "{:?}",
            m.cases[0].players
        );
        let zip = archive::export_run(temp.path(), &m.id).await.unwrap();
        archive::import_run_for_mode(temp.path(), zip, None, Mode::Game).unwrap();
    }
}
#[tokio::test]
async fn archives_and_comparison_preserve_match_identity_and_all_five_results() {
    let temp = tempfile::tempdir().unwrap();
    let m = run(temp.path(), config(temp.path())).await;
    let zip = archive::export_run(temp.path(), &m.id).await.unwrap();
    assert!(archive::import_run(temp.path(), zip, None).is_err());
    let zip = archive::export_run(temp.path(), &m.id).await.unwrap();
    let imported = archive::import_run_for_mode(temp.path(), zip, None, Mode::Game).unwrap();
    let comparison = comparison::compare(temp.path(), &[m.id.clone(), imported.id.clone()])
        .await
        .unwrap();
    assert_eq!(comparison.rows[0].alignment, "matched");
    assert_eq!(m.cases[0].players, imported.cases[0].players);
    assert_ne!(
        m.config.opponents[0].solver.program,
        imported.config.opponents[0].solver.program
    );
    let path = temp.path().join(&imported.id).join("run.json");
    let mut altered = imported;
    altered.opponent_identities[0].push('x');
    std::fs::write(path, serde_json::to_vec(&altered).unwrap()).unwrap();
    assert_eq!(
        comparison::compare(temp.path(), &[m.id, altered.id])
            .await
            .unwrap()
            .rows[0]
            .alignment,
        "different"
    );
}
#[tokio::test]
async fn comparison_matches_built_and_no_build_runs() {
    let temp = tempfile::tempdir().unwrap();
    let mut c = config(temp.path());
    c.label = "比較識別・ビルドあり・回帰確認1件".into();
    *c.solver.args.last_mut().unwrap() = "valid".into();
    let build = CommandSpec {
        program: "python3".into(),
        args: vec!["-c".into(), "open('built', 'w').write('ok')".into()],
        cwd: temp.path().into(),
        env: BTreeMap::new(),
    };
    for opponent in &mut c.opponents {
        *opponent.solver.args.last_mut().unwrap() = "valid".into();
        opponent.build = Some(build.clone());
    }
    let built = run(temp.path(), c.clone()).await;
    assert!(temp.path().join("built").is_file());
    assert_eq!(built.cases[0].status, "AC");

    // The CLI's --no-build option clears only the build recipes.
    c.label = "比較識別・ビルド省略・回帰確認1件".into();
    c.build = None;
    for opponent in &mut c.opponents {
        opponent.build = None;
    }
    let skipped = run(temp.path(), c).await;
    assert_eq!(skipped.cases[0].status, "AC");
    let compared = comparison::compare(temp.path(), &[built.id, skipped.id])
        .await
        .unwrap();
    assert_eq!(compared.rows.len(), 1);
    assert_eq!(compared.rows[0].alignment, "matched");
}

#[tokio::test]
async fn replay_validation_rejects_opponent_and_shared_traffic_tampering() {
    let temp = tempfile::tempdir().unwrap();
    let m = run(temp.path(), config(temp.path())).await;
    let path = temp.path().join(&m.id).join("0/replay.json");
    let r: Replay = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let names = m.cases[0]
        .players
        .iter()
        .map(|p| p.name.clone())
        .collect::<Vec<_>>();
    let mut changed = r.clone();
    changed.opponents[0].days[0].frames[1].agents[0].fuel += 1;
    assert!(competition::validate_replay(&changed, &m.cases[0], &names).is_err());
    let mut changed = r;
    changed.opponents[2].days[1].traffics[0].status = 2;
    assert!(competition::validate_replay(&changed, &m.cases[0], &names).is_err());
}
#[cfg(unix)]
#[tokio::test]
async fn cancellation_kills_all_five_process_groups() {
    let temp = tempfile::tempdir().unwrap();
    let mut c = config(temp.path());
    *c.solver.args.last_mut().unwrap() = "cancel".into();
    for p in &mut c.opponents {
        *p.solver.args.last_mut().unwrap() = "cancel".into();
    }
    let (tx, rx) = tokio::sync::watch::channel(false);
    let root = temp.path().to_owned();
    let task = tokio::spawn(run_batch(run_id(), c, root.clone(), Arc::new(|_| {}), rx));
    for _ in 0..500 {
        if (0..5).all(|id| root.join(format!("started-{id}.json")).exists()) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!((0..5).all(|id| root.join(format!("started-{id}.json")).exists()));
    tx.send(true).unwrap();
    assert_eq!(task.await.unwrap().unwrap().status, "cancelled");
    for id in 0..5 {
        let pids: Vec<u32> = serde_json::from_slice(
            &std::fs::read(root.join(format!("started-{id}.json"))).unwrap(),
        )
        .unwrap();
        for pid in pids {
            for attempt in 0..100 {
                let status = std::fs::read_to_string(format!("/proc/{pid}/stat"));
                if status.as_ref().is_err()
                    || status
                        .as_ref()
                        .is_ok_and(|s| s.split_whitespace().nth(2) == Some("Z"))
                {
                    break;
                }
                assert!(attempt < 99, "process {pid} survived cancellation");
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }
    }
}
