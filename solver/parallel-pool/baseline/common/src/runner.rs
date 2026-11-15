pub use crate::process::CommandSpec;
use crate::process::{SolverProcess, run_build};
use crate::{
    engine::{Game, validate_kinds},
    generator::{day_seconds, generate_with_arena_maps, resolve_format},
    model::*,
};
use anyhow::{Context, Result, anyhow, ensure};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::Value;
use std::{
    cmp::Reverse,
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{sync::watch, task::JoinSet};

// Bound the eagerly sorted schedule and the growing in-memory case summaries.
pub const MAX_BATCH_COUNT: u32 = 10_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RunConfig {
    #[serde(skip_serializing_if = "Mode::is_solo")]
    pub mode: Mode,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub target_name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub opponents: Vec<PlayerConfig>,
    pub count: Option<u32>,
    pub seed: Option<String>,
    pub format: Option<MatchFormat>,
    pub arena_maps: bool,
    pub jobs: Option<usize>,
    pub solver_parallel: bool,
    pub label: String,
    pub build: Option<CommandSpec>,
    pub solver: CommandSpec,
    pub time_limit_ms: Option<u64>,
}
impl Default for RunConfig {
    fn default() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            mode: Mode::Solo,
            target_name: String::new(),
            opponents: vec![],
            count: Some(60),
            seed: None,
            format: None,
            arena_maps: true,
            jobs: None,
            solver_parallel: false,
            label: String::new(),
            build: Some(CommandSpec {
                program: "cargo".into(),
                args: vec![
                    "build".into(),
                    "--release".into(),
                    "-p".into(),
                    "solver".into(),
                ],
                cwd: cwd.clone(),
                env: BTreeMap::new(),
            }),
            solver: CommandSpec {
                program: "target/release/solver".into(),
                args: vec![],
                cwd,
                env: BTreeMap::new(),
            },
            time_limit_ms: None,
        }
    }
}
impl RunConfig {
    pub fn validate(&mut self) -> Result<()> {
        ensure!(
            self.target_name.chars().count() <= 80,
            "対象の名前は80文字以内です"
        );
        ensure!(
            self.opponents.len() == if self.mode == Mode::Game { 4 } else { 0 },
            "対戦は相手4人、単独テスタは相手なしで指定してください"
        );
        for opponent in &mut self.opponents {
            ensure!(
                !opponent.name.trim().is_empty() && opponent.name.chars().count() <= 80,
                "相手の名前は1〜80文字です"
            );
            opponent.solver.normalize()?;
            if let Some(build) = &mut opponent.build {
                build.normalize()?;
            }
        }

        ensure!(
            self.seed.is_some() != self.count.is_some(),
            "seed数または単一seedのどちらか一方を指定してください"
        );
        if let Some(s) = &self.seed {
            s.parse::<u64>()
                .context("seedは0〜18446744073709551615の整数です")?;
        }
        if let Some(n) = self.count {
            ensure!(
                (1..=MAX_BATCH_COUNT).contains(&n),
                "seed数は1〜{MAX_BATCH_COUNT}にしてください"
            );
        }
        ensure!(
            self.jobs.is_none_or(|n| (1..=256).contains(&n)),
            "同時実行数は1〜256、または自動にしてください"
        );
        ensure!(
            self.label.chars().count() <= 80,
            "実行名は80文字以内にしてください"
        );
        if let Some(t) = self.time_limit_ms {
            ensure!(
                (1..=3_600_000).contains(&t),
                "制限時間は1〜3600000 msにしてください"
            );
        }
        if let Some(b) = &mut self.build {
            b.normalize()?;
        }
        self.solver.normalize()?;
        Ok(())
    }
    pub fn player_names(&self) -> Vec<String> {
        std::iter::once(if self.target_name.trim().is_empty() {
            "対象ソルバ".into()
        } else {
            self.target_name.clone()
        })
        .chain(self.opponents.iter().map(|p| p.name.clone()))
        .collect()
    }
    pub fn total(&self) -> u32 {
        self.count.unwrap_or(1)
    }
    pub fn execution(&self, available_cpus: usize) -> Execution {
        let available_cpus = available_cpus.max(1);
        let jobs = self.jobs.unwrap_or(
            if self.solver_parallel || self.opponents.iter().any(|p| p.solver_parallel) {
                1
            } else {
                (available_cpus / self.mode.players() as usize).max(1)
            },
        );
        Execution {
            available_cpus,
            jobs: jobs.min(self.total() as usize),
        }
    }
    fn seed_at(&self, index: u32) -> u64 {
        self.seed
            .as_ref()
            .map(|s| s.parse().unwrap())
            .unwrap_or(u64::from(index))
    }
}

