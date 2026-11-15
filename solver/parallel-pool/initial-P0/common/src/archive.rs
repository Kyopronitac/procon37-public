//! Portable run archives. Imports publish a new record only after validation succeeds.
use crate::{comparison::read_manifest, engine::*, model::*, runner::*};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

pub const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_EXPANDED_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CASES: usize = 10_000;

#[derive(Serialize, Deserialize)]
struct ArchiveInfo {
    format: String,
    version: u32,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() < 80 && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}

fn validate_manifest(m: &Manifest) -> Result<()> {
    ensure!(
        m.schema_version == 3,
        "対応していない実行履歴のバージョンです"
    );
    ensure!(
        valid_id(&m.id) && m.imported_from.as_deref().is_none_or(valid_id),
        "実行IDが不正です"
    );
    ensure!(
        matches!(m.status.as_str(), "completed" | "cancelled" | "failed"),
        "実行中の履歴は移動できません。完了または中断を待ってください"
    );
    ensure!(
        m.cases.len() <= MAX_CASES
            && m.completed as usize == m.cases.len()
            && m.completed <= m.total
            && m.total > 0,
        "実行件数が不正です"
    );
    ensure!(
        m.status != "completed" || m.completed == m.total,
        "完了件数が一致しません"
    );
    ensure!(
        m.config.label.chars().count() <= 80,
        "実行名は80文字以内にしてください"
    );
    ensure!(
        m.config.opponents.len() == if m.config.mode == Mode::Game { 4 } else { 0 },
        "人数設定が不正です"
    );
    if m.config.mode == Mode::Game && !m.cases.is_empty() {
        ensure!(
            m.opponent_identities.len() == 4,
            "対戦相手の識別情報がありません"
        );
    }
    let mut seeds = BTreeSet::new();
    for case in &m.cases {
        let seed = case.seed.parse::<u64>().context("seedが不正です")?;
        ensure!(
            seed.to_string() == case.seed && seeds.insert(seed),
            "seedが重複しているか不正です"
        );
        ensure!(
            matches!(case.status.as_str(), "AC" | "WA" | "TLE" | "RE"),
            "判定が不正です"
        );
    }
    Ok(())
}

fn allowed_files(m: &Manifest) -> BTreeSet<String> {
    let mut names = BTreeSet::from([
        "archive.json".into(),
        "run.json".into(),
        "build.stdout.log".into(),
        "build.stderr.log".into(),
    ]);
    for case in &m.cases {
        if m.config.mode == Mode::Game {
            for id in 1..5 {
                names.insert(format!("{}/stderr-{id}.log", case.seed));
            }
        }
        for name in ["problem.json", "replay.json", "stderr.log"] {
            names.insert(format!("{}/{name}", case.seed));
        }
    }
    names
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_reader(
        File::open(path)
            .with_context(|| format!("必要なファイルがありません: {}", path.display()))?,
    )
    .with_context(|| format!("JSONが不正です: {}", path.display()))
}

