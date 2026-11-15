//! server: 公式簡易サーバー(procon-server)を使い、複数PCのソルバを対戦させる。
//!
//! 単独・対戦テスタ(solo/game)と同じく、CUIの `run` とGUIの `serve` から実行し、
//! 同じ形式の履歴(run.json・replay.json)を残す。

mod gui;
mod proxy;
mod runner;
mod state;

use anyhow::{Context, Result, bail, ensure};
use procon37_common::{
    comparison,
    model::MatchFormat,
    report,
    runner::{PlayerSummary, run_id},
};
use runner::{Manifest, Progress, ServerConfig};
use std::{path::PathBuf, sync::Arc};

const ROOT: &str = ".server/runs";
const GUI_PORT: u16 = 8082;
const MATCH_PORT: u16 = 11451;

const HELP: &str = r#"ヘキサうどん 対戦サーバー

  server run [件数] [--seed SEED] [--format first|semifinal|final] [--players N]
  server compare RUN_ID RUN_ID [RUN_ID ...] [--baseline RUN_ID] [--format FORMAT] [--json]
  server generate [--seed SEED] [--format first|semifinal|final] [--players N] [--output FILE]
  server serve [--port 8082]

共通・実行設定:
  --config FILE          実行設定のJSON
  --label NAME           履歴に表示する実行名
  --players N            チーム数（既定1、最大16）。チームNは token-pN で参加
  --time-limit-ms N      車種選択・各日の制限時間を上書き（各日は秒単位に切り上げ）
  --arena-maps          非公式練習場のマップを4 seedに1件含める（既定）
  --no-arena-maps       非公式練習場のマップを含めない
  --server-program FILE  公式対戦サーバー（既定 tester/server/procon-server）
  --match-port PORT      対戦窓口のポート（既定11451）
  --output-dir DIR       履歴保存先（既定 .server/runs）

件数は1〜10000、既定値は12。件数と--seedは排他。seedは0から順に1試合ずつ実行する。
各ソルバPCでは tester/server-bridge/run.sh を起動しておく（試合が始まると自動で参加）。
順位は 種類数 → 日別種類数 → 玉数 で決まり、回答時間は参考表示。
"#;

struct Options {
    config: ServerConfig,
    root: PathBuf,
    port: u16,
    match_port: u16,
    output: Option<PathBuf>,
}