pub fn available_cpus() -> usize {
    std::thread::available_parallelism().map_or(1, usize::from)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Execution {
    pub available_cpus: usize,
    pub jobs: usize,
}

pub fn verdict<'a>(failures: impl Iterator<Item = &'a Failure>) -> &'static str {
    let mut result = "AC";
    for failure in failures {
        if failure.kind == "timeout" {
            return "TLE";
        }
        if failure.kind == "process" {
            result = "RE";
        } else if result == "AC" {
            result = "WA";
        }
    }
    result
}

pub fn batch_verdict(cases: &[CaseSummary]) -> &'static str {
    for status in ["TLE", "RE", "WA"] {
        if cases.iter().any(|case| case.status == status) {
            return status;
        }
    }
    "AC"
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseSummary {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub players: Vec<PlayerSummary>,
    pub seed: String,
    pub format: MatchFormat,
    pub status: String,
    pub score: Score,
    pub answer_time_ms: u64,
    pub warnings: usize,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub opponent_identities: Vec<String>,
    pub schema_version: u32,
    pub id: String,
    pub status: String,
    #[serde(deserialize_with = "deserialize_history_config")]
    pub config: RunConfig,
    #[serde(default)]
    pub execution: Option<Execution>,
    #[serde(default)]
    pub verdict: Option<String>,
    pub total: u32,
    pub completed: u32,
    pub cases: Vec<CaseSummary>,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_from: Option<String>,
}

// Settings without arenaMaps use the new default, but saved records without it
// predate arena imports. Apply the legacy default only when reading history.
pub fn deserialize_history_config<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let mut config = serde_json::Value::deserialize(deserializer)?;
    if let Some(fields) = config.as_object_mut() {
        fields
            .entry("arenaMaps")
            .or_insert(serde_json::Value::Bool(false));
    }
    serde_json::from_value(config).map_err(serde::de::Error::custom)
}
#[derive(Clone)]
pub enum ProgressEvent {
    Status(Box<Manifest>),
    BuildLog(String),
}
pub type Progress = Arc<dyn Fn(ProgressEvent) + Send + Sync>;

pub fn run_id() -> String {
    format!(
        "{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        std::process::id()
    )
}
pub async fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let temporary = path.with_extension("json.tmp");
    tokio::fs::write(&temporary, serde_json::to_vec(value)?).await?;
    tokio::fs::rename(temporary, path).await?;
    Ok(())
}

pub async fn run_batch(
    id: String,
    mut config: RunConfig,
    root: PathBuf,
    progress: Progress,
    mut cancel: watch::Receiver<bool>,
) -> Result<Manifest> {
    config.validate()?;
    let directory = root.join(&id);
    tokio::fs::create_dir_all(&directory).await?;
    let mut manifest = Manifest {
        opponent_identities: vec![],

        imported_from: None,
        schema_version: 3,
        execution: Some(config.execution(available_cpus())),
        verdict: None,
        id,
        status: "building".into(),
        total: config.total(),
        completed: 0,
        cases: vec![],
        error: None,
        config: config.clone(),
    };
    write_json(&directory.join("run.json"), &manifest).await?;
    progress(ProgressEvent::Status(Box::new(manifest.clone())));
    let result = tokio::select! {
        result=execute_batch(&mut manifest,&directory,progress.clone())=>result,
        _=async {if !*cancel.borrow() {let _=cancel.changed().await;}}=>Err(anyhow!("実行を中断しました")),
    };
    match result {
        Ok(()) => {
            manifest.status = "completed".into();
            manifest.verdict = Some(batch_verdict(&manifest.cases).into());
        }
        Err(e) => {
            manifest.status = if *cancel.borrow() {
                "cancelled"
            } else {
                "failed"
            }
            .into();
            manifest.error = Some(format!("{e:#}"));
        }
    }
    manifest
        .cases
        .sort_by_key(|c| c.seed.parse::<u64>().unwrap_or(0));
    write_json(&directory.join("run.json"), &manifest).await?;
    progress(ProgressEvent::Status(Box::new(manifest.clone())));
    Ok(manifest)
}

