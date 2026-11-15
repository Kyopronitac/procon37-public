//! Lockstep matches: only the next day's traffic couples otherwise independent teams.
use crate::{
    engine::{Game, traffic_for, validate_kinds},
    generator::generate_for_players_with_arena_maps,
    model::*,
    process::SolverProcess,
    runner::*,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    cmp::Ordering,
    io::Read,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::task::JoinSet;

pub fn score_order(a: &Score, b: &Score) -> Ordering {
    (a.unique_brands, a.daily_brands, a.udon).cmp(&(b.unique_brands, b.daily_brands, b.udon))
}
pub fn rank(scores: &[Score], index: usize) -> usize {
    1 + scores
        .iter()
        .filter(|s| score_order(s, &scores[index]).is_gt())
        .count()
}

/// Content identity, not a security signature. Build recipes and scheduling hints are excluded.
pub fn identity(player: &PlayerConfig) -> Result<String> {
    let mut a = 0xcbf29ce484222325_u64;
    let mut b = 0x84222325cbf29ce4_u64;
    let mut feed = |bytes: &[u8]| {
        for &v in bytes {
            a = (a ^ u64::from(v)).wrapping_mul(0x100000001b3);
            b = (b ^ u64::from(v))
                .rotate_left(7)
                .wrapping_mul(0x100000001b3);
        }
    };
    feed(&serde_json::to_vec(&(
        &player.name,
        &player.version,
        &player.solver,
    ))?);
    let mut env: std::collections::BTreeMap<String, String> = std::env::vars()
        .filter(|(k, _)| {
            k.starts_with("PROCON37_")
                || matches!(k.as_str(), "PATH" | "OMP_NUM_THREADS" | "RAYON_NUM_THREADS")
        })
        .collect();
    env.extend(player.solver.env.clone());
    feed(&serde_json::to_vec(&env)?);
    let spec = &player.solver;
    let executable = if spec.program.contains('/') {
        Some(spec.cwd.join(&spec.program))
    } else {
        env.get("PATH").and_then(|path| {
            std::env::split_paths(path)
                .map(|p| spec.cwd.join(p).join(&spec.program))
                .find(|p| p.is_file())
        })
    };
    let mut paths: Vec<_> = executable.into_iter().collect();
    paths.extend(
        spec.args
            .iter()
            .map(|s| spec.cwd.join(s))
            .filter(|p| p.is_file()),
    );
    for path in paths {
        if let Ok(mut file) = std::fs::File::open(&path) {
            // Frame each file so moving bytes across file boundaries changes its identity.
            let path_bytes = serde_json::to_vec(&path)?;
            feed(&(path_bytes.len() as u64).to_le_bytes());
            feed(&path_bytes);
            feed(&file.metadata()?.len().to_le_bytes());
            let mut buffer = [0; 65536];
            loop {
                let n = file.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                feed(&buffer[..n]);
            }
        }
    }
    Ok(format!("content-v3:{a:016x}{b:016x}"))
}

struct Slot {
    process: Option<SolverProcess>,
    startup: Option<Failure>,
}
type Answer = (Option<Value>, Option<Failure>, u64);
async fn exchange_all(slots: &mut Vec<Slot>, requests: Vec<Request>) -> Result<Vec<Answer>> {
    let count = slots.len();
    let mut tasks = JoinSet::new();
    for (id, (mut slot, request)) in slots.drain(..).zip(requests).enumerate() {
        tasks.spawn(async move {
            let answer = if let Some(p) = &mut slot.process {
                p.exchange(&request).await
            } else {
                (None, slot.startup.clone(), 0)
            };
            (id, slot, answer)
        });
    }
    let mut ordered: Vec<Option<(Slot, Answer)>> = (0..count).map(|_| None).collect();
    while let Some(result) = tasks.join_next().await {
        let (id, slot, answer) = result?;
        ordered[id] = Some((slot, answer));
    }
    let mut answers = Vec::new();
    for entry in ordered {
        let (slot, answer) = entry.context("回答タスクが不足しています")?;
        slots.push(slot);
        answers.push(answer);
    }
    Ok(answers)
}

pub fn observations(games: &[Game], traffic: &[Traffic], ends_at: u64) -> Vec<Observation> {
    games
        .iter()
        .enumerate()
        .map(|(id, game)| Observation {
            ends_at,
            day: game.day,
            agents: game.agents.clone(),
            traffics: traffic.to_vec(),
            others: games
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != id)
                .map(|(other, g)| json!({"id":other,"agents":g.agents}))
                .collect(),
        })
        .collect()
}