fn validate_cases(dir: &Path, manifest: &Manifest) -> Result<()> {
    for case in &manifest.cases {
        let folder = dir.join(&case.seed);
        // Old error records may have no replay; preserve their unavailable status.
        if case.error.is_some() && !folder.join("replay.json").exists() {
            ensure!(case.status != "AC", "リプレイのないケースはACにできません");
            continue;
        }
        let problem: Problem = read_json(&folder.join("problem.json"))?;
        if manifest.config.mode == Mode::Game {
            let replay: Replay = read_json(&folder.join("replay.json"))?;
            ensure!(
                replay.case.problem == problem
                    && replay.case.seed == case.seed
                    && replay.case.format == case.format,
                "対戦問題が一致しません"
            );
            let names = manifest.config.player_names();
            crate::competition::validate_replay(&replay, case, &names)?;
            continue;
        }
        problem.validate()?;
        let replay: Replay = read_json(&folder.join("replay.json"))?;
        ensure!(
            replay.schema_version == 1
                && replay.case.seed == case.seed
                && replay.case.format == case.format
                && replay.case.problem == problem,
            "seed {} の問題とリプレイが一致しません",
            case.seed
        );
        ensure!(
            replay.score == case.score && replay.answer_time_ms == case.answer_time_ms,
            "seed {} のリプレイとスコアが一致しません",
            case.seed
        );
        ensure!(
            replay.days.len() == problem.day_steps.len(),
            "seed {} の日数が不正です",
            case.seed
        );
        for (index, day) in replay.days.iter().enumerate() {
            ensure!(
                day.day == index + 1
                    && day.steps == problem.day_steps[index]
                    && day.frames.len() == day.steps as usize + 1,
                "seed {} のフレーム数が不正です",
                case.seed
            );
            ensure!(
                day.traffics
                    .iter()
                    .all(|t| t.pos < problem.map.len() && t.status <= 2),
                "交通情報が不正です"
            );
            for (step, frame) in day.frames.iter().enumerate() {
                ensure!(
                    frame.step as usize == step
                        && frame.agents.len() == problem.agents.len()
                        && frame.pending.len() == problem.agents.len()
                        && frame.stocks.len() == problem.spots.len(),
                    "フレームの配列が不正です"
                );
                ensure!(
                    frame
                        .agents
                        .iter()
                        .all(|a| a.kind <= 1 && a.pos < problem.map.len())
                        && frame
                            .pending
                            .iter()
                            .flatten()
                            .all(|p| p.destination.is_none_or(|pos| pos < problem.map.len())),
                    "車の状態が不正です"
                );
            }
        }
        ensure!(
            replay
                .days
                .last()
                .and_then(|d| d.frames.last())
                .is_some_and(|f| f.score == replay.score),
            "最終スコアが一致しません"
        );
        validate_simulation(&problem, &replay, case)
            .with_context(|| format!("seed {} の再計算に失敗しました", case.seed))?;
    }
    Ok(())
}

// Transport failures cannot be reproduced without the original process. They
// must have no submitted answer; selection/plan failures are checked again.
pub(crate) fn validate_answer<T>(
    answer: Option<&serde_json::Value>,
    failure: Option<&Failure>,
    check: impl FnOnce(&serde_json::Value) -> std::result::Result<T, Failure>,
) -> Result<Option<T>> {
    let Some(answer) = answer else {
        ensure!(
            failure.is_some_and(|f| matches!(f.kind.as_str(), "timeout" | "process" | "protocol")),
            "未提出の回答に対応する通信エラーがありません"
        );
        return Ok(None);
    };
    match check(answer) {
        Ok(value) => {
            ensure!(failure.is_none(), "有効な回答に失敗が記録されています");
            Ok(Some(value))
        }
        Err(actual) => {
            ensure!(
                failure.is_some_and(|f| f.kind == actual.kind
                    && f.agent == actual.agent
                    && f.step == actual.step),
                "回答の失敗理由が一致しません"
            );
            Ok(None)
        }
    }
}

fn validate_simulation(problem: &Problem, replay: &Replay, case: &CaseSummary) -> Result<()> {
    let kinds = validate_answer(
        replay.selection_answer.as_ref(),
        replay.selection_failure.as_ref(),
        |answer| validate_kinds(answer, problem.agents.len()),
    )?
    .unwrap_or_else(|| vec![0; problem.agents.len()]);
    ensure!(kinds == replay.kinds, "採用した車種が一致しません");
    let mut game = Game::new(problem.clone(), &kinds);
    let mut answer_time_ms = 0u64;
    for day in &replay.days {
        let accepted = validate_answer(
            day.submitted_answer.as_ref(),
            day.failure.as_ref(),
            |answer| {
                let plan = serde_json::from_value::<Plan>(answer.clone())
                    .map_err(|e| Failure::new("invalid_plan", e.to_string()))?;
                game.play_day(plan)
            },
        )?;
        let actual = if let Some(actual) = accepted {
            answer_time_ms = answer_time_ms
                .checked_add(day.elapsed_ms)
                .context("回答時間が大きすぎます")?;
            actual
        } else {
            game.play_day(game.waiting_plan())
                .map_err(|e| anyhow::anyhow!(e.message))?
        };
        ensure!(
            day.plan == actual.plan
                && day.traffics == actual.traffics
                && day.frames == actual.frames,
            "{}日目の行動計画・交通情報・フレームが再計算結果と一致しません",
            day.day
        );
    }
    let failures = replay
        .selection_failure
        .iter()
        .chain(replay.days.iter().filter_map(|d| d.failure.as_ref()));
    ensure!(
        case.status == verdict(failures.clone()) && case.warnings == failures.count(),
        "判定または警告件数が一致しません"
    );
    ensure!(
        replay.score == game.score && replay.answer_time_ms == answer_time_ms,
        "得点または回答時間が再計算結果と一致しません"
    );
    Ok(())
}

