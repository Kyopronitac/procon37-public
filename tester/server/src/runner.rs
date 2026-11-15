//! 対戦の実行。seedごとに問題を生成して公式対戦サーバーを起動し、
//! 対戦窓口が記録した提出から共通エンジンで結果を再現して保存する。

use crate::state::{ActiveMatch, Registration, SharedState, Submission, TeamRecord, now_ms};
use anyhow::{Context, Result, anyhow, bail, ensure};
use procon37_common::{
    competition::{rank, total_stays},
    engine::{Game, traffic_for, validate_kinds},
    generator::generate_for_players_with_arena_maps,
    model::*,
    runner::{CaseSummary, MAX_BATCH_COUNT, PlayerSummary, batch_verdict, verdict, write_json},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{sync::watch, time::Instant};

/// 保存形式は単独・対戦テスタの run.json schema 3 と同じ。`config` だけが server 固有。
pub const SCHEMA_VERSION: u32 = 3;
pub const MAX_PLAYERS: u32 = 16;
pub const DEFAULT_PROGRAM: &str = "tester/server/procon-server";
const MATCH_START_DELAY_SECS: u64 = 5;
/// 試合終了後に、処理中の提出の記録を待つ上限。対戦窓口の1提出は公式対戦サーバーへ最大3回(各10秒で打ち切り)問い合わせる。
const DRAIN_LIMIT: Duration = Duration::from_secs(35);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ServerConfig {
    pub count: Option<u32>,
    pub seed: Option<String>,
    pub format: Option<MatchFormat>,
    pub arena_maps: bool,
    pub label: String,
    /// チーム数。トークンは token-p0, token-p1, … の順。
    pub players: u32,
    /// チームの表示名。空欄は「チームN」。
    pub team_names: Vec<String>,
    /// 車種選択・各日の回答時間の上書き。各日の回答時間は問題の形式に合わせて秒単位に切り上げる。
    pub time_limit_ms: Option<u64>,
    /// 公式対戦サーバー(procon-server)の実行ファイル。
    pub server_program: String,
}
impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            count: Some(12),
            seed: None,
            format: None,
            arena_maps: true,
            label: String::new(),
            players: 1,
            team_names: vec![],
            time_limit_ms: None,
            server_program: DEFAULT_PROGRAM.into(),
        }
    }
}
impl ServerConfig {
    pub fn validate(&mut self) -> Result<()> {
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
            (1..=MAX_PLAYERS).contains(&self.players),
            "チーム数は1〜{MAX_PLAYERS}にしてください"
        );
        ensure!(
            self.team_names.len() <= self.players as usize,
            "チーム名がチーム数より多く指定されています"
        );
        for name in &mut self.team_names {
            *name = name.trim().into();
            ensure!(name.chars().count() <= 80, "チーム名は80文字以内です");
        }
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
        ensure!(
            !self.server_program.trim().is_empty(),
            "公式対戦サーバーの実行ファイルを指定してください"
        );
        Ok(())
    }
    pub fn player_names(&self) -> Vec<String> {
        (0..self.players as usize)
            .map(|i| match self.team_names.get(i) {
                Some(name) if !name.trim().is_empty() => name.clone(),
                _ => format!("チーム{i}"),
            })
            .collect()
    }
    pub fn total(&self) -> u32 {
        self.count.unwrap_or(1)
    }
    fn seed_at(&self, index: u32) -> u64 {
        self.seed
            .as_ref()
            .map_or(u64::from(index), |s| s.parse().unwrap())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema_version: u32,
    pub id: String,
    pub status: String,
    #[serde(deserialize_with = "procon37_common::runner::deserialize_history_config")]
    pub config: ServerConfig,
    #[serde(default)]
    pub verdict: Option<String>,
    pub total: u32,
    pub completed: u32,
    pub cases: Vec<CaseSummary>,
    pub error: Option<String>,
}

pub type Progress = Arc<dyn Fn(&Manifest) + Send + Sync>;

/// 問題を生成する。時間の上書きは公式対戦サーバーと同じ秒単位で反映する。
pub fn generate_case(seed: u64, config: &ServerConfig) -> Result<Case> {
    let mut case = generate_for_players_with_arena_maps(
        seed,
        config.format,
        config.players,
        config.arena_maps,
    )?;
    if let Some(ms) = config.time_limit_ms {
        case.problem.day_seconds.fill(limit_secs(ms)? as u32);
    }
    Ok(case)
}
fn limit_secs(ms: u64) -> Result<u64> {
    Ok(u32::try_from(ms.div_ceil(1000).max(1))?.into())
}

