use crate::{
    archive, comparison,
    model::MatchFormat,
    report,
    runner::{self, Mode, ProgressEvent, RunConfig},
    server,
};
use anyhow::{Context, Result, bail, ensure};
use std::{path::PathBuf, sync::Arc};

const HELP: &str = r#"ヘキサうどん テスタ

  tester run [件数] [--seed SEED] [--format first|semifinal|final]
  tester compare RUN_ID RUN_ID [RUN_ID ...] [比較オプション]
  tester generate [--seed SEED] [--format first|semifinal|final] [--output FILE]
  tester serve [--port 8080]
  tester export RUN_ID [--output FILE.zip] [--output-dir DIR]
  tester import FILE.zip [--label NAME] [--output-dir DIR]

比較オプション:
  --baseline RUN_ID   比較の基準（既定: 最初の実行ID）
  --format FORMAT     first / semifinal / final / all（既定all）
  --json              比較結果をJSONで出力
  --output-dir DIR    履歴保存先（既定 .tester/runs）
  比較は2〜4件。全体・形式別の勝敗とスコア合計・差分を表示。
  同一問題かつ全履歴ACのケースを集計。

共通・実行設定:
  --config FILE       ビルド・ソルバ設定のJSON
  --jobs auto|N       同時実行するseed数（既定auto: CPU数）
  --solver-parallel   ソルバ内並列あり（autoでは1seedずつ）
  --solver-serial     ソルバ内並列なし（既定）
  --label NAME        履歴に表示する実行名
  --solver FILE       ソルバ実行ファイル
  --no-build          実行前のビルドを省略
  --time-limit-ms N   タイプ選択・各日の制限時間を上書き
  --arena-maps       非公式練習場のマップを4 seedに1件含める（既定）
  --no-arena-maps    非公式練習場のマップを含めない
  --output-dir DIR    結果保存先（既定 .tester/runs）
  -- ARGS...          ソルバに渡す引数

件数は1〜10000、既定値は60。件数と--seedは排他。形式自動決定はseed % 3の順に1回戦・準決勝・決勝。
seed 0＋1回戦・seed 1＋準決勝・seed 2＋決勝は公式サンプル（プレイヤー数は1）を使用。
上記の対応は形式自動決定・明示指定の両方で有効。
実行の完了・中断・失敗時に、全体・形式別のスコア合計と判定件数を表示。
"#;

pub async fn run(mode: Mode) -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        if mode == Mode::Game {
            print!(
                "{}",
                HELP.replace("tester", "game")
                    .replace(".tester/runs", ".game/runs")
                    .replace("8080", "8081")
                    .replace("プレイヤー数は1", "プレイヤー数は5")
                    .replace("auto: CPU数", "auto: max(1, CPU数 / 5)")
            );
            println!(
                "対戦相手4枠は --config の opponents 配列で設定します。順位は得点3指標のみ、回答時間は参考表示です。"
            );
        } else {
            print!("{HELP}");
        }
        return Ok(());
    }
    let command = &args[0];
    if command == "compare" {
        return compare_history(&args[1..], mode).await;
    }
    if matches!(command.as_str(), "export" | "import") {
        return transfer_history(&args, mode).await;
    }
    ensure!(
        ["run", "generate", "serve"].contains(&command.as_str()),
        "不明なコマンドです\n{HELP}"
    );
    let mut config = RunConfig::for_mode(mode);
    let option_end = args.iter().position(|s| s == "--").unwrap_or(args.len());
    if let Some(i) = args[..option_end].iter().position(|s| s == "--config") {
        let path = args
            .get(i + 1)
            .context("--configにファイルを指定してください")?;
        config = serde_json::from_slice(
            &std::fs::read(path).with_context(|| format!("設定を読み込めません: {path}"))?,
        )?;
    }
    let mut root = PathBuf::from(mode.root());
    let mut output = None;
    let mut port = mode.port();
    let mut count = None;
    let mut seed = None;
    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        i += 1;
        if arg == "--" {
            config.solver.args = args[i..].to_vec();
            break;
        }
        if arg == "--solver-parallel" || arg == "--solver-serial" {
            config.solver_parallel = arg == "--solver-parallel";
            continue;
        }
        if arg == "--no-build" {
            config.build = None;
            for opponent in &mut config.opponents {
                opponent.build = None;
            }
            continue;
        }
        if arg == "--arena-maps" || arg == "--no-arena-maps" {
            config.arena_maps = arg == "--arena-maps";
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
            "--jobs" => {
                config.jobs = if value == "auto" {
                    None
                } else {
                    Some(value.parse()?)
                }
            }
            "--label" => config.label = value.clone(),
            "--solver" => config.solver.program = value.clone(),
            "--time-limit-ms" => config.time_limit_ms = Some(value.parse()?),
            "--output-dir" => root = PathBuf::from(value),
            "--output" => output = Some(PathBuf::from(value)),
            "--port" => port = value.parse()?,
            _ => bail!("不明なオプション: {arg}"),
        }
    }
    ensure!(
        !(count.is_some() && seed.is_some()),
        "件数と--seedは同時に指定できません"
    );
    if let Some(n) = count {
        config.count = Some(n);
        config.seed = None;
    }
    if let Some(s) = seed {
        config.seed = Some(s);
        config.count = None;
    }
    config.mode = mode;
    match command.as_str() {
        "generate" => {
            let seed = config.seed.as_deref().unwrap_or("0").parse()?;
            let case = crate::generator::generate_for_players_with_arena_maps(
                seed,
                config.format,
                mode.players(),
                config.arena_maps,
            )?;
            let bytes = serde_json::to_vec_pretty(&case.problem)?;
            if let Some(path) = output {
                std::fs::write(path, bytes)?;
            } else {
                println!("{}", String::from_utf8(bytes)?);
            }
            eprintln!(
                "seed={} format={} generator={}",
                case.seed,
                case.format.name(),
                case.generator_version
            );
        }
        "serve" => {
            server::serve(root, port, config).await?;
        }
        "run" => {
            config.validate()?;
            let execution = config.execution(runner::available_cpus());
            eprintln!(
                "利用可能CPU={} 同時seed={} ソルバ内並列={}",
                execution.available_cpus, execution.jobs, config.solver_parallel
            );
            let id = runner::run_id();
            let path = root.join(&id);
            let (tx, rx) = tokio::sync::watch::channel(false);
            let signal = tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                let _ = tx.send(true);
            });
            let printed = std::sync::Mutex::new(0usize);
            let progress = Arc::new(move |event| match event {
                ProgressEvent::BuildLog(log) => eprint!("{log}"),
                ProgressEvent::Status(m) => {
                    let mut done = printed.lock().unwrap();
                    for c in m.cases.iter().skip(*done) {
                        eprintln!(
                            "seed={} format={} 種類={} 日別種類={} 玉数={} 回答={}ms {}",
                            c.seed,
                            c.format.name(),
                            c.score.unique_brands,
                            c.score.daily_brands,
                            c.score.udon,
                            c.answer_time_ms,
                            c.status
                        );
                    }
                    *done = m.cases.len();
                }
            });
            let result = runner::run_batch(id, config, root, progress, rx).await;
            signal.abort();
            let manifest = result?;
            print!("{}", report::run_summary(&manifest));
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