fn regular_file(dir: &Path, name: &str) -> Result<Option<File>> {
    if let Some((parent, _)) = name.split_once('/') {
        match fs::symlink_metadata(dir.join(parent)) {
            Ok(meta) => ensure!(
                meta.is_dir() && !meta.is_symlink(),
                "履歴にリンクが含まれています"
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        }
    }
    let path = dir.join(name);
    match fs::symlink_metadata(&path) {
        Ok(meta) => {
            ensure!(
                meta.is_file() && !meta.is_symlink(),
                "通常のファイルではありません: {name}"
            );
            ensure!(
                meta.len() <= MAX_FILE_BYTES,
                "ファイルが64 MiBを超えています: {name}"
            );
            Ok(Some(File::open(path)?))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Export only known data files; executables and untracked files are never packaged.
pub async fn export_run(root: &Path, id: &str) -> Result<File> {
    ensure!(valid_id(id), "実行IDが不正です");
    let dir = root.join(id);
    let meta = fs::symlink_metadata(&dir)?;
    ensure!(
        meta.is_dir() && !meta.is_symlink(),
        "実行履歴のディレクトリではありません"
    );
    ensure!(
        regular_file(&dir, "run.json")?.is_some(),
        "実行履歴がありません"
    );
    let mut manifest = read_manifest(&dir).await?;
    ensure!(
        (1..=3).contains(&manifest.schema_version),
        "対応していない実行履歴のバージョンです"
    );
    ensure!(manifest.id == id, "保存された実行IDが一致しません");
    manifest.schema_version = 3; // read_manifest upgrades legacy verdicts.
    validate_manifest(&manifest)?;
    tokio::task::spawn_blocking(move || {
        let mut zip = ZipWriter::new(tempfile::tempfile()?);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(6));
        zip.start_file("archive.json", options)?;
        serde_json::to_writer(
            &mut zip,
            &ArchiveInfo {
                format: manifest.config.mode.archive().into(),
                version: 1,
            },
        )?;
        zip.start_file("run.json", options)?;
        serde_json::to_writer(&mut zip, &manifest)?;
        let mut expanded = 0;
        for name in allowed_files(&manifest) {
            if matches!(name.as_str(), "archive.json" | "run.json") {
                continue;
            }
            if let Some(file) = regular_file(&dir, &name)? {
                zip.start_file(&name, options)?;
                let count = std::io::copy(&mut file.take(MAX_FILE_BYTES + 1), &mut zip)?;
                expanded += count;
                ensure!(
                    count <= MAX_FILE_BYTES && expanded <= MAX_EXPANDED_BYTES,
                    "履歴の展開サイズが上限を超えています"
                );
            } else if name.ends_with(".json")
                && manifest
                    .cases
                    .iter()
                    .any(|c| name.starts_with(&format!("{}/", c.seed)) && c.error.is_none())
            {
                anyhow::bail!("必要なファイルがありません: {name}");
            }
        }
        let mut file = zip.finish()?;
        ensure!(
            file.metadata()?.len() <= MAX_ARCHIVE_BYTES,
            "ZIPが256 MiBを超えています"
        );
        file.rewind()?;
        Ok(file)
    })
    .await?
}

pub fn import_run<R: Read + Seek>(
    root: &Path,
    source: R,
    label: Option<String>,
) -> Result<Manifest> {
    import_run_for_mode(root, source, label, Mode::Solo)
}
pub fn import_run_for_mode<R: Read + Seek>(
    root: &Path,
    mut source: R,
    label: Option<String>,
    mode: Mode,
) -> Result<Manifest> {
    ensure!(
        source.seek(SeekFrom::End(0))? <= MAX_ARCHIVE_BYTES,
        "ZIPは256 MiB以下にしてください"
    );
    source.rewind()?;
    let mut zip = ZipArchive::new(source).context("実行履歴のZIPを読み込めません")?;
    ensure!(
        zip.len() <= MAX_CASES * 7 + 4,
        "ZIP内のファイル数が多すぎます"
    );
    let info: ArchiveInfo = zip_json(&mut zip, "archive.json", 4096)?;
    ensure!(
        info.format == mode.archive() && info.version == 1,
        "対応していない履歴アーカイブです"
    );
    let mut manifest: Manifest = zip_json(&mut zip, "run.json", 8 * 1024 * 1024)?;
    validate_manifest(&manifest)?;
    ensure!(
        manifest.config.mode == mode,
        "履歴のテスタ種別が一致しません"
    );
    if let Some(label) = label.filter(|s| !s.trim().is_empty()) {
        ensure!(
            label.chars().count() <= 80,
            "実行名は80文字以内にしてください"
        );
        manifest.config.label = label.trim().into();
    }
    let allowed = allowed_files(&manifest);
    let mut seen = BTreeSet::new();
    let mut expanded = 0;
    // Check every path before creating anything. No ZIP entry controls the output root.
    for i in 0..zip.len() {
        let entry = zip.by_index(i)?;
        ensure!(
            allowed.contains(entry.name())
                && seen.insert(entry.name().to_owned())
                && !entry.is_dir()
                && !entry.is_symlink()
                && !entry.encrypted(),
            "ZIP内に不正なファイルがあります: {}",
            entry.name()
        );
        expanded += entry.size();
        ensure!(
            entry.size() <= MAX_FILE_BYTES && expanded <= MAX_EXPANDED_BYTES,
            "ZIPの展開サイズが上限を超えています"
        );
    }
    fs::create_dir_all(root)?;
    let staging = tempfile::Builder::new()
        .prefix(".import-")
        .tempdir_in(root)?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let path = staging.path().join(entry.name());
        fs::create_dir_all(path.parent().unwrap())?;
        let expected = entry.size();
        let count = std::io::copy(
            &mut (&mut entry).take(MAX_FILE_BYTES + 1),
            &mut File::create(path)?,
        )?;
        ensure!(
            count == expected && count <= MAX_FILE_BYTES,
            "ZIP内のファイルサイズが一致しません"
        );
    }
    validate_cases(staging.path(), &manifest)?;
    // An archive is result data, not authorization to execute its commands.
    let local = RunConfig::for_mode(mode);
    manifest.config.build = local.build;
    manifest.config.solver = local.solver;
    manifest.config.solver_parallel = local.solver_parallel;
    for (saved, default) in manifest.config.opponents.iter_mut().zip(local.opponents) {
        saved.solver = default.solver;
        saved.build = default.build;
        saved.solver_parallel = default.solver_parallel;
    }
    manifest.imported_from = Some(
        manifest
            .imported_from
            .clone()
            .unwrap_or_else(|| manifest.id.clone()),
    );
    manifest.id = run_id();
    let destination = root.join(&manifest.id);
    ensure!(
        !destination.exists(),
        "実行IDが衝突しました。取り込みをやり直してください"
    );
    let mut file = File::create(staging.path().join("run.json"))?;
    serde_json::to_writer(&mut file, &manifest)?;
    file.flush()?;
    drop(file);
    fs::remove_file(staging.path().join("archive.json"))?;
    fs::rename(staging.path(), destination)?;
    Ok(manifest)
}

fn zip_json<R: Read + Seek, T: serde::de::DeserializeOwned>(
    zip: &mut ZipArchive<R>,
    name: &str,
    limit: u64,
) -> Result<T> {
    let entry = zip.by_name(name).with_context(|| {
        format!("{name}がありません。テスタから書き出したZIPを指定してください")
    })?;
    ensure!(entry.size() <= limit, "{name}が大きすぎます");
    let mut bytes = Vec::new();
    entry.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "{name}が大きすぎます");
    serde_json::from_slice(&bytes).with_context(|| format!("{name}が不正です"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, io::Cursor, sync::Arc};

    async fn record(root: &Path) -> Manifest {
        record_mode(root, "archive").await
    }
    async fn record_mode(root: &Path, mode: &str) -> Manifest {
        let config = RunConfig {
            count: Some(3),
            build: None,
            jobs: Some(3),
            label: "別端末の実行".into(),
            solver: CommandSpec {
                program: "python3".into(),
                args: vec![
                    "-u".into(),
                    "-c".into(),
                    include_str!("../../tester/solo/tests/fixtures/solver.py").into(),
                    mode.into(),
                ],
                cwd: root.into(),
                env: BTreeMap::new(),
            },
            ..RunConfig::default()
        };
        let (_cancel, rx) = tokio::sync::watch::channel(false);
        let m = run_batch(run_id(), config, root.into(), Arc::new(|_| {}), rx)
            .await
            .unwrap();
        fs::write(root.join(&m.id).join("build.stderr.log"), "ビルドログ\n").unwrap();
        fs::write(root.join(&m.id).join("0/stderr.log"), "ソルバログ\n").unwrap();
        m
    }
    fn entries(file: File) -> Vec<(String, Vec<u8>)> {
        let mut zip = ZipArchive::new(file).unwrap();
        (0..zip.len())
            .map(|i| {
                let mut file = zip.by_index(i).unwrap();
                let name = file.name().to_owned();
                let mut bytes = vec![];
                file.read_to_end(&mut bytes).unwrap();
                (name, bytes)
            })
            .collect()
    }
    fn packed(entries: &[(String, Vec<u8>)]) -> Cursor<Vec<u8>> {
        let mut zip = ZipWriter::new(Cursor::new(vec![]));
        for (name, bytes) in entries {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(bytes).unwrap();
        }
        let mut data = zip.finish().unwrap();
        data.rewind().unwrap();
        data
    }
    fn empty(root: &Path) -> bool {
        fs::read_dir(root).unwrap().next().is_none()
    }

    fn edit_json(
        data: &mut [(String, Vec<u8>)],
        name: &str,
        edit: impl FnOnce(&mut serde_json::Value),
    ) {
        let bytes = &mut data.iter_mut().find(|(n, _)| n == name).unwrap().1;
        let mut value = serde_json::from_slice(bytes).unwrap();
        edit(&mut value);
        *bytes = serde_json::to_vec(&value).unwrap();
    }

    fn seed_zero(value: &mut serde_json::Value) -> &mut serde_json::Value {
        value["cases"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|case| case["seed"] == "0")
            .unwrap()
    }

    #[tokio::test]
    async fn legacy_arena_setting_survives_archive_import_and_export() {
        let source = tempfile::tempdir().unwrap();
        let original = record(source.path()).await;
        let original_bytes = entries(export_run(source.path(), &original.id).await.unwrap());
        for setting in [None, Some(false), Some(true)] {
            let target = tempfile::tempdir().unwrap();
            let mut data = original_bytes.clone();
            edit_json(&mut data, "run.json", |manifest| {
                let config = manifest["config"].as_object_mut().unwrap();
                config.remove("arenaMaps");
                if let Some(enabled) = setting {
                    config.insert("arenaMaps".into(), enabled.into());
                }
            });
            let imported = import_run(target.path(), packed(&data), None).unwrap();
            assert_eq!(imported.config.arena_maps, setting.unwrap_or(false));
            let saved = read_manifest(&target.path().join(&imported.id))
                .await
                .unwrap();
            assert_eq!(saved.config.arena_maps, setting.unwrap_or(false));
            assert_eq!(saved.cases.len(), original.cases.len());
            let exported = entries(export_run(target.path(), &imported.id).await.unwrap());
            let bytes = &exported
                .iter()
                .find(|(name, _)| name == "run.json")
                .unwrap()
                .1;
            let manifest: Manifest = serde_json::from_slice(bytes).unwrap();
            assert_eq!(manifest.config.arena_maps, setting.unwrap_or(false));
        }
    }

    #[tokio::test]
    async fn import_replaces_archive_commands_with_local_defaults() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let m = record(source.path()).await;
        let mut data = entries(export_run(source.path(), &m.id).await.unwrap());
        for with_build in [true, false] {
            edit_json(&mut data, "run.json", |v| {
                let command = serde_json::json!({
                    "program": "/bin/sh", "args": ["-c", "echo archive-command"],
                    "cwd": "/", "env": {"BASH_ENV": "/archive/startup"}
                });
                v["config"]["solver"] = command.clone();
                v["config"]["build"] = if with_build {
                    command
                } else {
                    serde_json::Value::Null
                };
            });
            let imported = import_run(target.path(), packed(&data), None).unwrap();
            let saved: Manifest =
                read_json(&target.path().join(&imported.id).join("run.json")).unwrap();
            let defaults = RunConfig::default();
            for cfg in [&imported.config, &saved.config] {
                assert_eq!(
                    serde_json::to_value(&cfg.build).unwrap(),
                    serde_json::to_value(&defaults.build).unwrap()
                );
                assert_eq!(
                    serde_json::to_value(&cfg.solver).unwrap(),
                    serde_json::to_value(&defaults.solver).unwrap()
                );
                assert_eq!(cfg.count, m.config.count);
                assert_eq!(cfg.label, m.config.label);
            }
            assert_eq!(
                serde_json::to_value(&saved.cases).unwrap(),
                serde_json::to_value(&m.cases).unwrap()
            );
        }
    }

    #[tokio::test]
    async fn import_rejects_fabricated_scores_and_replay_state() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let m = record(source.path()).await;
        let data = entries(export_run(source.path(), &m.id).await.unwrap());
        let mut fabricated = data.clone();
        edit_json(&mut fabricated, "run.json", |v| {
            seed_zero(v)["score"]["udon"] = 999.into();
        });
        edit_json(&mut fabricated, "0/replay.json", |v| {
            v["score"]["udon"] = 999.into();
            for day in v["days"].as_array_mut().unwrap() {
                for frame in day["frames"].as_array_mut().unwrap() {
                    frame["score"]["udon"] = 999.into();
                }
            }
        });
        assert!(import_run(target.path(), packed(&fabricated), None).is_err());
        assert!(empty(target.path()));

        for field in ["plan", "answer", "event", "traffic", "failure", "kinds"] {
            let mut bad = data.clone();
            edit_json(&mut bad, "0/replay.json", |v| match field {
                "plan" => v["days"][0]["plan"][0] = serde_json::json!([6]),
                "answer" => v["days"][0]["submittedAnswer"][0] = serde_json::json!([6]),
                "event" => {
                    v["days"][0]["frames"][0]["events"] = serde_json::json!([
                        {"kind": "refill", "agent": 0, "amount": 999}
                    ])
                }
                "traffic" => v["days"][0]["traffics"][0]["status"] = 2.into(),
                "failure" => {
                    v["days"][0]["failure"] = serde_json::json!({
                        "kind": "timeout", "message": "fabricated", "agent": null, "step": null
                    })
                }
                "kinds" => v["kinds"][0] = 1.into(),
                _ => unreachable!(),
            });
            assert!(
                import_run(target.path(), packed(&bad), None).is_err(),
                "{field}"
            );
            assert!(empty(target.path()));
        }
        for field in ["status", "warnings"] {
            let mut bad = data.clone();
            edit_json(&mut bad, "run.json", |v| {
                seed_zero(v)[field] = if field == "status" {
                    "WA".into()
                } else {
                    1.into()
                };
            });
            assert!(
                import_run(target.path(), packed(&bad), None).is_err(),
                "{field}"
            );
            assert!(empty(target.path()));
        }
        let mut missing = data;
        missing.retain(|(name, _)| name != "0/replay.json");
        edit_json(&mut missing, "run.json", |v| {
            seed_zero(v)["error"] = "missing replay".into();
        });
        assert!(import_run(target.path(), packed(&missing), None).is_err());
        assert!(empty(target.path()));
    }

    #[tokio::test]
    async fn import_preserves_fallbacks_and_rejects_disguising_them_as_ac() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        for mode in ["invalid", "exit", "malformed"] {
            let m = record_mode(source.path(), mode).await;
            let mut data = entries(export_run(source.path(), &m.id).await.unwrap());
            let imported = import_run(target.path(), packed(&data), None).unwrap();
            assert_eq!(imported.cases[0].status, m.cases[0].status);
            assert_ne!(imported.cases[0].status, "AC");
            edit_json(&mut data, "run.json", |v| {
                v["cases"][0]["status"] = "AC".into();
                v["cases"][0]["warnings"] = 0.into();
            });
            let count = fs::read_dir(target.path()).unwrap().count();
            assert!(import_run(target.path(), packed(&data), None).is_err());
            assert_eq!(fs::read_dir(target.path()).unwrap().count(), count);
        }
    }

    #[tokio::test]
    async fn round_trip_preserves_comparison_replay_logs_and_origin_without_overwrite() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let original = record(source.path()).await;
        let bytes = entries(export_run(source.path(), &original.id).await.unwrap());
        let imported = import_run(target.path(), packed(&bytes), Some("ノートPC".into())).unwrap();
        let again = import_run(
            target.path(),
            export_run(target.path(), &imported.id).await.unwrap(),
            None,
        )
        .unwrap();
        assert_ne!(original.id, imported.id);
        assert_ne!(imported.id, again.id);
        assert_eq!(
            imported.imported_from.as_deref(),
            Some(original.id.as_str())
        );
        assert_eq!(again.imported_from, imported.imported_from);
        assert_eq!(imported.config.label, "ノートPC");
        for name in [
            "0/problem.json",
            "0/replay.json",
            "0/stderr.log",
            "build.stderr.log",
        ] {
            assert_eq!(
                fs::read(source.path().join(&original.id).join(name)).unwrap(),
                fs::read(target.path().join(&imported.id).join(name)).unwrap()
            );
        }
        let comparison = crate::comparison::compare(target.path(), &[imported.id, again.id])
            .await
            .unwrap();
        assert_eq!(comparison.rows.len(), 3);
        assert!(comparison.rows.iter().all(|r| r.alignment == "matched"
            && r.cases.iter().all(|c| c.as_ref().unwrap().status == "AC")));
        assert_eq!(
            read_manifest(&source.path().join(&original.id))
                .await
                .unwrap()
                .config
                .label,
            original.config.label
        );
    }

    #[tokio::test]
    async fn invalid_imports_leave_no_history_or_partial_files() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let m = record(source.path()).await;
        let data = entries(export_run(source.path(), &m.id).await.unwrap());
        for name in [
            "../outside",
            "/tmp/outside",
            "0/../../outside",
            "0/executable",
            "00/problem.json",
        ] {
            let mut bad = data.clone();
            bad.push((name.into(), vec![1]));
            assert!(
                import_run(target.path(), packed(&bad), None).is_err(),
                "{name}"
            );
            assert!(empty(target.path()));
        }
        let mut missing = data.clone();
        missing.retain(|(name, _)| name != "0/replay.json");
        assert!(import_run(target.path(), packed(&missing), None).is_err());
        assert!(empty(target.path()));
        let mut wrong_score = data;
        let (_, manifest) = wrong_score
            .iter_mut()
            .find(|(name, _)| name == "run.json")
            .unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(manifest).unwrap();
        value["cases"][0]["score"]["udon"] = 999.into();
        *manifest = serde_json::to_vec(&value).unwrap();
        assert!(import_run(target.path(), packed(&wrong_score), None).is_err());
        assert!(empty(target.path()));
        assert!(import_run(target.path(), Cursor::new(b"not zip"), None).is_err());
    }

    #[tokio::test]
    async fn ce_and_cancelled_runs_are_portable_but_running_records_are_rejected() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let mut manifest = Manifest {
            opponent_identities: vec![],

            schema_version: 3,
            id: run_id(),
            status: "failed".into(),
            config: RunConfig::default(),
            execution: None,
            verdict: Some("CE".into()),
            total: 60,
            completed: 0,
            cases: vec![],
            error: Some("ビルド失敗".into()),
            imported_from: None,
        };
        let dir = source.path().join(&manifest.id);
        fs::create_dir(&dir).unwrap();
        for status in ["failed", "cancelled", "running"] {
            manifest.status = status.into();
            write_json(&dir.join("run.json"), &manifest).await.unwrap();
            let archive = export_run(source.path(), &manifest.id).await;
            if status == "running" {
                assert!(archive.is_err());
            } else {
                let copy = import_run(target.path(), archive.unwrap(), None).unwrap();
                assert_eq!(copy.status, status);
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn export_rejects_symlinks_and_import_rejects_zip_links() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let m = record(source.path()).await;
        let data = entries(export_run(source.path(), &m.id).await.unwrap());
        let mut zip = ZipWriter::new(Cursor::new(vec![]));
        for (name, bytes) in data {
            if name == "0/stderr.log" {
                zip.add_symlink(name, "/etc/passwd", SimpleFileOptions::default())
                    .unwrap();
            } else {
                zip.start_file(name, SimpleFileOptions::default()).unwrap();
                zip.write_all(&bytes).unwrap();
            }
        }
        assert!(import_run(target.path(), zip.finish().unwrap(), None).is_err());
        assert!(empty(target.path()));
        let log = source.path().join(&m.id).join("0/stderr.log");
        fs::remove_file(&log).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", log).unwrap();
        assert!(export_run(source.path(), &m.id).await.is_err());
    }
}