async fn cancelled(cancel: &mut watch::Receiver<bool>) {
    while !*cancel.borrow_and_update() {
        if cancel.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// seedを順に対戦させて履歴を書き出す。CUIとGUIの両方から呼ぶ。
/// 公式対戦サーバーと対戦窓口は1試合ずつしか扱えないため、並列実行はしない。
pub async fn run_batch(
    id: String,
    mut config: ServerConfig,
    root: PathBuf,
    shared: SharedState,
    progress: Progress,
    mut cancel: watch::Receiver<bool>,
) -> Result<Manifest> {
    config.validate()?;
    let directory = root.join(&id);
    tokio::fs::create_dir_all(&directory).await?;
    let mut manifest = Manifest {
        schema_version: SCHEMA_VERSION,
        id,
        status: "running".into(),
        verdict: None,
        total: config.total(),
        completed: 0,
        cases: vec![],
        error: None,
        config,
    };
    write_json(&directory.join("run.json"), &manifest).await?;
    progress(&manifest);
    let result = tokio::select! {
        result = execute(&mut manifest, &directory, &shared, &progress) => result,
        _ = cancelled(&mut cancel) => Err(anyhow!("実行を中断しました")),
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
    progress(&manifest);
    Ok(manifest)
}

async fn execute(
    manifest: &mut Manifest,
    directory: &Path,
    shared: &SharedState,
    progress: &Progress,
) -> Result<()> {
    for index in 0..manifest.total {
        let seed = manifest.config.seed_at(index);
        let match_id = format!("{}-s{seed}", manifest.id);
        let summary = run_case(
            seed,
            &match_id,
            &manifest.config,
            &directory.join(seed.to_string()),
            shared,
        )
        .await
        .with_context(|| format!("seed {seed}"))?;
        manifest.cases.push(summary);
        manifest.completed += 1;
        write_json(&directory.join("run.json"), manifest).await?;
        progress(manifest);
    }
    Ok(())
}

pub fn tokens(players: u32) -> Vec<String> {
    (0..players).map(|i| format!("token-p{i}")).collect()
}

/// 公式対戦サーバーの設定JSON(docs/config.md)。
pub fn official_config(problem: &Problem, tokens: &[String], names: &[String]) -> Value {
    json!({
        "problem": {
            "spots": problem.spots,
            "agentStarts": problem.agents,
            "fuelLimits": problem.fuel_limits,
            "daySteps": problem.day_steps,
            "daySeconds": problem.day_seconds,
            "busyThreshold": problem.busy_threshold,
            "jammedThreshold": problem.jammed_threshold,
            "width": problem.map.width,
            "height": problem.map.height,
            "cells": problem.map.cells,
        },
        "teams": tokens.iter().zip(names).map(|(token, name)| json!({"token": token, "name": name})).collect::<Vec<_>>(),
    })
}

fn free_port() -> Result<u16> {
    Ok(
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?
            .local_addr()?
            .port(),
    )
}

async fn run_case(
    seed: u64,
    match_id: &str,
    config: &ServerConfig,
    directory: &Path,
    shared: &SharedState,
) -> Result<CaseSummary> {
    tokio::fs::create_dir_all(directory).await?;
    let case = generate_case(seed, config)?;
    write_json(&directory.join("problem.json"), &case.problem).await?;
    let names = config.player_names();
    let tokens = tokens(config.players);
    let settings = std::path::absolute(directory.join("config.json"))?;
    write_json(&settings, &official_config(&case.problem, &tokens, &names)).await?;

    let select_ms = config.time_limit_ms.unwrap_or(case.format.parameters().4);
    let program = std::path::absolute(&config.server_program)?;
    ensure!(
        program.is_file(),
        "公式対戦サーバーが見つかりません: {}（公式配布の procon-server を配置してください）",
        program.display()
    );
    let port = free_port()?;
    let log = std::fs::File::create(directory.join("server.log"))?;
    let started_ms = now_ms();
    let mut server = tokio::process::Command::new(&program)
        .arg("-config")
        .arg(&settings)
        .args([
            "-addr",
            &format!("127.0.0.1:{port}"),
            "-kind-deadline",
            &format!("{select_ms}ms"),
            "-match-start-delay",
            &format!("{MATCH_START_DELAY_SECS}s"),
        ])
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("公式対戦サーバーを起動できません: {}", program.display()))?;
    // ここで対戦窓口に公開し、ソルバPCが `/setting` で問題を取得できるようにする(本番の「試合開始」に当たる)。
    let registration = Registration::new(
        shared,
        ActiveMatch {
            id: match_id.into(),
            tokens: tokens.clone(),
            records: vec![TeamRecord::default(); tokens.len()],
            internal_port: port,
            select_started_ms: started_ms,
            select_ends_ms: started_ms + select_ms,
            shortened: config.time_limit_ms.is_some(),
            day_seconds: case.problem.day_seconds.clone(),
            observed: None,
            in_flight: 0,
        },
    );
    let days: u64 = case.problem.day_seconds.iter().map(|&s| u64::from(s)).sum();
    let limit =
        Duration::from_millis(select_ms) + Duration::from_secs(MATCH_START_DELAY_SECS + days + 30);
    wait_for_end(&mut server, port, &tokens[0], limit, directory).await?;
    // 締切直前に受理された提出が記録されるのを待ってから、公式対戦サーバーを止めて記録を取り出す。
    registration.drain(DRAIN_LIMIT).await;
    let _ = server.kill().await;
    let records = registration.finish();

    let (replay, players) = reconstruct(&case, &records, &names)?;
    write_json(&directory.join("replay.json"), &replay).await?;
    for (i, record) in records.iter().enumerate() {
        let text = team_log(
            i,
            &names[i],
            &tokens[i],
            record,
            started_ms,
            case.problem.day_steps.len(),
        );
        tokio::fs::write(directory.join(format!("team-{i}.log")), text).await?;
    }
    Ok(CaseSummary {
        seed: case.seed,
        format: case.format,
        status: combined_verdict(&players).into(),
        score: replay.score.clone(),
        answer_time_ms: replay.answer_time_ms,
        warnings: players.iter().map(|p| p.warnings).sum(),
        error: None,
        players,
    })
}

/// 公式対戦サーバーは結果を返さず、終了後も待ち続ける。`match has ended` を待って終える。
async fn wait_for_end(
    server: &mut tokio::process::Child,
    port: u16,
    token: &str,
    limit: Duration,
    directory: &Path,
) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;
    let url = format!("http://127.0.0.1:{port}/?token={token}");
    let deadline = Instant::now() + limit;
    loop {
        if let Some(status) = server.try_wait()? {
            bail!(
                "公式対戦サーバーが終了しました（{status}）。server.log:\n{}",
                log_tail(&directory.join("server.log"))
            );
        }
        if let Ok(response) = client.get(&url).send().await
            && let Ok(body) = response.bytes().await
            && serde_json::from_slice::<Value>(&body).is_ok_and(|v| {
                v.get("message")
                    .and_then(Value::as_str)
                    .is_some_and(|m| m.contains("ended"))
            })
        {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "試合が予定時刻を過ぎても終了しませんでした"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn log_tail(path: &Path) -> String {
    let text = std::fs::read(path).unwrap_or_default();
    let text = String::from_utf8_lossy(&text[text.len().saturating_sub(2000)..]).into_owned();
    text.trim().to_owned()
}

fn selection(record: &TeamRecord, agents: usize) -> (Vec<u8>, Option<Value>, Option<Failure>, u64) {
    let fallback = vec![0; agents];
    // 対戦窓口は車種選択を1件ずつ中継して記録するので、記録の最後の受理が公式対戦サーバーの最後の受理になる。
    let accepted = record.kinds.iter().rev().find(|s| s.accepted);
    let answer = accepted.or(record.kinds.last()).map(|s| s.body.clone());
    let elapsed = accepted
        .zip(record.setting_at_ms)
        .map_or(0, |(s, at)| s.at_ms.saturating_sub(at));
    let (kinds, failure) = match accepted {
        Some(s) => match validate_kinds(&s.body, agents) {
            Ok(kinds) => (kinds, None),
            Err(e) => (fallback, Some(e)),
        },
        None if record.kinds.is_empty() => (
            fallback,
            Some(Failure::new(
                "timeout",
                "期限内に車種選択が提出されませんでした",
            )),
        ),
        None => (
            fallback,
            Some(Failure::new(
                "invalid_selection",
                format!(
                    "提出した車種選択がすべて却下されました（{}件）",
                    record.kinds.len()
                ),
            )),
        ),
    };
    // 応答を受け取れず採否が分からない車種選択は、観測した状態の車種と一致すれば受理されたものとする。
    // 車種は試合を通して変わらないので、観測できた最も早い日の車を使う(1日目を観測できなかった場合も確かめる)。
    let observed = record
        .day_agents
        .values()
        .next()
        .map(|a| a.iter().map(|a| a.kind).collect::<Vec<u8>>());
    if let Some(observed) = observed.as_ref().filter(|o| **o != kinds)
        && let Some(s) =
            record.kinds.iter().rev().find(|s| {
                s.unknown && validate_kinds(&s.body, agents).is_ok_and(|k| k == *observed)
            })
    {
        let elapsed = record
            .setting_at_ms
            .map_or(0, |at| s.at_ms.saturating_sub(at));
        return (observed.clone(), Some(s.body.clone()), None, elapsed);
    }
    // 採用した車種と違う内容の採否不明の車種選択が最後の受理より後に残り、観測した車種でも確かめられなければ
    // 確定しない(応答を失った選択が最後に受理されたかもしれない)。対戦窓口はチームごとに車種選択を1件ずつ
    // 中継するので、最後の受理より前の採否不明の選択は、受理されていてもその受理で上書きされている。
    let confirmed = observed.as_ref().is_some_and(|o| *o == kinds);
    let after_accepted = record
        .kinds
        .iter()
        .rposition(|s| s.accepted)
        .map_or(0, |i| i + 1);
    let unresolved = record.kinds[after_accepted..]
        .iter()
        .any(|s| s.unknown && validate_kinds(&s.body, agents).is_ok_and(|k| k != kinds));
    if unresolved && !confirmed {
        let failure = Failure::new(
            "unconfirmed_selection",
            "車種選択の採否を確かめられません（公式対戦サーバーの応答が無く、状態の車種でも確かめられないため）。公式の結果と異なる可能性があります",
        );
        return (kinds, answer, Some(failure), elapsed);
    }
    (kinds, answer, failure, elapsed)
}

/// 観測した道路状態が、求めた道路状態と同じ道路のセルをすべて含むか(一部だけの観測は使わない)。
fn same_roads(observed: &[Traffic], computed: &[Traffic]) -> bool {
    observed.len() == computed.len() && observed.iter().zip(computed).all(|(a, b)| a.pos == b.pos)
}

/// 採否不明の提出(`unknown`)がどれも、いま採用する計画(受理が確かな最後の計画、無ければ全車待機)と同じ内容か。
/// 同じなら、どれが実行されても結果は変わらない(応答を失った提出の再送が受理された場合など)。
fn same_as_current(game: &Game, accepted: Option<&Submission>, unknown: &[&Submission]) -> bool {
    let plan_of = |s: &Submission| serde_json::from_value::<Plan>(s.body.clone()).ok();
    let current = match accepted {
        Some(s) => plan_of(s),
        None => Some(game.waiting_plan()),
    };
    current.is_some_and(|current| {
        unknown
            .iter()
            .all(|u| plan_of(u).as_ref() == Some(&current))
    })
}

/// 翌日の状態で観測したもの。道路状態は1チームの試合でだけ使う(自チームの滞在だけで決まるので、同じ位置・燃料に
/// 戻る計画でも道路の滞在の違いで見分けられる)。
#[derive(Clone, Copy)]
struct Next<'a> {
    agents: &'a [Agent],
    traffics: Option<&'a [Traffic]>,
}

/// 計画で1日進めた結果が、翌日の状態で観測したもの(`next`)と一致するか。
fn explains(game: &Game, traffic: &[Traffic], plan: &Plan, next: Next) -> bool {
    let mut trial = game.clone();
    trial
        .play_day_with_traffic(plan.clone(), traffic.to_vec())
        .is_ok()
        && trial.agents == next.agents
        && next.traffics.is_none_or(|t| trial.traffic() == t)
}

/// 日の締切付近で処理され記録した日が推定の提出(`chosen`)を、翌日の状態で観測した車(`next`)で確かめる。
/// その計画で翌日の車を説明でき、実行されなかった場合(同じ日のほかの受理、無ければ全車待機)では説明できない
/// ときだけ、その日に適用されたと確かめられる。
fn estimated_day_confirmed(
    game: &Game,
    traffic: &[Traffic],
    chosen: &Submission,
    others: &[Submission],
    next: Option<Next>,
) -> bool {
    let plan_of = |s: &Submission| serde_json::from_value::<Plan>(s.body.clone()).ok();
    let Some(next) = next else { return false };
    let alternative = others
        .iter()
        .filter(|o| o.accepted && !std::ptr::eq(*o, chosen))
        .max_by_key(|o| o.revision)
        .and_then(plan_of)
        .unwrap_or_else(|| game.waiting_plan());
    plan_of(chosen).is_some_and(|plan| explains(game, traffic, &plan, next))
        && !explains(game, traffic, &alternative, next)
}

/// 応答を受け取れず採否が分からない計画(`unknown`)を、翌日の状態で観測した車(`next`)で確かめる。
/// 採用する計画と、確かめられなかったか(true)を返す。いま採用する計画(受理が確かな最後の計画、無ければ全車待機)と
/// 採否不明の計画のうち、翌日の車を説明できる計画がちょうど1つ(同じ内容の再送は1つと数える)ならそれを採用する。
/// 内容の違う計画が複数説明できる(同じ位置・燃料に戻るが途中で回った店が違うなど)ときは、どれが実行されたか
/// 分からないので確かめられないとする。どれも説明できなければ、いまの計画のままとする。
fn reconcile<'a>(
    game: &Game,
    traffic: &[Traffic],
    accepted: Option<&'a Submission>,
    unknown: &[&'a Submission],
    next: Next,
) -> (Option<&'a Submission>, bool) {
    let explains = |plan: &Plan| explains(game, traffic, plan, next);
    let current = match accepted {
        Some(s) => serde_json::from_value::<Plan>(s.body.clone()).ok(),
        None => Some(game.waiting_plan()),
    };
    let mut candidates = unknown.to_vec();
    candidates.sort_by_key(|s| std::cmp::Reverse(s.at_ms));
    let mut explaining: Vec<(Option<&'a Submission>, Plan)> = current
        .filter(|plan| explains(plan))
        .map(|plan| (accepted, plan))
        .into_iter()
        .collect();
    for s in candidates {
        if let Ok(plan) = serde_json::from_value::<Plan>(s.body.clone())
            && explains(&plan)
            && explaining.iter().all(|(_, other)| *other != plan)
        {
            explaining.push((Some(s), plan));
        }
    }
    match explaining.as_slice() {
        [] => (accepted, false),
        [(chosen, _)] => (*chosen, false),
        [(chosen, _), ..] => (*chosen, true),
    }
}

/// 提出記録から共通エンジンで対戦を再現し、対戦リプレイ(schema 2)と各チームの成績を作る。
/// 公式対戦サーバーと同じく、各日は最後に受理された計画(revision が最大の受理)を採用し、無ければ全車待機とする。
pub fn reconstruct(
    case: &Case,
    records: &[TeamRecord],
    names: &[String],
) -> Result<(Replay, Vec<PlayerSummary>)> {
    let problem = &case.problem;
    let empty = TeamRecord::default();
    let records: Vec<&TeamRecord> = (0..names.len())
        .map(|i| records.get(i).unwrap_or(&empty))
        .collect();
    let mut games = Vec::new();
    let mut replays = Vec::new();
    for record in &records {
        let (kinds, answer, failure, elapsed) = selection(record, problem.agents.len());
        games.push(Game::new(problem.clone(), &kinds));
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
    // 採否・適用日を確かめられない日があったか。以降の道路状態は、観測できなければ公式と異なるかもしれない。
    let mut tainted = false;
    // チームごとに、前の日に記録した日が推定の提出を確かめられなかったか(翌日に適用されたかもしれない)。
    let mut carried = vec![false; records.len()];
    // チームごとに、確かめられない日があり再現が公式とずれているかもしれないか。獲得した系列・玉数は状態に
    // 含まれず観測できないので、後の日に観測した車が再現と一致しても解除しない。
    let mut diverged = vec![false; records.len()];
    for day in 0..problem.day_steps.len() {
        // 道路状態は、各チームの状態で観測した値(公式対戦サーバーが全チームの滞在から求めた値)があればそれを使う。
        // 観測が無い日は、再現した滞在から求める。
        let computed = traffic_for(problem, &history, names.len() as u32);
        let observed = records
            .iter()
            .find_map(|r| {
                r.day_traffics
                    .get(&day)
                    .filter(|o| same_roads(o, &computed))
            })
            .cloned();
        let traffic_unconfirmed = tainted && observed.is_none();
        let traffic = observed.unwrap_or(computed);
        let mut day_unconfirmed = false;
        for (id, record) in records.iter().enumerate() {
            let game = &mut games[id];
            let inherited = diverged[id];
            let submissions: &[Submission] = record.plans.get(&day).map_or(&[], Vec::as_slice);
            // 並行して提出すると記録の順(応答が返った順)と受理の順がずれるため、revision で最後の受理を選ぶ。
            let accepted = submissions
                .iter()
                .filter(|s| s.accepted)
                .max_by_key(|s| s.revision);
            let unknown: Vec<&Submission> = submissions.iter().filter(|s| s.unknown).collect();
            // 採否不明の提出を確かめられない場合は、公式の結果と異なる可能性があるので確定した結果として扱わない。
            // 翌日の状態が無い(最終日など)ときは確かめられない。採否不明の提出は revision が無く、並行提出では
            // 到着の順と処理の順が一致しないため、受理が確かな提出との前後も決められない。
            let next = record.day_agents.get(&(day + 1)).map(|agents| Next {
                agents,
                traffics: (names.len() == 1)
                    .then(|| record.day_traffics.get(&(day + 1)))
                    .flatten()
                    .filter(|t| same_roads(t, &traffic))
                    .map(Vec::as_slice),
            });
            let (accepted, unconfirmed) = match next {
                _ if unknown.is_empty() => (accepted, false),
                Some(next) => reconcile(game, &traffic, accepted, &unknown, next),
                None => (accepted, !same_as_current(game, accepted, &unknown)),
            };
            // 記録した日が推定の提出(受理・採否不明とも)は、公式対戦サーバーが翌日の計画として適用したかもしれない。
            // 採用した計画としてこの日に適用されたと翌日の状態で確かめられたもの以外があれば、この日と翌日を
            // 確かめられないとする。
            let confirmed = accepted.filter(|s| {
                s.uncertain_day && estimated_day_confirmed(game, &traffic, s, submissions, next)
            });
            let open = submissions.iter().any(|s| {
                s.uncertain_day
                    && (s.accepted || s.unknown)
                    && !confirmed.is_some_and(|c| std::ptr::eq(c, s))
            });
            let unconfirmed = unconfirmed || open || carried[id];
            carried[id] = open;
            let mut played = None;
            let mut elapsed = 0;
            let failure = match accepted {
                Some(s) => match serde_json::from_value::<Plan>(s.body.clone())
                    .map_err(|e| Failure::new("invalid_plan", e.to_string()))
                    .and_then(|plan| game.play_day_with_traffic(plan, traffic.clone()))
                {
                    Ok(d) => {
                        played = Some(d);
                        elapsed = record
                            .day_seen_ms
                            .get(&day)
                            .map_or(0, |&seen| s.at_ms.saturating_sub(seen));
                        None
                    }
                    Err(e) => Some(Failure {
                        message: format!(
                            "公式対戦サーバーは受理しましたが、再現で不正と判定しました: {}",
                            e.message
                        ),
                        ..e
                    }),
                },
                None if submissions.is_empty() => Some(Failure::new(
                    "timeout",
                    "期限内に行動計画が提出されませんでした",
                )),
                None if unknown.is_empty() => Some(Failure::new(
                    "invalid_plan",
                    format!(
                        "提出した行動計画がすべて却下されました（{}件）",
                        submissions.len()
                    ),
                )),
                None => Some(Failure::new(
                    "invalid_plan",
                    format!(
                        "提出した行動計画の受理を確認できませんでした（{}件、うち公式対戦サーバーの応答なし{}件）",
                        submissions.len(),
                        unknown.len()
                    ),
                )),
            };
            day_unconfirmed |= unconfirmed || inherited;
            diverged[id] |= unconfirmed;
            let failure = if unconfirmed {
                Some(Failure::new(
                    "unconfirmed_plan",
                    "提出の採否または適用日を確かめられません（公式対戦サーバーの応答が無い・締切付近で処理された提出を、翌日の状態で確かめられないため）。公式の結果と異なる可能性があります",
                ))
            } else if inherited && failure.is_none() {
                Some(Failure::new(
                    "unconfirmed_plan",
                    "前の日に採否を確かめられない提出があり、この日の車の状態を確かめられません（公式の結果と異なる可能性があります）",
                ))
            } else if traffic_unconfirmed && failure.is_none() {
                Some(Failure::new(
                    "unconfirmed_plan",
                    "前の日に採否を確かめられない提出があり、この日の道路状態を確かめられません（公式の結果と異なる可能性があります）",
                ))
            } else {
                failure
            };
            let mut result = match played {
                Some(d) => {
                    replays[id].answer_time_ms += elapsed;
                    d
                }
                None => game
                    .play_day_with_traffic(game.waiting_plan(), traffic.clone())
                    .map_err(|e| anyhow!("待機への置き換えに失敗: {}", e.message))?,
            };
            result.submitted_answer = accepted.or(submissions.last()).map(|s| s.body.clone());
            result.failure = failure;
            result.elapsed_ms = elapsed;
            replays[id].days.push(result);
            replays[id].score = game.score.clone();
        }
        tainted |= day_unconfirmed;
        history.push(total_stays(&games, problem.map.len()));
        if history.len() > 2 {
            history.remove(0);
        }
    }
    let players = summaries(&replays, names);
    let mut replay = replays.remove(0);
    replay.schema_version = 2;
    replay.opponents = replays;
    Ok((replay, players))
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

/// GUIの「ログ」で表示する提出記録。時刻は試合の登録からの経過時間。
fn team_log(
    id: usize,
    name: &str,
    token: &str,
    record: &TeamRecord,
    started_ms: u64,
    days: usize,
) -> String {
    let at = |ms: u64| format!("+{:.3}s", ms.saturating_sub(started_ms) as f64 / 1000.0);
    let result = |s: &Submission| {
        let text = if s.unknown {
            "採否不明（公式対戦サーバーの応答なし）"
        } else if s.accepted {
            "受理"
        } else {
            "却下"
        };
        let note = if s.uncertain_day {
            " · 日の締切付近で処理され、記録した日は推定"
        } else {
            ""
        };
        match s.revision {
            Some(r) => format!("{text}（HTTP {} / revision {r}{note}）", s.status),
            None => format!("{text}（HTTP {}{note}）", s.status),
        }
    };
    let mut out =
        format!("チーム{id} · {name} · {token}\n時刻は試合の登録からの経過時間です。\n\n");
    match record.setting_at_ms {
        Some(ms) => writeln!(out, "問題の取得  {}", at(ms)),
        None => writeln!(out, "問題の取得  なし（ソルバPCが接続していません）"),
    }
    .unwrap();
    if record.kinds.is_empty() {
        out.push_str("車種選択    提出なし\n");
    }
    for s in &record.kinds {
        let mut body = s.body.to_string();
        if body.chars().count() > 120 {
            body = body.chars().take(120).chain("…".chars()).collect();
        }
        writeln!(out, "車種選択    {}  {}  {body}", at(s.at_ms), result(s)).unwrap();
    }
    for day in 0..days {
        let seen = record.day_seen_ms.get(&day).copied();
        writeln!(
            out,
            "\n{}日目  状態の取得 {}",
            day + 1,
            seen.map_or("なし".into(), at)
        )
        .unwrap();
        let submissions = record.plans.get(&day).map_or(&[][..], Vec::as_slice);
        if submissions.is_empty() {
            out.push_str("  提出なし\n");
        }
        for s in submissions {
            let answer = seen.map_or(String::new(), |ms| {
                format!("  回答 {} ms", s.at_ms.saturating_sub(ms))
            });
            writeln!(out, "  提出 {}  {}{answer}", at(s.at_ms), result(s)).unwrap();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn submission(body: Value, at_ms: u64, accepted: bool) -> Submission {
        Submission {
            body,
            at_ms,
            status: 200,
            revision: Some(if accepted { 1 } else { -1 }),
            accepted,
            uncertain_day: false,
            unknown: false,
        }
    }
    fn config(players: u32) -> ServerConfig {
        ServerConfig {
            count: None,
            seed: Some("0".into()),
            players,
            ..ServerConfig::default()
        }
    }

    #[test]
    fn history_arena_defaults_preserve_old_runs_and_explicit_settings() {
        let new_config: ServerConfig = serde_json::from_str("{}").unwrap();
        assert!(new_config.arena_maps);
        for setting in [None, Some(false), Some(true)] {
            let mut saved = serde_json::to_value(config(16)).unwrap();
            saved.as_object_mut().unwrap().remove("arenaMaps");
            if let Some(enabled) = setting {
                saved["arenaMaps"] = enabled.into();
            }
            let mut manifest: Manifest = serde_json::from_value(json!({
                "schemaVersion": SCHEMA_VERSION, "id": "101-1", "status": "running",
                "config": saved, "total": 1, "completed": 0,
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

    #[test]
    fn reconstruction_uses_the_last_accepted_plan_and_reports_missing_answers() {
        let config = config(3);
        let case = generate_case(0, &config).unwrap();
        let problem = &case.problem;
        let agents = problem.agents.len();
        let wait = |d: usize| json!(vec![vec![-(problem.day_steps[d] as i64)]; agents]);
        let mut active = TeamRecord {
            setting_at_ms: Some(1_000),
            kinds: vec![submission(json!(vec![1; agents]), 1_500, true)],
            ..TeamRecord::default()
        };
        for d in 0..problem.day_steps.len() {
            active.day_seen_ms.insert(d, 10_000 * (d as u64 + 1));
            active.plans.insert(
                d,
                vec![
                    submission(wait(d), 10_000 * (d as u64 + 1) + 200, true),
                    submission(json!([[0]]), 10_000 * (d as u64 + 1) + 300, false),
                ],
            );
        }
        let rejected = TeamRecord {
            kinds: vec![Submission {
                status: 403,
                revision: None,
                ..submission(json!(vec![0; agents]), 2_000, false)
            }],
            plans: [(0, vec![submission(json!([[0]]), 12_000, false)])].into(),
            ..TeamRecord::default()
        };
        let names = config.player_names();
        let (replay, players) = reconstruct(&case, &[active, rejected], &names).unwrap();
        assert_eq!(replay.schema_version, 2);
        assert_eq!(replay.opponents.len(), 2);
        assert_eq!(replay.kinds, vec![1; agents]);
        assert_eq!(replay.selection_ms, 500);
        assert!(replay.days.iter().all(|d| d.failure.is_none()));
        assert_eq!(replay.answer_time_ms, 200 * problem.day_steps.len() as u64);
        assert_eq!(replay.days[0].submitted_answer, Some(wait(0)));
        assert_eq!(players[0].status, "AC");
        let rejected = &replay.opponents[0];
        assert_eq!(
            rejected.selection_failure.as_ref().unwrap().kind,
            "invalid_selection"
        );
        assert_eq!(
            rejected.days[0].failure.as_ref().unwrap().kind,
            "invalid_plan"
        );
        assert_eq!(rejected.days[1].failure.as_ref().unwrap().kind, "timeout");
        assert_eq!(players[1].status, "TLE");
        // 接続していないチームも全車待機で最後まで再生できる。
        let absent = &replay.opponents[1];
        assert_eq!(absent.days.len(), problem.day_steps.len());
        assert_eq!(absent.selection_failure.as_ref().unwrap().kind, "timeout");
        assert_eq!(
            players.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["チーム0", "チーム1", "チーム2"]
        );
    }

    #[test]
    fn parallel_submissions_use_the_accepted_plan_with_the_highest_revision() {
        let config = config(1);
        let case = generate_case(0, &config).unwrap();
        let problem = &case.problem;
        let agents = problem.agents.len();
        let wait = |d: usize| json!(vec![vec![-(problem.day_steps[d] as i64)]; agents]);
        let mut record = TeamRecord {
            kinds: vec![submission(json!(vec![0; agents]), 0, true)],
            ..TeamRecord::default()
        };
        for d in 0..problem.day_steps.len() {
            record.plans.insert(d, vec![submission(wait(d), 0, true)]);
        }
        // 後から受理された計画(revision 7)の応答が先に返り、記録では前に並んだ場合。
        let later = Submission {
            revision: Some(7),
            ..submission(json!([[0]]), 100, true)
        };
        let earlier = Submission {
            revision: Some(3),
            ..submission(wait(0), 200, true)
        };
        record.plans.insert(0, vec![later, earlier]);
        let (replay, _) = reconstruct(&case, &[record], &config.player_names()).unwrap();
        assert_eq!(replay.days[0].submitted_answer, Some(json!([[0]])));
    }

    #[test]
    fn plans_whose_response_was_lost_are_confirmed_by_the_next_state() {
        let config = config(1);
        let case = generate_case(0, &config).unwrap();
        let problem = &case.problem;
        let agents = problem.agents.len();
        let game = Game::new(problem.clone(), &vec![0; agents]);
        let traffic = traffic_for(problem, &[], 1);
        // 1台だけ隣のセルへ動いて残りを待つ計画と、そのときの翌日の車。
        let (plan, moved) = (0..6)
            .flat_map(|direction| (1..=4).map(move |cost| (direction, cost)))
            .find_map(|(direction, cost)| {
                let mut plan = game.waiting_plan();
                plan[0] = vec![direction, -(i64::from(problem.day_steps[0]) - cost)];
                let mut trial = game.clone();
                (trial
                    .play_day_with_traffic(plan.clone(), traffic.clone())
                    .is_ok()
                    && trial.agents != game.agents)
                    .then(|| (json!(plan), trial.agents))
            })
            .unwrap();
        let record = |next: Vec<Agent>| {
            let lost = Submission {
                status: 502,
                revision: None,
                unknown: true,
                ..submission(plan.clone(), 100, false)
            };
            TeamRecord {
                kinds: vec![submission(json!(vec![0; agents]), 0, true)],
                plans: [(0, vec![lost])].into(),
                day_agents: [(1, next)].into(),
                ..TeamRecord::default()
            }
        };
        // 翌日の車が計画どおりなら、応答が無くても受理されたものとして再現する。
        let (replay, _) = reconstruct(&case, &[record(moved)], &config.player_names()).unwrap();
        assert!(replay.days[0].failure.is_none());
        assert_eq!(replay.days[0].submitted_answer, Some(plan.clone()));
        // 翌日の車が待機のままなら、受理を確認できない。
        let (replay, _) = reconstruct(
            &case,
            &[record(game.agents.clone())],
            &config.player_names(),
        )
        .unwrap();
        let failure = replay.days[0].failure.as_ref().unwrap();
        assert!(failure.message.contains("応答なし1件"));
        // 翌日の状態が無ければ(最終日など)確かめられないので、確定した結果として扱わない。
        let unseen = TeamRecord {
            day_agents: Default::default(),
            ..record(game.agents.clone())
        };
        let (replay, players) =
            reconstruct(&case, std::slice::from_ref(&unseen), &config.player_names()).unwrap();
        assert_eq!(
            replay.days[0].failure.as_ref().unwrap().kind,
            "unconfirmed_plan"
        );
        assert_ne!(players[0].status, "AC");
        // 受理が確かな提出より先に届いた採否不明の提出も、処理の順は分からないので確かめられない。
        let mut both = unseen;
        let known = submission(
            json!(vec![vec![-i64::from(problem.day_steps[0])]; agents]),
            200,
            true,
        );
        both.plans.get_mut(&0).unwrap().push(known);
        let (replay, _) = reconstruct(&case, &[both], &config.player_names()).unwrap();
        assert_eq!(
            replay.days[0].failure.as_ref().unwrap().kind,
            "unconfirmed_plan"
        );
    }

    #[test]
    fn plans_on_an_estimated_day_are_confirmed_only_by_the_next_state() {
        let config = config(1);
        let case = generate_case(0, &config).unwrap();
        let problem = &case.problem;
        let agents = problem.agents.len();
        let game = Game::new(problem.clone(), &vec![0; agents]);
        let traffic = traffic_for(problem, &[], 1);
        let (plan, moved) = (0..6)
            .flat_map(|direction| (1..=4).map(move |cost| (direction, cost)))
            .find_map(|(direction, cost)| {
                let mut plan = game.waiting_plan();
                plan[0] = vec![direction, -(i64::from(problem.day_steps[0]) - cost)];
                let mut trial = game.clone();
                (trial
                    .play_day_with_traffic(plan.clone(), traffic.clone())
                    .is_ok()
                    && trial.agents != game.agents)
                    .then(|| (json!(plan), trial.agents))
            })
            .unwrap();
        let record = |day_agents: std::collections::BTreeMap<usize, Vec<Agent>>| TeamRecord {
            kinds: vec![submission(json!(vec![0; agents]), 0, true)],
            plans: [(
                0,
                vec![Submission {
                    uncertain_day: true,
                    ..submission(plan.clone(), 100, true)
                }],
            )]
            .into(),
            day_agents,
            ..TeamRecord::default()
        };
        // 翌日の車が計画どおりなら、その日に適用されたと確かめられる。
        let (replay, _) = reconstruct(
            &case,
            &[record([(1, moved)].into())],
            &config.player_names(),
        )
        .unwrap();
        assert!(replay.days[0].failure.is_none());
        // 翌日の状態が無ければ確かめられない。
        let (replay, _) =
            reconstruct(&case, &[record(Default::default())], &config.player_names()).unwrap();
        assert_eq!(
            replay.days[0].failure.as_ref().unwrap().kind,
            "unconfirmed_plan"
        );
    }

    #[test]
    fn lost_plans_on_an_estimated_day_leave_the_next_day_unconfirmed() {
        let config = config(1);
        let case = generate_case(0, &config).unwrap();
        let problem = &case.problem;
        let agents = problem.agents.len();
        let game = Game::new(problem.clone(), &vec![0; agents]);
        let wait = |d: usize| json!(vec![vec![-(problem.day_steps[d] as i64)]; agents]);
        // 応答が無く、締切付近で処理されたので1日目に記録した提出。翌日の車は待機のまま。
        let lost = Submission {
            status: 502,
            revision: None,
            unknown: true,
            uncertain_day: true,
            ..submission(wait(0), 100, false)
        };
        let record = TeamRecord {
            kinds: vec![submission(json!(vec![0; agents]), 0, true)],
            plans: [(0, vec![lost]), (1, vec![submission(wait(1), 100, true)])].into(),
            day_agents: [(1, game.agents.clone()), (2, game.agents.clone())].into(),
            ..TeamRecord::default()
        };
        // 2日目に適用されたかもしれないので、1日目も2日目も確定しない。
        let (replay, _) = reconstruct(&case, &[record], &config.player_names()).unwrap();
        assert_eq!(
            replay.days[0].failure.as_ref().unwrap().kind,
            "unconfirmed_plan"
        );
        assert_eq!(
            replay.days[1].failure.as_ref().unwrap().kind,
            "unconfirmed_plan"
        );
    }

    /// 1日目に1台だけ隣のセルへ動いて残りを待つ、全車待機とは違う計画。
    fn moving_plan(problem: &Problem) -> Value {
        let game = Game::new(problem.clone(), &vec![0; problem.agents.len()]);
        let traffic = traffic_for(problem, &[], 1);
        (0..6)
            .flat_map(|direction| (1..=4).map(move |cost| (direction, cost)))
            .find_map(|(direction, cost)| {
                let mut plan = game.waiting_plan();
                plan[0] = vec![direction, -(i64::from(problem.day_steps[0]) - cost)];
                let mut trial = game.clone();
                trial
                    .play_day_with_traffic(plan.clone(), traffic.clone())
                    .is_ok()
                    .then(|| json!(plan))
            })
            .unwrap()
    }

    #[test]
    fn later_days_stay_unconfirmed_after_an_unconfirmed_day() {
        let config = config(1);
        let case = generate_case(0, &config).unwrap();
        let problem = &case.problem;
        let agents = problem.agents.len();
        let wait = |d: usize| json!(vec![vec![-(problem.day_steps[d] as i64)]; agents]);
        let lost = Submission {
            status: 502,
            revision: None,
            unknown: true,
            ..submission(moving_plan(problem), 100, false)
        };
        let record = TeamRecord {
            kinds: vec![submission(json!(vec![0; agents]), 0, true)],
            plans: [
                (0, vec![lost]),
                (1, vec![submission(wait(1), 100, true)]),
                (2, vec![submission(wait(2), 100, true)]),
            ]
            .into(),
            // 3日目に観測した車は再現と一致するが、獲得した系列・玉数は観測できないので確定しない。
            day_agents: [(2, Game::new(problem.clone(), &vec![0; agents]).agents)].into(),
            day_traffics: (0..3).map(|d| (d, traffic_for(problem, &[], 1))).collect(),
            ..TeamRecord::default()
        };
        let (replay, _) = reconstruct(&case, &[record], &config.player_names()).unwrap();
        for day in 0..3 {
            assert_eq!(
                replay.days[day].failure.as_ref().unwrap().kind,
                "unconfirmed_plan"
            );
        }
    }

    #[test]
    fn a_retried_final_plan_identical_to_the_accepted_one_is_confirmed() {
        let config = config(1);
        let case = generate_case(0, &config).unwrap();
        let problem = &case.problem;
        let agents = problem.agents.len();
        let wait = |d: usize| json!(vec![vec![-(problem.day_steps[d] as i64)]; agents]);
        let last = problem.day_steps.len() - 1;
        let mut record = TeamRecord {
            kinds: vec![submission(json!(vec![0; agents]), 0, true)],
            ..TeamRecord::default()
        };
        for d in 0..=last {
            record.plans.insert(d, vec![submission(wait(d), 100, true)]);
        }
        // 最終日の提出の応答が失われ、同じ内容の再送が受理された。
        record.plans.get_mut(&last).unwrap().insert(
            0,
            Submission {
                status: 502,
                revision: None,
                unknown: true,
                ..submission(wait(last), 50, false)
            },
        );
        let (replay, players) = reconstruct(&case, &[record], &config.player_names()).unwrap();
        assert!(replay.days[last].failure.is_none());
        assert_eq!(players[0].status, "AC");
    }

    #[test]
    fn lost_vehicle_selections_that_cannot_be_observed_are_unconfirmed() {
        let config = config(1);
        let case = generate_case(0, &config).unwrap();
        let agents = case.problem.agents.len();
        let accepted = vec![0; agents];
        let mut other = accepted.clone();
        other[0] = 1;
        let lost = |kinds: &Vec<u8>| Submission {
            status: 502,
            revision: None,
            unknown: true,
            ..submission(json!(kinds), 200, false)
        };
        let record = |lost_kinds: &Vec<u8>, day_agents| TeamRecord {
            kinds: vec![submission(json!(accepted), 100, true), lost(lost_kinds)],
            day_agents,
            ..TeamRecord::default()
        };
        // 状態を観測できなければ、後から応答を失った違う選択が受理されたかもしれない。
        let (_, _, failure, _) = selection(&record(&other, Default::default()), agents);
        assert_eq!(failure.unwrap().kind, "unconfirmed_selection");
        // 観測した車種が受理済みの選択と一致すれば確定する。
        let seen = Game::new(case.problem.clone(), &accepted).agents;
        let (kinds, _, failure, _) = selection(&record(&other, [(0, seen)].into()), agents);
        assert!(failure.is_none());
        assert_eq!(kinds, accepted);
        // 同じ内容の再送なら、どれが受理されても同じ。
        let (_, _, failure, _) = selection(&record(&accepted, Default::default()), agents);
        assert!(failure.is_none());
        // 応答を失った違う選択の後に受理された選択があれば、その受理で上書きされている。
        let superseded = TeamRecord {
            kinds: vec![lost(&other), submission(json!(accepted), 300, true)],
            ..TeamRecord::default()
        };
        let (kinds, _, failure, _) = selection(&superseded, agents);
        assert!(failure.is_none());
        assert_eq!(kinds, accepted);
    }

    #[test]
    fn lost_vehicle_selections_are_confirmed_by_the_earliest_state() {
        let config = config(1);
        let case = generate_case(0, &config).unwrap();
        let agents = case.problem.agents.len();
        let mut chosen = vec![0; agents];
        chosen[0] = 1;
        let lost = Submission {
            status: 502,
            revision: None,
            unknown: true,
            ..submission(json!(chosen), 100, false)
        };
        // 1日目は観測できず、3日目の状態で車種を観測した。
        let observed = Game::new(case.problem.clone(), &chosen).agents;
        let record = TeamRecord {
            kinds: vec![lost],
            day_agents: [(2, observed)].into(),
            ..TeamRecord::default()
        };
        let (kinds, _, failure, _) = selection(&record, agents);
        assert_eq!(kinds, chosen);
        assert!(failure.is_none());
    }

    #[test]
    fn observed_traffic_is_used_and_uncertainty_carries_over_without_it() {
        let config = config(2);
        let case = generate_case(0, &config).unwrap();
        let problem = &case.problem;
        let agents = problem.agents.len();
        let wait = |d: usize| json!(vec![vec![-(problem.day_steps[d] as i64)]; agents]);
        let mut traffic = traffic_for(problem, &[], 2);
        traffic[0].status = 2;
        // チーム0は1日目の採否を確かめられない。チーム1はすべて確かに受理された。
        let lost = Submission {
            status: 502,
            revision: None,
            unknown: true,
            ..submission(moving_plan(problem), 100, false)
        };
        let uncertain = TeamRecord {
            kinds: vec![submission(json!(vec![0; agents]), 0, true)],
            plans: [(0, vec![lost]), (1, vec![submission(wait(1), 100, true)])].into(),
            ..TeamRecord::default()
        };
        let certain = |day_traffics: std::collections::BTreeMap<usize, Vec<Traffic>>| TeamRecord {
            kinds: vec![submission(json!(vec![0; agents]), 0, true)],
            plans: [
                (0, vec![submission(wait(0), 100, true)]),
                (1, vec![submission(wait(1), 100, true)]),
            ]
            .into(),
            day_traffics,
            ..TeamRecord::default()
        };
        // チーム0の滞在は以降の道路状態に影響しうるので、観測の無い2日目はチーム1も確かめられない。
        let names = config.player_names();
        let records = [uncertain.clone(), certain(Default::default())];
        let (replay, _) = reconstruct(&case, &records, &names).unwrap();
        let other = &replay.opponents[0];
        assert!(other.days[0].failure.is_none());
        assert!(
            other.days[1]
                .failure
                .as_ref()
                .unwrap()
                .message
                .contains("道路状態")
        );
        // 2日目の道路状態を観測していれば、それを使ってチーム1は確定する。
        let records = [uncertain, certain([(1, traffic.clone())].into())];
        let (replay, _) = reconstruct(&case, &records, &names).unwrap();
        let other = &replay.opponents[0];
        assert!(other.days[1].failure.is_none());
        assert_eq!(other.days[1].traffics, traffic);
    }

    #[test]
    fn a_single_team_round_trip_is_confirmed_by_the_observed_roads() {
        let config = config(1);
        // 補給車が道路を行って戻り、全車待機と同じ位置・燃料で終わるが、道路の滞在が変わる計画を探す。
        let (case, kinds, plan, trip) = (0..40u64)
            .find_map(|seed| {
                let case = generate_case(seed, &config).ok()?;
                let problem = &case.problem;
                let mut kinds = vec![0; problem.agents.len()];
                kinds[0] = 1;
                let game = Game::new(problem.clone(), &kinds);
                let traffic = traffic_for(problem, &[], 1);
                let steps = i64::from(problem.day_steps[0]);
                let mut waited = game.clone();
                waited
                    .play_day_with_traffic(game.waiting_plan(), traffic.clone())
                    .ok()?;
                let found = (0..6)
                    .flat_map(|d| (1..=4).flat_map(move |a| (1..=4).map(move |b| (d, a, b))))
                    .find_map(|(d, a, b)| {
                        let mut plan = game.waiting_plan();
                        plan[0] = vec![d, (d + 3) % 6, -(steps - a - b)];
                        let mut trial = game.clone();
                        (trial
                            .play_day_with_traffic(plan.clone(), traffic.clone())
                            .is_ok()
                            && trial.agents == waited.agents
                            && trial.traffic() != waited.traffic())
                        .then_some((plan, trial))
                    })?;
                Some((case.clone(), kinds, found.0, found.1))
            })
            .expect("道路の滞在が変わる往復が見つかりません");
        let problem = &case.problem;
        let lost = Submission {
            status: 502,
            revision: None,
            unknown: true,
            ..submission(json!(plan), 100, false)
        };
        let record = TeamRecord {
            kinds: vec![submission(json!(kinds), 0, true)],
            plans: [(0, vec![lost])].into(),
            day_agents: [(1, trip.agents.clone())].into(),
            day_traffics: [(1, trip.traffic())].into(),
            ..TeamRecord::default()
        };
        // 車は待機と同じでも、翌日の道路状態が往復の場合と一致するので、往復が実行されたと確かめられる。
        let (replay, _) = reconstruct(&case, &[record], &config.player_names()).unwrap();
        assert!(replay.days[0].failure.is_none());
        assert_eq!(replay.days[0].submitted_answer, Some(json!(plan)));
        assert_eq!(replay.days.len(), problem.day_steps.len());
    }

    #[test]
    fn plans_that_end_in_the_same_state_are_left_unconfirmed() {
        let config = config(1);
        let case = generate_case(0, &config).unwrap();
        let problem = &case.problem;
        let agents = problem.agents.len();
        // 補給車(燃料が減らない)が隣のセルへ行って戻る計画は、全車待機と同じ位置・燃料で終わる。
        let mut kinds = vec![0; agents];
        kinds[0] = 1;
        let game = Game::new(problem.clone(), &kinds);
        let traffic = traffic_for(problem, &[], 1);
        let steps = i64::from(problem.day_steps[0]);
        let mut waited = game.clone();
        waited
            .play_day_with_traffic(game.waiting_plan(), traffic.clone())
            .unwrap();
        let round_trip = (0..6)
            .flat_map(|d| (1..=4).flat_map(move |a| (1..=4).map(move |b| (d, a, b))))
            .find_map(|(d, a, b)| {
                let mut plan = game.waiting_plan();
                plan[0] = vec![d, (d + 3) % 6, -(steps - a - b)];
                let mut trial = game.clone();
                (trial
                    .play_day_with_traffic(plan.clone(), traffic.clone())
                    .is_ok()
                    && trial.agents == waited.agents)
                    .then_some(plan)
            })
            .unwrap();
        let wait = json!(game.waiting_plan());
        let record = TeamRecord {
            kinds: vec![submission(json!(kinds), 0, true)],
            plans: [(
                0,
                vec![
                    submission(wait, 100, true),
                    Submission {
                        status: 502,
                        revision: None,
                        unknown: true,
                        ..submission(json!(round_trip), 200, false)
                    },
                ],
            )]
            .into(),
            day_agents: [(1, waited.agents.clone())].into(),
            ..TeamRecord::default()
        };
        let (replay, _) = reconstruct(&case, &[record], &config.player_names()).unwrap();
        assert_eq!(
            replay.days[0].failure.as_ref().unwrap().kind,
            "unconfirmed_plan"
        );
    }

    #[test]
    fn plans_the_engine_rejects_fall_back_to_waiting_with_a_warning() {
        let config = config(1);
        let case = generate_case(0, &config).unwrap();
        let agents = case.problem.agents.len();
        let mut record = TeamRecord {
            kinds: vec![submission(json!(vec![0; agents]), 0, true)],
            ..TeamRecord::default()
        };
        for (d, &steps) in case.problem.day_steps.iter().enumerate() {
            let plan = if d == 0 {
                json!([[0]])
            } else {
                json!(vec![vec![-(steps as i64)]; agents])
            };
            record.plans.insert(d, vec![submission(plan, 0, true)]);
        }
        let (replay, players) = reconstruct(&case, &[record], &config.player_names()).unwrap();
        let failure = replay.days[0].failure.as_ref().unwrap();
        assert_eq!(failure.kind, "invalid_plan");
        assert!(
            failure
                .message
                .starts_with("公式対戦サーバーは受理しましたが")
        );
        assert_eq!(players[0].status, "WA");
    }

    #[test]
    fn configuration_is_validated_like_the_testers() {
        let mut ok = config(2);
        ok.team_names = vec!["  A  ".into()];
        ok.validate().unwrap();
        assert_eq!(ok.player_names(), ["A", "チーム1"]);
        for broken in [
            ServerConfig {
                count: Some(1),
                ..config(2)
            },
            ServerConfig {
                seed: Some("x".into()),
                ..config(2)
            },
            config(0),
            config(MAX_PLAYERS + 1),
            ServerConfig {
                team_names: vec!["a".into(); 3],
                ..config(2)
            },
            ServerConfig {
                time_limit_ms: Some(0),
                ..config(2)
            },
        ] {
            assert!(broken.clone().validate().is_err(), "{broken:?}");
        }
    }

    #[test]
    fn arena_map_setting_is_saved_and_controls_the_generated_problem() {
        let legacy: ServerConfig = serde_json::from_str("{}").unwrap();
        assert!(legacy.arena_maps);
        let excluded: ServerConfig = serde_json::from_str(r#"{"arenaMaps":false}"#).unwrap();
        assert!(!excluded.arena_maps);
        assert_eq!(serde_json::to_value(&excluded).unwrap()["arenaMaps"], false);
        for seed in [3, 7, 11] {
            let included = generate_case(seed, &config(5)).unwrap();
            let procedural = generate_case(
                seed,
                &ServerConfig {
                    arena_maps: false,
                    ..config(5)
                },
            )
            .unwrap();
            assert_ne!(included.problem.map, procedural.problem.map);
            assert_eq!(procedural.problem.players, 5);
        }
    }

    #[test]
    fn time_limit_is_rounded_up_to_whole_seconds() {
        let config = ServerConfig {
            time_limit_ms: Some(1_500),
            ..config(2)
        };
        let case = generate_case(3, &config).unwrap();
        assert!(case.problem.day_seconds.iter().all(|&s| s == 2));
        assert_eq!(case.problem.players, 2);
        let official = official_config(&case.problem, &tokens(2), &config.player_names());
        assert_eq!(
            official["teams"][1],
            json!({"token":"token-p1","name":"チーム1"})
        );
        assert_eq!(
            official["problem"]["agentStarts"],
            json!(case.problem.agents)
        );
        assert_eq!(official["problem"]["daySeconds"][0], 2);
    }
}