fn parse(command: &str, args: &[String]) -> Result<Options> {
    let mut config = ServerConfig::default();
    if let Some(i) = args.iter().position(|s| s == "--config") {
        let path = args
            .get(i + 1)
            .context("--configにファイルを指定してください")?;
        config = serde_json::from_slice(
            &std::fs::read(path).with_context(|| format!("設定を読み込めません: {path}"))?,
        )
        .with_context(|| format!("設定を読み込めません: {path}"))?;
    }
    let mut options = Options {
        config,
        root: PathBuf::from(ROOT),
        port: GUI_PORT,
        match_port: MATCH_PORT,
        output: None,
    };
    let (mut count, mut seed) = (None, None);
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        i += 1;
        if arg == "--arena-maps" || arg == "--no-arena-maps" {
            options.config.arena_maps = arg == "--arena-maps";
            continue;
        }
        if !arg.starts_with('-') {
            ensure!(command == "run" && count.is_none(), "不明な引数: {arg}");
            count = Some(arg.parse::<u32>().context("seed数が不正です")?);
            continue;
        }
        let value = args
            .get(i)
            .with_context(|| format!("{arg}に値を指定してください"))?;
        i += 1;
        let config = &mut options.config;
        match arg.as_str() {
            "--config" => {}
            "--seed" => {
                value.parse::<u64>().context("seedが不正です")?;
                seed = Some(value.clone());
            }
            "--format" => {
                config.format = if value == "auto" {
                    None
                } else {
                    Some(MatchFormat::parse(value)?)
                }
            }
            "--label" => config.label = value.clone(),
            "--players" => config.players = value.parse().context("チーム数が不正です")?,
            "--time-limit-ms" => config.time_limit_ms = Some(value.parse()?),
            "--server-program" => config.server_program = value.clone(),
            "--output-dir" => options.root = PathBuf::from(value),
            "--output" => options.output = Some(PathBuf::from(value)),
            "--port" => options.port = value.parse()?,
            "--match-port" => options.match_port = value.parse()?,
            _ => bail!("不明なオプション: {arg}"),
        }
    }
    ensure!(
        !(count.is_some() && seed.is_some()),
        "件数と--seedは同時に指定できません"
    );
    if let Some(n) = count {
        options.config.count = Some(n);
        options.config.seed = None;
    }
    if let Some(s) = seed {
        options.config.seed = Some(s);
        options.config.count = None;
    }
    Ok(options)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{HELP}");
        return Ok(());
    }
    let command = args[0].as_str();
    if command == "compare" {
        return compare(&args[1..]).await;
    }
    ensure!(
        ["run", "generate", "serve"].contains(&command),
        "不明なコマンドです\n{HELP}"
    );
    let mut options = parse(command, &args[1..])?;
    match command {
        "generate" => {
            let seed = options.config.seed.as_deref().unwrap_or("0").parse()?;
            let case = runner::generate_case(seed, &options.config)?;
            let bytes = serde_json::to_vec_pretty(&case.problem)?;
            if let Some(path) = options.output {
                std::fs::write(path, bytes)?;
            } else {
                println!("{}", String::from_utf8(bytes)?);
            }
            eprintln!(
                "seed={} format={} players={} generator={}",
                case.seed,
                case.format.name(),
                case.problem.players,
                case.generator_version
            );
        }
        "serve" => {
            let shared = state::SharedState::default();
            let error = match proxy::bind(options.match_port).await {
                Ok(listener) => {
                    tokio::spawn(proxy::serve(listener, shared.clone()));
                    eprintln!("対戦窓口: {}", window_url(options.match_port));
                    None
                }
                // 履歴の閲覧はできるよう、GUIは起動して実行だけを断る。
                Err(e) => {
                    eprintln!("{e:#}");
                    Some(format!("{e:#}"))
                }
            };
            let window = gui::Window {
                port: options.match_port,
                address: local_address(),
                error,
            };
            gui::serve(options.root, options.port, options.config, shared, window).await?;
        }
        "run" => {
            options.config.validate()?;
            let shared = state::SharedState::default();
            let listener = proxy::bind(options.match_port).await?;
            tokio::spawn(proxy::serve(listener, shared.clone()));
            let names = options.config.player_names();
            eprintln!("対戦窓口: {}", window_url(options.match_port));
            for (i, name) in names.iter().enumerate() {
                eprintln!("  チーム{i} {name}: run.sh の TEAM={i}（token-p{i}）");
            }
            let id = run_id();
            let path = options.root.join(&id);
            let (tx, rx) = tokio::sync::watch::channel(false);
            let signal = tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                let _ = tx.send(true);
            });
            let printed = std::sync::Mutex::new(0usize);
            let progress: Progress = Arc::new(move |m: &Manifest| {
                let mut done = printed.lock().unwrap();
                for c in m.cases.iter().skip(*done) {
                    eprintln!(
                        "seed={} format={} {} 順位 {}",
                        c.seed,
                        c.format.name(),
                        c.status,
                        ranks_text(&c.players)
                    );
                }
                *done = m.cases.len();
            });
            let result =
                runner::run_batch(id, options.config, options.root, shared, progress, rx).await;
            signal.abort();
            let manifest = result?;
            // 集計表示は単独・対戦テスタと共通。スコアはチーム0の値。
            let saved = comparison::read_manifest(&path).await?;
            print!("{}", report::run_summary(&saved));
            print!("{}", team_summary(&manifest));
            println!("結果: {}", path.canonicalize().unwrap_or(path).display());
            if manifest.status != "completed" {
                bail!(
                    "{}: {}",
                    manifest.verdict.as_deref().unwrap_or("実行中断"),
                    manifest.error.unwrap_or(manifest.status)
                );
            }
            if manifest.cases.iter().any(|c| c.status != "AC") {
                bail!("WA / TLE / REのケースが含まれます。結果を確認してください");
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn ranks_text(players: &[PlayerSummary]) -> String {
    players
        .iter()
        .map(|p| {
            format!(
                "{}:{}位(種類{} 日別{} 玉{}{})",
                p.id,
                p.rank,
                p.score.unique_brands,
                p.score.daily_brands,
                p.score.udon,
                if p.status == "AC" {
                    String::new()
                } else {
                    format!(" {}", p.status)
                }
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// チーム別の平均順位・スコア合計。順位は全員ACの試合のみ(対戦テスタと同じ基準)。
fn team_summary(manifest: &Manifest) -> String {
    let names = manifest.config.player_names();
    let accepted: Vec<_> = manifest
        .cases
        .iter()
        .filter(|c| c.status == "AC" && c.players.len() == names.len())
        .collect();
    let mut out = format!(
        "\nチーム別（順位は全員ACの{}試合・平均は小さいほど上位 / スコアは全完了試合の合計）\n",
        accepted.len()
    );
    for (i, name) in names.iter().enumerate() {
        let ranks: Vec<usize> = accepted.iter().map(|c| c.players[i].rank).collect();
        let average = if ranks.is_empty() {
            "—".into()
        } else {
            format!(
                "{:.2}位",
                ranks.iter().sum::<usize>() as f64 / ranks.len() as f64
            )
        };
        let scores = manifest.cases.iter().filter_map(|c| c.players.get(i));
        let (unique, daily, udon) = scores.fold((0, 0, 0), |(a, b, c), p| {
            (
                a + p.score.unique_brands,
                b + p.score.daily_brands,
                c + u64::from(p.score.udon),
            )
        });
        out.push_str(&format!(
            "  {i}: {name}  平均順位 {average}  種類数 {unique}  日別種類 {daily}  玉数 {udon}\n"
        ));
    }
    out
}

async fn compare(args: &[String]) -> Result<()> {
    let mut ids = vec![];
    let mut root = PathBuf::from(ROOT);
    let (mut baseline, mut format, mut json) = (None, None, false);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--baseline" | "--format" | "--output-dir" => {
                let value = args
                    .next()
                    .filter(|v| !v.starts_with('-'))
                    .with_context(|| format!("{arg}に値を指定してください"))?;
                match arg.as_str() {
                    "--baseline" => baseline = Some(value.clone()),
                    "--format" if value == "all" => format = None,
                    "--format" => format = Some(MatchFormat::parse(value)?),
                    _ => root = PathBuf::from(value),
                }
            }
            flag if flag.starts_with('-') => bail!("不明な比較オプション: {flag}"),
            _ => ids.push(arg.clone()),
        }
    }
    let baseline = match baseline {
        Some(id) => ids
            .iter()
            .position(|candidate| *candidate == id)
            .context("--baselineは比較対象の実行IDを指定してください")?,
        None => 0,
    };
    // 比較はチーム0の得点で行う(単独・対戦テスタと同じ照合・集計)。
    let comparison = comparison::compare(&root, &ids).await?;
    let report = report::comparison_report(&comparison, baseline, format)?;
    if json {
        serde_json::to_writer_pretty(std::io::stdout().lock(), &report)?;
        println!();
    } else {
        print!("{}", report::comparison_text(&report));
    }
    Ok(())
}

fn window_url(port: u16) -> String {
    format!(
        "http://{}:{port}/",
        local_address().unwrap_or_else(|| "このPCのIPアドレス".into())
    )
}

/// ソルバPCから接続するときのアドレス。経路表から既定の送信元を調べるだけで、通信はしない。
fn local_address() -> Option<String> {
    let socket = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(("192.0.2.1", 9)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then(|| ip.to_string())
}