async fn execute_batch(
    manifest: &mut Manifest,
    directory: &Path,
    progress: Progress,
) -> Result<()> {
    if let Some(build) = &manifest.config.build
        && let Err(error) = run_build(build, directory, {
            let p = progress.clone();
            Arc::new(move |s| p(ProgressEvent::BuildLog(s)))
        })
        .await
    {
        manifest.verdict = Some("CE".into());
        return Err(error);
    }
    let mut built = std::collections::BTreeSet::new();
    if let Some(build) = &manifest.config.build {
        built.insert(serde_json::to_string(build)?);
    }
    for opponent in &manifest.config.opponents {
        if let Some(build) = &opponent.build
            && built.insert(serde_json::to_string(build)?)
        {
            let p = progress.clone();
            if let Err(e) = run_build(
                build,
                directory,
                Arc::new(move |s| p(ProgressEvent::BuildLog(s))),
            )
            .await
            {
                manifest.verdict = Some("CE".into());
                return Err(e);
            }
        }
    }
    if manifest.config.mode == Mode::Game {
        manifest.opponent_identities = manifest
            .config
            .opponents
            .iter()
            .map(crate::competition::identity)
            .collect::<Result<Vec<_>>>()?;
    }
    manifest.status = "running".into();
    write_json(&directory.join("run.json"), manifest).await?;
    progress(ProgressEvent::Status(Box::new(manifest.clone())));
    let mut pending = scheduled_seeds(&manifest.config).await?.into_iter();
    let mut tasks = JoinSet::new();
    while pending.len() > 0 || !tasks.is_empty() {
        while tasks.len() < manifest.execution.as_ref().unwrap().jobs {
            let Some(seed) = pending.next() else { break };
            let cfg = manifest.config.clone();
            let dir = directory.join(seed.to_string());
            tasks.spawn(async move { run_case(seed, &cfg, &dir).await });
        }
        if let Some(result) = tasks.join_next().await {
            manifest
                .cases
                .push(result.context("ケースの実行タスクが終了しました")??);
            manifest.completed += 1;
            write_json(&directory.join("run.json"), manifest).await?;
            progress(ProgressEvent::Status(Box::new(manifest.clone())));
        }
    }
    Ok(())
}

fn estimated_case_ms(seed: u64, config: &RunConfig) -> Result<u64> {
    let days = day_seconds(seed, config.format, config.arena_maps)?;
    Ok(if let Some(limit) = config.time_limit_ms {
        limit * (1 + days.len() as u64)
    } else {
        resolve_format(seed, config.format).parameters().4
            + days.into_iter().map(|s| u64::from(s) * 1000).sum::<u64>()
    })
}

async fn scheduled_seeds(config: &RunConfig) -> Result<Vec<u64>> {
    // Longest processing time first: deadline-driven solvers leave fewer idle slots
    // at the end. Game players answer concurrently, so do not multiply by five.
    // Store only cost/seed pairs; maps are generated when an execution slot opens.
    let mut pending = Vec::new();
    for index in 0..config.total() {
        let seed = config.seed_at(index);
        pending.push((Reverse(estimated_case_ms(seed, config)?), seed));
        if index % 1024 == 1023 {
            tokio::task::yield_now().await;
        }
    }
    pending.sort_unstable(); // Equal estimates retain ascending seed order.
    Ok(pending.into_iter().map(|(_, seed)| seed).collect())
}