pub async fn run_case(seed: u64, config: &RunConfig, directory: &Path) -> Result<CaseSummary> {
    tokio::fs::create_dir_all(directory).await?;
    let case = generate_for_players_with_arena_maps(seed, config.format, 5, config.arena_maps)?;
    write_json(&directory.join("problem.json"), &case.problem).await?;
    let specs: Vec<_> = std::iter::once(&config.solver)
        .chain(config.opponents.iter().map(|p| &p.solver))
        .collect();
    ensure!(specs.len() == 5, "対戦は5人固定です");
    let mut slots = Vec::new();
    for (id, spec) in specs.iter().enumerate() {
        let log = if id == 0 {
            "stderr.log".into()
        } else {
            format!("stderr-{id}.log")
        };
        match SolverProcess::spawn_named(spec, directory, &log).await {
            Ok(p) => slots.push(Slot {
                process: Some(p),
                startup: None,
            }),
            Err(e) => slots.push(Slot {
                process: None,
                startup: Some(Failure::new("process", format!("{e:#}"))),
            }),
        }
    }
    let request = Request {
        protocol_version: PROTOCOL_VERSION,
        request_id: 0,
        phase: "select".into(),
        time_limit_ms: config.time_limit_ms.unwrap_or(case.format.parameters().4),
        problem: Some(case.problem.clone()),
        state: None,
        steps: None,
        previous_accepted: None,
    };
    let answers = exchange_all(&mut slots, vec![request; 5]).await?;
    let mut games = Vec::new();
    let mut replays = Vec::new();
    let mut accepted = Vec::new();
    for (answer, mut failure, elapsed) in answers {
        let kinds = answer
            .as_ref()
            .and_then(|v| match validate_kinds(v, case.problem.agents.len()) {
                Ok(k) => Some(k),
                Err(e) => {
                    failure = Some(e);
                    None
                }
            })
            .unwrap_or_else(|| vec![0; case.problem.agents.len()]);
        games.push(Game::new(case.problem.clone(), &kinds));
        accepted.push(failure.is_none());
        replays.push(Replay {
            opponents: vec![],
            schema_version: REPLAY_VERSION,
            case: case.clone(),
            kinds,
            selection_answer: answer,
            selection_failure: failure,
            selection_ms: elapsed,
            days: vec![],
            score: Score::default(),
            answer_time_ms: 0,
        });
    }
    let mut history = Vec::new();
    for day_index in 0..case.problem.day_steps.len() {
        let limit = config
            .time_limit_ms
            .unwrap_or(u64::from(case.problem.day_seconds[day_index]) * 1000);
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let traffic = traffic_for(&case.problem, &history, 5);
        let requests = observations(&games, &traffic, now + limit.div_ceil(1000))
            .into_iter()
            .enumerate()
            .map(|(id, state)| Request {
                protocol_version: PROTOCOL_VERSION,
                request_id: (day_index + 1) as u64,
                phase: "plan".into(),
                time_limit_ms: limit,
                problem: None,
                state: Some(state),
                steps: Some(case.problem.day_steps[day_index]),
                previous_accepted: Some(accepted[id]),
            })
            .collect();
        let answers = exchange_all(&mut slots, requests).await?;
        for (id, (answer, mut failure, elapsed)) in answers.into_iter().enumerate() {
            let game = &mut games[id];
            let mut actual = None;
            if let Some(value) = &answer {
                match serde_json::from_value::<Plan>(value.clone())
                    .map_err(|e| Failure::new("invalid_plan", e.to_string()))
                    .and_then(|plan| game.play_day_with_traffic(plan, traffic.clone()))
                {
                    Ok(d) => actual = Some(d),
                    Err(e) => failure = Some(e),
                }
            }
            accepted[id] = failure.is_none();
            let mut day = if let Some(d) = actual {
                replays[id].answer_time_ms += elapsed;
                d
            } else {
                game.play_day_with_traffic(game.waiting_plan(), traffic.clone())
                    .map_err(|e| anyhow::anyhow!(e.message))?
            };
            day.submitted_answer = answer;
            day.failure = failure;
            day.elapsed_ms = elapsed;
            replays[id].days.push(day);
            replays[id].score = game.score.clone();
        }
        history.push(total_stays(&games, case.problem.map.len()));
        if history.len() > 2 {
            history.remove(0);
        }
    }
    for slot in &mut slots {
        if let Some(p) = &mut slot.process {
            p.finish().await;
        }
    }
    let names: Vec<_> = config.player_names();
    let players = summaries(&replays, &names);
    let status = combined_verdict(&players).into();
    let mut replay = replays.remove(0);
    replay.schema_version = 2;
    replay.opponents = replays;
    let summary = CaseSummary {
        seed: case.seed,
        format: case.format,
        status,
        score: replay.score.clone(),
        answer_time_ms: replay.answer_time_ms,
        warnings: players.iter().map(|p| p.warnings).sum(),
        error: None,
        players,
    };
    write_json(&directory.join("replay.json"), &replay).await?;
    Ok(summary)
}
pub fn total_stays(games: &[Game], count: usize) -> Vec<u32> {
    let mut stays = vec![0; count];
    for game in games {
        for (total, n) in stays.iter_mut().zip(game.last_stays()) {
            *total += n;
        }
    }
    stays
}
fn summaries(replays: &[Replay], names: &[String]) -> Vec<PlayerSummary> {
    let scores: Vec<_> = replays.iter().map(|r| r.score.clone()).collect();
    replays
        .iter()
        .enumerate()
        .map(|(id, r)| {
            let failures = r
                .selection_failure
                .iter()
                .chain(r.days.iter().filter_map(|d| d.failure.as_ref()));
            PlayerSummary {
                id,
                name: names[id].clone(),
                rank: rank(&scores, id),
                score: r.score.clone(),
                status: verdict(failures.clone()).into(),
                answer_time_ms: r.answer_time_ms,
                warnings: failures.count(),
            }
        })
        .collect()
}
fn combined_verdict(players: &[PlayerSummary]) -> &'static str {
    ["TLE", "RE", "WA"]
        .into_iter()
        .find(|v| players.iter().any(|p| p.status == *v))
        .unwrap_or("AC")
}

