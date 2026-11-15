use crate::{model::Problem, runner::*};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

/// Read old records without modifying them on disk.
pub async fn read_manifest(directory: &Path) -> Result<Manifest> {
    let mut manifest: Manifest =
        serde_json::from_slice(&tokio::fs::read(directory.join("run.json")).await?)?;
    for case in &mut manifest.cases {
        match case.status.as_str() {
            "ok" => case.status = "AC".into(),
            "error" => case.status = "WA".into(),
            "fallback" | "WA" if manifest.schema_version < 3 && case.error.is_none() => {
                let seed = case.seed.parse::<u64>()?;
                let bytes =
                    tokio::fs::read(directory.join(seed.to_string()).join("replay.json")).await?;
                let replay: crate::model::Replay = serde_json::from_slice(&bytes)?;
                case.status = verdict(
                    replay
                        .selection_failure
                        .iter()
                        .chain(replay.days.iter().filter_map(|d| d.failure.as_ref())),
                )
                .into();
            }
            _ => {}
        }
    }
    if manifest.schema_version == 1
        && manifest.status == "failed"
        && manifest
            .error
            .as_deref()
            .is_some_and(|e| e.starts_with("ビルド"))
    {
        manifest.verdict = Some("CE".into());
    }
    if manifest.status == "completed" {
        manifest.verdict = Some(batch_verdict(&manifest.cases).into());
    }
    Ok(manifest)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub runs: Vec<Manifest>,
    pub rows: Vec<ComparisonRow>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonRow {
    pub seed: String,
    pub alignment: &'static str,
    pub cases: Vec<Option<CaseSummary>>,
}

fn alignment(cases: &[Option<CaseSummary>], problems: &[Option<Problem>]) -> &'static str {
    if cases.iter().any(Option::is_none) {
        return "missing";
    }
    if problems.iter().any(Option::is_none) {
        return "unavailable";
    }
    if problems.windows(2).all(|pair| pair[0] == pair[1])
        && cases
            .windows(2)
            .all(|pair| pair[0].as_ref().unwrap().format == pair[1].as_ref().unwrap().format)
    {
        "matched"
    } else {
        "different"
    }
}

pub async fn compare(root: &Path, ids: &[String]) -> Result<Comparison> {
    ensure!(
        (2..=4).contains(&ids.len()),
        "比較する実行は2〜4件を選んでください"
    );
    ensure!(
        ids.iter().collect::<BTreeSet<_>>().len() == ids.len(),
        "同じ実行が重複しています"
    );
    let mut runs = Vec::new();
    for id in ids {
        ensure!(
            !id.is_empty() && id.len() < 80 && id.bytes().all(|b| b.is_ascii_digit() || b == b'-'),
            "実行IDが不正です"
        );
        runs.push(
            read_manifest(&root.join(id))
                .await
                .with_context(|| format!("実行 {id} を読み込めません"))?,
        );
    }
    let indices = runs
        .iter()
        .map(|run| {
            run.cases
                .iter()
                .map(|case| Ok((case.seed.parse::<u64>()?, case)))
                .collect::<Result<BTreeMap<_, _>>>()
        })
        .collect::<Result<Vec<_>>>()?;
    let seeds = indices
        .iter()
        .flat_map(|index| index.keys().copied())
        .collect::<BTreeSet<_>>();
    let mut rows = Vec::new();
    for seed in seeds {
        let cases = indices
            .iter()
            .map(|index| index.get(&seed).map(|case| (*case).clone()))
            .collect::<Vec<_>>();
        let mut problems = Vec::new();
        for (id, case) in ids.iter().zip(&cases) {
            let problem = if case.is_some() {
                tokio::fs::read(root.join(id).join(seed.to_string()).join("problem.json"))
                    .await
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Problem>(&bytes).ok())
            } else {
                None
            };
            problems.push(problem);
        }
        rows.push(ComparisonRow {
            seed: seed.to_string(),
            alignment: alignment(&cases, &problems),
            cases,
        });
    }
    if runs.iter().any(|r| r.config.mode == Mode::Game) {
        ensure!(
            runs.iter().all(|r| r.config.mode == Mode::Game),
            "異なるテスタの履歴は比較できません"
        );
        let matched = runs.iter().all(|r| r.opponent_identities.len() == 4)
            && runs.windows(2).all(|r| {
                r[0].opponent_identities == r[1].opponent_identities
                    && r[0].config.time_limit_ms == r[1].config.time_limit_ms
            });
        if !matched {
            for row in &mut rows {
                if row.alignment == "matched" {
                    row.alignment = "different";
                }
            }
        }
    }
    Ok(Comparison { runs, rows })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{generator::generate, model::*};

    #[test]
    fn compares_saved_problems_instead_of_assuming_equal_seeds_match() {
        let case = generate(0, Some(MatchFormat::First)).unwrap();
        let summary = CaseSummary {
            players: vec![],

            seed: "0".into(),
            format: case.format,
            status: "AC".into(),
            score: Score::default(),
            answer_time_ms: 0,
            warnings: 0,
            error: None,
        };
        let cases = vec![Some(summary.clone()), Some(summary)];
        let problems = vec![Some(case.problem.clone()), Some(case.problem.clone())];
        assert_eq!(alignment(&cases, &problems), "matched");
        let mut changed = problems.clone();
        changed[1].as_mut().unwrap().fuel_limits += 1;
        assert_eq!(alignment(&cases, &changed), "different");
        assert_eq!(
            alignment(&cases, &[None, problems[1].clone()]),
            "unavailable"
        );
        assert_eq!(alignment(&[None, cases[1].clone()], &problems), "missing");
    }
}