async fn run_case(seed: u64, config: &RunConfig, directory: &Path) -> Result<CaseSummary> {
    if config.mode == Mode::Game {
        return crate::competition::run_case(seed, config, directory).await;
    }

    tokio::fs::create_dir_all(directory).await?;
    let case = generate_with_arena_maps(seed, config.format, config.arena_maps)?;
    write_json(&directory.join("problem.json"), &case.problem).await?;
    let (mut solver, startup) = match SolverProcess::spawn(&config.solver, directory).await {
        Ok(p) => (Some(p), None),
        Err(e) => (None, Some(Failure::new("process", format!("{e:#}")))),
    };
    let mut request = Request {
        protocol_version: PROTOCOL_VERSION,
        request_id: 0,
        phase: "select".into(),
        time_limit_ms: config.time_limit_ms.unwrap_or(case.format.parameters().4),
        problem: Some(case.problem.clone()),
        state: None,
        steps: None,
        previous_accepted: None,
    };
    let (selection_answer, mut selection_failure, selection_ms) = if let Some(p) = &mut solver {
        p.exchange(&request).await
    } else {
        (None, startup.clone(), 0)
    };
    let kinds = if let Some(answer) = &selection_answer {
        match validate_kinds(answer, case.problem.agents.len()) {
            Ok(k) => k,
            Err(e) => {
                selection_failure = Some(e);
                vec![0; case.problem.agents.len()]
            }
        }
    } else {
        vec![0; case.problem.agents.len()]
    };
    let mut previous_accepted = selection_failure.is_none();
    let mut game = Game::new(case.problem.clone(), &kinds);
    let mut days = Vec::new();
    let mut answer_time_ms = 0;
    while game.day < case.problem.day_steps.len() {
        let limit = config
            .time_limit_ms
            .unwrap_or(u64::from(case.problem.day_seconds[game.day]) * 1000);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        request = Request {
            protocol_version: PROTOCOL_VERSION,
            request_id: (game.day + 1) as u64,
            phase: "plan".into(),
            time_limit_ms: limit,
            problem: None,
            state: Some(game.observation(now + limit.div_ceil(1000))),
            steps: Some(case.problem.day_steps[game.day]),
            previous_accepted: Some(previous_accepted),
        };
        let (answer, mut failure, elapsed) = if let Some(p) = &mut solver {
            p.exchange(&request).await
        } else {
            (None, startup.clone(), 0)
        };
        let mut accepted = None;
        if let Some(value) = &answer {
            match serde_json::from_value::<Plan>(value.clone()) {
                Ok(plan) => match game.play_day(plan) {
                    Ok(day) => accepted = Some(day),
                    Err(e) => failure = Some(e),
                },
                Err(e) => {
                    failure = Some(Failure::new(
                        "invalid_plan",
                        format!("行動計画の形式が不正です: {e}"),
                    ))
                }
            }
        }
        previous_accepted = failure.is_none();
        let mut day = match accepted {
            Some(day) => {
                answer_time_ms += elapsed;
                day
            }
            None => game
                .play_day(game.waiting_plan())
                .map_err(|e| anyhow!("待機への置き換えに失敗: {}", e.message))?,
        };
        day.submitted_answer = answer;
        day.failure = failure;
        day.elapsed_ms = elapsed;
        days.push(day);
    }
    if let Some(p) = &mut solver {
        p.finish().await;
    }
    let warnings = usize::from(selection_failure.is_some())
        + days.iter().filter(|d| d.failure.is_some()).count();
    let summary = CaseSummary {
        players: vec![],

        seed: case.seed.clone(),
        format: case.format,
        status: verdict(
            selection_failure
                .iter()
                .chain(days.iter().filter_map(|d| d.failure.as_ref())),
        )
        .into(),
        score: game.score.clone(),
        answer_time_ms,
        warnings,
        error: None,
    };
    let replay = Replay {
        opponents: vec![],

        schema_version: REPLAY_VERSION,
        case,
        kinds,
        selection_answer,
        selection_failure,
        selection_ms,
        days,
        score: game.score,
        answer_time_ms,
    };
    write_json(&directory.join("replay.json"), &replay).await?;
    Ok(summary)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::generator::generate;
    use tokio::time::Duration;

    #[test]
    fn history_arena_defaults_preserve_old_runs_and_explicit_settings() {
        let new_config: RunConfig = serde_json::from_str("{}").unwrap();
        assert!(new_config.arena_maps);
        for mode in [Mode::Solo, Mode::Game] {
            for setting in [None, Some(false), Some(true)] {
                let mut config = serde_json::to_value(RunConfig::for_mode(mode)).unwrap();
                config.as_object_mut().unwrap().remove("arenaMaps");
                if let Some(enabled) = setting {
                    config["arenaMaps"] = enabled.into();
                }
                let mut manifest: Manifest = serde_json::from_value(serde_json::json!({
                    "schemaVersion": 3, "id": "101-1", "status": "running",
                    "config": config, "total": 60, "completed": 0,
                    "cases": [], "error": null,
                }))
                .unwrap();
                manifest.config.validate().unwrap();
                assert_eq!(manifest.config.arena_maps, setting.unwrap_or(false));
                let restored: Manifest =
                    serde_json::from_value(serde_json::to_value(manifest).unwrap()).unwrap();
                assert_eq!(restored.config.arena_maps, setting.unwrap_or(false));
            }
        }
    }

    #[test]
    fn batch_count_is_bounded_without_restricting_single_seeds() {
        for mode in [Mode::Solo, Mode::Game] {
            for count in [0, 10_001, u32::MAX] {
                let mut cfg = RunConfig::for_mode(mode);
                cfg.count = Some(count);
                assert!(cfg.validate().is_err(), "accepted count {count}");
            }
            for count in [1, 60, 10_000] {
                let mut cfg = RunConfig::for_mode(mode);
                cfg.count = Some(count);
                cfg.validate().unwrap();
            }
            let mut cfg = RunConfig::for_mode(mode);
            cfg.count = None;
            cfg.seed = Some(u64::MAX.to_string());
            cfg.validate().unwrap();
        }
    }

    #[tokio::test]
    async fn oversized_batch_is_rejected_before_creating_history() {
        let temp = Temp::new();
        let mut cfg = config(&temp.0, "valid");
        cfg.count = Some(u32::MAX);
        let (_tx, rx) = watch::channel(false);
        let error = run_batch("run".into(), cfg, temp.0.clone(), Arc::new(|_| {}), rx)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("seed数"));
        assert!(!temp.0.join("run").exists());
    }

    #[test]
    fn automatic_concurrency_respects_solver_parallelism_and_case_count() {
        let mut cfg = RunConfig::default();
        assert_eq!(cfg.total(), 60);
        assert_eq!(cfg.execution(8).jobs, 8);
        assert_eq!(cfg.execution(128).jobs, 60);
        cfg.solver_parallel = true;
        assert_eq!(cfg.execution(8).jobs, 1);
        cfg.jobs = Some(3);
        assert_eq!(cfg.execution(8).jobs, 3);
        cfg.count = None;
        cfg.seed = Some("0".into());
        assert_eq!(cfg.execution(8).jobs, 1);
    }

    #[test]
    fn arena_map_config_defaults_to_enabled_and_preserves_explicit_exclusion() {
        let legacy: RunConfig = serde_json::from_str("{}").unwrap();
        assert!(legacy.arena_maps);
        let config: RunConfig = serde_json::from_str(r#"{"arenaMaps":false}"#).unwrap();
        assert!(!config.arena_maps);
        assert_eq!(serde_json::to_value(config).unwrap()["arenaMaps"], false);
    }

    #[test]
    fn estimates_match_generated_request_limits() {
        for format in [
            None,
            Some(MatchFormat::First),
            Some(MatchFormat::Semifinal),
            Some(MatchFormat::Final),
        ] {
            for seed in (0..60).chain([u64::MAX]) {
                for arena_maps in [true, false] {
                    let case = generate_with_arena_maps(seed, format, arena_maps).unwrap();
                    for limit in [None, Some(1250)] {
                        let cfg = RunConfig {
                            format,
                            arena_maps,
                            time_limit_ms: limit,
                            ..RunConfig::default()
                        };
                        let selection = limit.unwrap_or(case.format.parameters().4);
                        let daily: u64 = case
                            .problem
                            .day_seconds
                            .iter()
                            .map(|&s| limit.unwrap_or(u64::from(s) * 1000))
                            .sum();
                        assert_eq!(estimated_case_ms(seed, &cfg).unwrap(), selection + daily);
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn schedule_respects_overrides_ties_and_single_seed() {
        let mut cfg = RunConfig {
            count: Some(6),
            ..RunConfig::default()
        };
        assert_eq!(scheduled_seeds(&cfg).await.unwrap(), [3, 5, 2, 1, 4, 0]);
        cfg.time_limit_ms = Some(1500);
        assert_eq!(scheduled_seeds(&cfg).await.unwrap(), [3, 5, 0, 1, 2, 4]);
        cfg.count = None;
        cfg.seed = Some(u64::MAX.to_string());
        assert_eq!(scheduled_seeds(&cfg).await.unwrap(), [u64::MAX]);
    }

    #[tokio::test]
    async fn long_cases_first_reduce_the_full_60_case_tail() {
        let cfg = RunConfig::default();
        let order = scheduled_seeds(&cfg).await.unwrap();
        let mut seeds = order.clone();
        seeds.sort_unstable();
        assert_eq!(seeds, (0..60).collect::<Vec<_>>());
        let costs: Vec<u64> = (0..60)
            .map(|seed| {
                let case = generate(seed, None).unwrap();
                case.format.parameters().4
                    + case
                        .problem
                        .day_seconds
                        .iter()
                        .map(|&s| u64::from(s) * 1000)
                        .sum::<u64>()
            })
            .collect();
        let makespan = |order: &[u64]| {
            let mut slots = [0_u64; 24];
            for &seed in order {
                *slots.iter_mut().min().unwrap() += costs[seed as usize];
            }
            *slots.iter().max().unwrap()
        };
        assert!(makespan(&order) < makespan(&seeds));
    }

    #[tokio::test]
    async fn both_modes_dispatch_long_cases_first_and_save_in_seed_order() {
        for mode in [Mode::Solo, Mode::Game] {
            let temp = Temp::new();
            let mut cfg = RunConfig::for_mode(mode);
            cfg.count = Some(6);
            cfg.jobs = Some(1);
            cfg.build = None;
            cfg.solver = config(&temp.0, "quiet").solver;
            for opponent in &mut cfg.opponents {
                opponent.build = None;
                opponent.solver = cfg.solver.clone();
            }
            let completed = Arc::new(std::sync::Mutex::new(Vec::new()));
            let observed = completed.clone();
            let progress = Arc::new(move |event| {
                if let ProgressEvent::Status(m) = event {
                    let mut seeds = observed.lock().unwrap();
                    if m.completed as usize > seeds.len() {
                        seeds.push(m.cases.last().unwrap().seed.parse::<u64>().unwrap());
                    }
                }
            });
            let (_tx, rx) = watch::channel(false);
            let manifest = run_batch("run".into(), cfg, temp.0.clone(), progress, rx)
                .await
                .unwrap();
            assert_eq!(manifest.verdict.as_deref(), Some("AC"));
            assert_eq!(*completed.lock().unwrap(), [3, 5, 2, 1, 4, 0]);
            assert_eq!(manifest.completed, 6);
            let saved: Manifest =
                serde_json::from_slice(&std::fs::read(temp.0.join("run/run.json")).unwrap())
                    .unwrap();
            for (seed, summary) in saved.cases.iter().enumerate() {
                assert_eq!(summary.seed, seed.to_string());
                assert!(summary.players.iter().all(|p| p.status == "AC"));
                let r = replay(&temp.0.join("run").join(&summary.seed)).await;
                assert_eq!(
                    r.case,
                    crate::generator::generate_for_players(seed as u64, None, mode.players())
                        .unwrap()
                );
            }
        }
    }

    #[test]
    fn time_limit_verdict_takes_priority_over_wrong_answers() {
        let failures = [
            Failure::new("protocol", "bad JSON"),
            Failure::new("timeout", "late"),
        ];
        assert_eq!(verdict(failures.iter()), "TLE");
        assert_eq!(verdict(failures[..1].iter()), "WA");
        assert_eq!(verdict([].iter()), "AC");
        let process = Failure::new("process", "exit");
        assert_eq!(verdict([&failures[0], &process].into_iter()), "RE");
        assert_eq!(verdict([&process, &failures[0]].into_iter()), "RE");
        assert_eq!(verdict([&process, &failures[1]].into_iter()), "TLE");
    }

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("hexaudon-test-{}", run_id()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn config(dir: &Path, mode: &str) -> RunConfig {
        RunConfig {
            mode: Mode::Solo,
            target_name: String::new(),
            opponents: vec![],
            count: Some(2),
            seed: None,
            format: Some(MatchFormat::First),
            arena_maps: true,
            jobs: Some(2),
            solver_parallel: false,
            label: String::new(),
            build: None,
            time_limit_ms: Some(2000),
            solver: CommandSpec {
                program: "python3".into(),
                args: vec![
                    "-u".into(),
                    "-c".into(),
                    include_str!("../../tester/solo/tests/fixtures/solver.py").into(),
                    mode.into(),
                ],
                cwd: dir.into(),
                env: BTreeMap::new(),
            },
        }
    }
    async fn replay(path: &Path) -> Replay {
        serde_json::from_slice(&tokio::fs::read(path.join("replay.json")).await.unwrap()).unwrap()
    }
    #[tokio::test]
    async fn seed_processes_overlap_only_with_available_job_slots() {
        for (parallel, expected) in [(false, 2), (true, 1)] {
            let temp = Temp::new();
            let mut cfg = config(&temp.0, "schedule");
            cfg.count = Some(3);
            cfg.jobs = if parallel { None } else { Some(2) };
            cfg.solver_parallel = parallel;
            let (_tx, rx) = watch::channel(false);
            let m = run_batch("run".into(), cfg, temp.0.clone(), Arc::new(|_| {}), rx)
                .await
                .unwrap();
            assert_eq!(m.verdict.as_deref(), Some("AC"));
            assert_eq!(m.execution.unwrap().jobs, expected);
            let counts: Value =
                serde_json::from_slice(&std::fs::read(temp.0.join("parallel.json")).unwrap())
                    .unwrap();
            assert_eq!(counts["peak"], expected);
            assert_eq!(counts["active"], 0);
        }
    }

    #[tokio::test]
    async fn timeouts_are_tle_in_summary_and_legacy_records() {
        let temp = Temp::new();
        let mut cfg = config(&temp.0, "late");
        cfg.count = Some(1);
        cfg.time_limit_ms = Some(200);
        let (_tx, rx) = watch::channel(false);
        let mut m = run_batch("run".into(), cfg, temp.0.clone(), Arc::new(|_| {}), rx)
            .await
            .unwrap();
        assert_eq!(m.cases[0].status, "TLE");
        assert_eq!(m.verdict.as_deref(), Some("TLE"));
        m.schema_version = 1;
        m.cases[0].status = "fallback".into();
        write_json(&temp.0.join("run/run.json"), &m).await.unwrap();
        let loaded = crate::comparison::read_manifest(&temp.0.join("run"))
            .await
            .unwrap();
        assert_eq!(loaded.cases[0].status, "TLE");
    }
    #[tokio::test]
    async fn builds_once_runs_parallel_and_drains_threaded_stderr() {
        let temp = Temp::new();
        let mut cfg = config(&temp.0, "valid");
        cfg.build = Some(CommandSpec {
            program: "python3".into(),
            args: vec!["-c".into(), "open('built.txt','a').write('once\\n')".into()],
            cwd: temp.0.clone(),
            env: BTreeMap::new(),
        });
        let (_tx, rx) = watch::channel(false);
        let manifest = run_batch("run".into(), cfg, temp.0.clone(), Arc::new(|_| {}), rx)
            .await
            .unwrap();
        assert_eq!(manifest.status, "completed");
        assert_eq!(manifest.completed, 2);
        assert_eq!(
            std::fs::read_to_string(temp.0.join("built.txt")).unwrap(),
            "once\n"
        );
        assert!(manifest.cases.iter().all(|c| c.status == "AC"));
        for seed in 0..2 {
            let dir = temp.0.join("run").join(seed.to_string());
            let r = replay(&dir).await;
            assert_eq!(r.case, generate(seed, Some(MatchFormat::First)).unwrap());
            assert_eq!(r.days.len(), r.case.problem.day_steps.len());
            assert!(std::fs::metadata(dir.join("stderr.log")).unwrap().len() > 400_000);
            for day in r.days {
                assert_eq!(day.frames.len(), day.steps as usize + 1);
                assert!(day.failure.is_none());
            }
        }
    }
    #[tokio::test]
    async fn invalid_responses_fall_back_and_continue_with_actual_state() {
        let temp = Temp::new();
        let cfg = config(&temp.0, "invalid");
        let result = run_case(0, &cfg, &temp.0).await.unwrap();
        let r = replay(&temp.0).await;
        assert_eq!(result.status, "WA");
        assert_eq!(result.warnings, 2);
        assert_eq!(r.kinds, vec![0; 4]);
        assert!(r.selection_failure.is_some());
        assert!(r.days[0].failure.is_some());
        assert_eq!(
            r.days[0].submitted_answer.as_ref().unwrap()[0],
            serde_json::json!([6])
        );
        assert!(r.days[1..].iter().all(|d| d.failure.is_none()));
        assert_eq!(r.days[0].plan, vec![vec![-i64::from(r.days[0].steps)]; 4]);
        assert!(
            r.days[0]
                .frames
                .last()
                .unwrap()
                .agents
                .iter()
                .zip(r.case.problem.agents)
                .all(|(a, p)| a.pos == p)
        );
    }
    #[tokio::test]
    async fn terminated_process_finishes_all_days_without_restart() {
        let temp = Temp::new();
        let cfg = config(&temp.0, "exit");
        let result = run_case(0, &cfg, &temp.0).await.unwrap();
        let r = replay(&temp.0).await;
        assert_eq!(result.status, "RE");
        assert_eq!(result.warnings, 1 + r.days.len());
        assert!(r.days.iter().all(|d| d.failure.is_some()));
    }
    #[tokio::test]
    async fn startup_failure_is_re_and_legacy_wa_is_reclassified() {
        let temp = Temp::new();
        let mut cfg = config(&temp.0, "valid");
        cfg.solver.program = "./missing-solver".into();
        cfg.count = Some(1);
        let (_tx, rx) = watch::channel(false);
        let mut manifest = run_batch("run".into(), cfg, temp.0.clone(), Arc::new(|_| {}), rx)
            .await
            .unwrap();
        assert_eq!(manifest.verdict.as_deref(), Some("RE"));
        assert_eq!(manifest.cases[0].status, "RE");
        manifest.schema_version = 2;
        manifest.verdict = Some("WA".into());
        manifest.cases[0].status = "WA".into();
        write_json(&temp.0.join("run/run.json"), &manifest)
            .await
            .unwrap();
        let loaded = crate::comparison::read_manifest(&temp.0.join("run"))
            .await
            .unwrap();
        assert_eq!(loaded.cases[0].status, "RE");
        assert_eq!(loaded.verdict.as_deref(), Some("RE"));
    }

    #[tokio::test]
    async fn expired_response_is_discarded_by_request_id() {
        let temp = Temp::new();
        let cfg = config(&temp.0, "late");
        let mut process = SolverProcess::spawn(&cfg.solver, &temp.0).await.unwrap();
        let mut req = Request {
            protocol_version: 1,
            request_id: 0,
            phase: "select".into(),
            time_limit_ms: 200,
            problem: Some(generate(0, None).unwrap().problem),
            state: None,
            steps: None,
            previous_accepted: None,
        };
        let (_, failure, _) = process.exchange(&req).await;
        assert_eq!(failure.unwrap().kind, "timeout");
        req.request_id = 1;
        req.phase = "plan".into();
        req.steps = Some(10);
        req.time_limit_ms = 2000;
        let (answer, failure, _) = process.exchange(&req).await;
        assert!(failure.is_none());
        assert_eq!(answer.unwrap()[0], serde_json::json!([-10]));
        process.finish().await;
    }
    #[tokio::test]
    async fn malformed_and_oversized_stdout_are_rejected_without_hanging() {
        for mode in ["malformed", "oversized"] {
            let temp = Temp::new();
            let cfg = config(&temp.0, mode);
            let result = run_case(0, &cfg, &temp.0).await.unwrap();
            let r = replay(&temp.0).await;
            assert_eq!(result.status, "WA");
            assert_eq!(r.selection_failure.unwrap().kind, "protocol");
        }
    }
    #[tokio::test]
    async fn build_failure_stops_before_cases() {
        let temp = Temp::new();
        let mut cfg = config(&temp.0, "valid");
        cfg.build = Some(CommandSpec {
            program: "sh".into(),
            args: vec!["-c".into(), "echo failure >&2; exit 3".into()],
            cwd: temp.0.clone(),
            env: BTreeMap::new(),
        });
        let (_tx, rx) = watch::channel(false);
        let m = run_batch("run".into(), cfg, temp.0.clone(), Arc::new(|_| {}), rx)
            .await
            .unwrap();
        assert_eq!(m.status, "failed");
        assert_eq!(m.verdict.as_deref(), Some("CE"));
        assert_eq!(m.completed, 0);
        assert!(
            std::fs::read_to_string(temp.0.join("run/build.stderr.log"))
                .unwrap()
                .contains("failure")
        );
        assert!(!temp.0.join("run/0").exists());
    }
    #[tokio::test]
    async fn cancel_stops_solver_process_group() {
        let temp = Temp::new();
        let mut cfg = config(&temp.0, "cancel");
        cfg.count = Some(1);
        cfg.time_limit_ms = Some(30_000);
        let (tx, rx) = watch::channel(false);
        let dir = temp.0.clone();
        let task = tokio::spawn(run_batch("run".into(), cfg, dir, Arc::new(|_| {}), rx));
        let started = temp.0.join("started.json");
        let pids: Vec<u32> = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(bytes) = tokio::fs::read(&started).await
                    && let Ok(ids) = serde_json::from_slice(&bytes)
                {
                    break ids;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        tx.send(true).unwrap();
        let m = task.await.unwrap().unwrap();
        assert_eq!(m.status, "cancelled");
        for pid in pids {
            // A terminated orphan may briefly be a zombie until the OS reaps it.
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if unsafe { libc::kill(pid as i32, 0) } != 0 {
                        break;
                    }
                    #[cfg(target_os = "linux")]
                    if std::fs::read_to_string(format!("/proc/{pid}/stat"))
                        .is_ok_and(|s| s.contains(") Z"))
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("solver process survived cancellation");
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Solo,
    Game,
}
impl Mode {
    pub fn is_solo(&self) -> bool {
        *self == Self::Solo
    }
    pub fn players(self) -> u32 {
        if self == Self::Game { 5 } else { 1 }
    }
    pub fn root(self) -> &'static str {
        if self == Self::Game {
            ".game/runs"
        } else {
            ".tester/runs"
        }
    }
    pub fn archive(self) -> &'static str {
        if self == Self::Game {
            "procon37-game-run"
        } else {
            "procon37-run"
        }
    }
    pub fn port(self) -> u16 {
        if self == Self::Game { 8081 } else { 8080 }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerConfig {
    pub name: String,
    pub solver: CommandSpec,
    pub build: Option<CommandSpec>,
    #[serde(default)]
    pub solver_parallel: bool,
    #[serde(default)]
    pub version: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlayerSummary {
    pub id: usize,
    pub name: String,
    pub rank: usize,
    pub score: Score,
    pub status: String,
    pub answer_time_ms: u64,
    pub warnings: usize,
}
impl RunConfig {
    pub fn for_mode(mode: Mode) -> Self {
        let mut config = Self {
            mode,
            ..Self::default()
        };
        if mode == Mode::Game {
            config.opponents = (1..=4)
                .map(|id| PlayerConfig {
                    name: [
                        "pool-generation-002",
                        "pool-generation-001",
                        "pool-generation-004",
                        "pool-generation-003",
                    ][id - 1]
                        .into(),
                    version: [
                        "pool-generation-002-run-1790551749037999892-54402-v1",
                        "pool-generation-001-run-1790521499134996039-41850-v1",
                        "pool-generation-004-run-1790600156388062978-4-v1",
                        "pool-generation-003-run-1790563781811011480-62511-v1",
                    ][id - 1]
                        .into(),
                    solver: CommandSpec {
                        program: format!("target/release/game-sample-{id}"),
                        args: vec!["0".into()],
                        cwd: config.solver.cwd.clone(),
                        env: BTreeMap::new(),
                    },
                    build: Some(CommandSpec {
                        program: "cargo".into(),
                        args: vec![
                            "build".into(),
                            "--release".into(),
                            "-p".into(),
                            "game-samples".into(),
                        ],
                        cwd: config.solver.cwd.clone(),
                        env: BTreeMap::new(),
                    }),
                    solver_parallel: false,
                })
                .collect();
        }
        config
    }
}