pub fn validate_replay(replay: &Replay, case: &CaseSummary, names: &[String]) -> Result<()> {
    use crate::archive::validate_answer;
    ensure!(
        replay.schema_version == 2 && replay.opponents.len() == 4,
        "5人用リプレイではありません"
    );
    replay.case.problem.validate_players(5)?;
    let replays: Vec<_> = std::iter::once(replay)
        .chain(replay.opponents.iter())
        .collect();
    let p = &replay.case.problem;
    let mut games = Vec::new();
    for (id, r) in replays.iter().enumerate() {
        ensure!(
            r.case == replay.case
                && r.days.len() == p.day_steps.len()
                && (id == 0 || (r.schema_version == 1 && r.opponents.is_empty())),
            "プレイヤーの問題・日数・形式が不正です"
        );
        let kinds = validate_answer(
            r.selection_answer.as_ref(),
            r.selection_failure.as_ref(),
            |a| validate_kinds(a, p.agents.len()),
        )?
        .unwrap_or_else(|| vec![0; p.agents.len()]);
        ensure!(kinds == r.kinds, "車種が一致しません");
        games.push(Game::new(p.clone(), &kinds));
    }
    let mut history = vec![];
    let mut times = [0_u64; 5];
    for d in 0..p.day_steps.len() {
        let traffic = traffic_for(p, &history, 5);
        for (id, game) in games.iter_mut().enumerate() {
            let day = &replays[id].days[d];
            ensure!(
                day.day == d + 1 && day.steps == p.day_steps[d],
                "日付・ステップが一致しません"
            );
            let accepted = validate_answer(
                day.submitted_answer.as_ref(),
                day.failure.as_ref(),
                |value| {
                    let plan = serde_json::from_value(value.clone())
                        .map_err(|e| Failure::new("invalid_plan", e.to_string()))?;
                    game.play_day_with_traffic(plan, traffic.clone())
                },
            )?;
            let actual = if let Some(actual) = accepted {
                times[id] = times[id]
                    .checked_add(day.elapsed_ms)
                    .context("回答時間が大きすぎます")?;
                actual
            } else {
                game.play_day_with_traffic(game.waiting_plan(), traffic.clone())
                    .map_err(|e| anyhow::anyhow!(e.message))?
            };
            ensure!(
                day.plan == actual.plan
                    && day.frames == actual.frames
                    && day.traffics == actual.traffics,
                "プレイヤー{id}の日{}の再計算が一致しません",
                d + 1
            );
        }
        history.push(total_stays(&games, p.map.len()));
    }
    for (id, r) in replays.iter().enumerate() {
        ensure!(
            r.score == games[id].score && r.answer_time_ms == times[id],
            "プレイヤーの得点・回答時間が不正です"
        );
    }
    let copies: Vec<_> = replays.into_iter().cloned().collect();
    let actual = summaries(&copies, names);
    ensure!(
        actual == case.players
            && case.status == combined_verdict(&actual)
            && case.warnings == actual.iter().map(|p| p.warnings).sum::<usize>()
            && case.score == replay.score
            && case.answer_time_ms == replay.answer_time_ms,
        "順位・集計・判定が不正です"
    );
    Ok(())
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn identity_distinguishes_redistributed_file_contents() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("opponent"), b"executable").unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        std::fs::write(&first, b"a").unwrap();
        std::fs::write(&second, b"bc").unwrap();
        let player = PlayerConfig {
            name: "fixture".into(),
            version: "v1".into(),
            solver: CommandSpec {
                program: "./opponent".into(),
                args: vec!["first".into(), "second".into()],
                cwd: temp.path().into(),
                env: BTreeMap::new(),
            },
            build: None,
            solver_parallel: false,
        };
        let original = identity(&player).unwrap();
        std::fs::write(&first, b"ab").unwrap();
        std::fs::write(&second, b"c").unwrap();
        assert_ne!(identity(&player).unwrap(), original);
        std::fs::write(&first, b"a").unwrap();
        std::fs::write(&second, b"bc").unwrap();
        assert_eq!(identity(&player).unwrap(), original);
    }

    #[test]
    fn identity_ignores_build_recipes_but_detects_runtime_and_content_changes() {
        let temp = tempfile::tempdir().unwrap();
        let executable = temp.path().join("opponent");
        let argument = temp.path().join("input.json");
        std::fs::write(&executable, b"executable-v1").unwrap();
        std::fs::write(&argument, b"input-v1").unwrap();
        let player = PlayerConfig {
            name: "fixture".into(),
            version: "v1".into(),
            solver: CommandSpec {
                program: "./opponent".into(),
                args: vec!["input.json".into()],
                cwd: temp.path().into(),
                env: BTreeMap::new(),
            },
            build: None,
            solver_parallel: false,
        };
        let expected = identity(&player).unwrap();
        let mut with_build = player.clone();
        with_build.build = Some(CommandSpec {
            program: "unused-build-recipe".into(),
            args: vec![],
            cwd: temp.path().into(),
            env: BTreeMap::new(),
        });
        with_build.solver_parallel = true;
        assert_eq!(identity(&with_build).unwrap(), expected);

        for field in ["program", "args", "cwd", "env", "version"] {
            let mut changed = player.clone();
            match field {
                "program" => changed.solver.program = "./another-opponent".into(),
                "args" => changed.solver.args.push("1".into()),
                "cwd" => changed.solver.cwd = temp.path().join("another-directory"),
                "env" => {
                    changed.solver.env.insert("SEED".into(), "1".into());
                }
                "version" => changed.version = "v2".into(),
                _ => unreachable!(),
            }
            assert_ne!(identity(&changed).unwrap(), expected, "{field}");
        }
        std::fs::write(&executable, b"executable-v2").unwrap();
        assert_ne!(identity(&player).unwrap(), expected);
        std::fs::write(&executable, b"executable-v1").unwrap();
        assert_eq!(identity(&player).unwrap(), expected);
        std::fs::write(&argument, b"input-v2").unwrap();
        assert_ne!(identity(&player).unwrap(), expected);
    }
}