async fn transfer_history(args: &[String], mode: Mode) -> Result<()> {
    let export = args[0] == "export";
    let source = args
        .get(1)
        .context("exportには実行ID、importにはZIPファイルを指定してください")?;
    let mut root = PathBuf::from(mode.root());
    let mut output = PathBuf::from(format!("run-{source}.procon37.zip"));
    let mut label = None;
    for pair in args[2..].chunks(2) {
        let value = pair.get(1).context("オプションに値を指定してください")?;
        match pair[0].as_str() {
            "--output-dir" => root = PathBuf::from(value),
            "--output" if export => output = PathBuf::from(value),
            "--label" if !export => label = Some(value.clone()),
            flag => bail!("不明なオプション: {flag}"),
        }
    }
    if export {
        ensure!(
            comparison::read_manifest(&root.join(source))
                .await?
                .config
                .mode
                == mode,
            "別のテスタの履歴です"
        );
        let mut archive = archive::export_run(&root, source).await?;
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        std::io::copy(&mut archive, &mut file)?;
        file.persist_noclobber(&output)
            .context("出力先が既に存在するか、書き込みできません")?;
        println!("エクスポート: {}", output.display());
    } else {
        let file = std::fs::File::open(source).context("ZIPファイルを読み込めません")?;
        let manifest = archive::import_run_for_mode(&root, file, label, mode)?;
        println!("インポート: {}", root.join(&manifest.id).display());
    }
    Ok(())
}

struct CompareOptions {
    ids: Vec<String>,
    root: PathBuf,
    baseline: usize,
    format: Option<MatchFormat>,
    json: bool,
}
impl CompareOptions {
    fn parse(args: &[String], mode: Mode) -> Result<Self> {
        let mut ids = vec![];
        let mut root = PathBuf::from(mode.root());
        let mut baseline = None;
        let mut format = None;
        let mut json = false;
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
                        "--format" => {
                            format = if value == "all" {
                                None
                            } else {
                                Some(MatchFormat::parse(value)?)
                            }
                        }
                        _ => root = PathBuf::from(value),
                    }
                }
                flag if flag.starts_with('-') => bail!("不明な比較オプション: {flag}"),
                _ => ids.push(arg.clone()),
            }
        }
        ensure!(
            (2..=4).contains(&ids.len()),
            "比較する実行IDを2〜4件指定してください"
        );
        let baseline = match baseline {
            Some(id) => ids
                .iter()
                .position(|candidate| *candidate == id)
                .context("--baselineは比較対象の実行IDを指定してください")?,
            None => 0,
        };
        Ok(Self {
            ids,
            root,
            baseline,
            format,
            json,
        })
    }
}
async fn compare_history(args: &[String], mode: Mode) -> Result<()> {
    let options = CompareOptions::parse(args, mode)?;
    let comparison = comparison::compare(&options.root, &options.ids).await?;
    ensure!(
        comparison.runs.iter().all(|m| m.config.mode == mode),
        "別のテスタの履歴です"
    );
    let report = report::comparison_report(&comparison, options.baseline, options.format)?;
    if options.json {
        serde_json::to_writer_pretty(std::io::stdout().lock(), &report)?;
        println!();
    } else {
        print!("{}", report::comparison_text(&report));
    }
    Ok(())
}
