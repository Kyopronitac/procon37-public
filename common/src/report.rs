//! CLI reports use the same saved-problem alignment and score priority as the GUI.
use crate::{comparison::Comparison, model::MatchFormat, runner::*};
use anyhow::{Result, ensure};
use serde::Serialize;
use std::{cmp::Ordering, fmt::Write};

#[derive(Default, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScoreTotals {
    pub unique_brands: u128,
    pub daily_brands: u128,
    pub udon: u128,
}
impl ScoreTotals {
    fn add(&mut self, case: &CaseSummary) {
        self.unique_brands += case.score.unique_brands as u128;
        self.daily_brands += case.score.daily_brands as u128;
        self.udon += u128::from(case.score.udon);
    }
}
#[derive(Default, Debug, Serialize, PartialEq, Eq)]
pub struct VerdictCounts {
    #[serde(rename = "AC")]
    pub ac: usize,
    #[serde(rename = "WA")]
    pub wa: usize,
    #[serde(rename = "TLE")]
    pub tle: usize,
    #[serde(rename = "RE")]
    pub re: usize,
}
impl VerdictCounts {
    fn add(&mut self, status: &str) {
        match status {
            "AC" => self.ac += 1,
            "WA" => self.wa += 1,
            "TLE" => self.tle += 1,
            "RE" => self.re += 1,
            _ => {}
        }
    }
}
#[derive(Default, Debug, Serialize, PartialEq, Eq)]
pub struct CaseTotals {
    pub count: usize,
    pub scores: ScoreTotals,
    pub verdicts: VerdictCounts,
}
pub fn case_totals<'a>(cases: impl Iterator<Item = &'a CaseSummary>) -> CaseTotals {
    let mut total = CaseTotals::default();
    for case in cases {
        total.count += 1;
        total.scores.add(case);
        total.verdicts.add(&case.status);
    }
    total
}

fn scopes(format: Option<MatchFormat>) -> Vec<Option<MatchFormat>> {
    match format {
        Some(format) => vec![Some(format)],
        None => vec![
            None,
            Some(MatchFormat::First),
            Some(MatchFormat::Semifinal),
            Some(MatchFormat::Final),
        ],
    }
}
fn format_name(format: Option<MatchFormat>) -> &'static str {
    match format {
        None => "全体",
        Some(MatchFormat::First) => "1回戦",
        Some(MatchFormat::Semifinal) => "準決勝",
        Some(MatchFormat::Final) => "決勝",
    }
}
fn state_name(status: &str) -> &str {
    match status {
        "building" => "ビルド中",
        "running" => "実行中",
        "completed" => "完了",
        "cancelled" => "中断",
        "failed" => "失敗",
        other => other,
    }
}
fn single_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

// Cells contain only numbers, ASCII markers, and fixed Japanese labels.
fn table(out: &mut String, headings: &[&str], rows: Vec<Vec<String>>) {
    let width = |s: &str| {
        s.chars()
            .map(|c| if c.is_ascii() { 1 } else { 2 })
            .sum::<usize>()
    };
    let widths = headings
        .iter()
        .enumerate()
        .map(|(i, h)| {
            rows.iter()
                .map(|r| width(&r[i]))
                .chain([width(h)])
                .max()
                .unwrap()
        })
        .collect::<Vec<_>>();
    for row in std::iter::once(headings.iter().map(|h| h.to_string()).collect()).chain(rows) {
        for (i, value) in row.iter().enumerate() {
            if i > 0 {
                out.push_str("  ");
            }
            let padding = " ".repeat(widths[i] - width(value));
            if i == 0 {
                out.push_str(value);
                out.push_str(&padding);
            } else {
                out.push_str(&padding);
                out.push_str(value);
            }
        }
        out.push('\n');
    }
}

pub fn run_summary(manifest: &Manifest) -> String {
    let mut out = format!("\n実行ID: {}\n", manifest.id);
    if !manifest.config.label.is_empty() {
        writeln!(out, "実行名: {}", single_line(&manifest.config.label)).unwrap();
    }
    writeln!(
        out,
        "非公式練習場のマップ: {}",
        if manifest.config.arena_maps {
            "含める（4 seedに1件）"
        } else {
            "含めない"
        }
    )
    .unwrap();
    writeln!(
        out,
        "状態: {} / 完了 {} / {}件 / CE={}（実行単位）",
        state_name(&manifest.status),
        manifest.completed,
        manifest.total,
        usize::from(manifest.verdict.as_deref() == Some("CE"))
    )
    .unwrap();
    out.push_str("スコア合計・判定件数（完了ケース。AC以外のスコアも含む）\n");
    let rows = scopes(None)
        .into_iter()
        .map(|format| {
            let t = case_totals(
                manifest
                    .cases
                    .iter()
                    .filter(|c| format.is_none_or(|f| c.format == f)),
            );
            vec![
                format_name(format).into(),
                t.count.to_string(),
                t.scores.unique_brands.to_string(),
                t.scores.daily_brands.to_string(),
                t.scores.udon.to_string(),
                t.verdicts.ac.to_string(),
                t.verdicts.wa.to_string(),
                t.verdicts.tle.to_string(),
                t.verdicts.re.to_string(),
            ]
        })
        .collect();
    table(
        &mut out,
        &[
            "形式",
            "件数",
            "種類数",
            "日別種類",
            "玉数",
            "AC",
            "WA",
            "TLE",
            "RE",
        ],
        rows,
    );
    if manifest.config.mode == Mode::Game {
        out.push_str("\n対象ソルバの順位（全員ACのみ・平均は小さいほど上位）\n");
        let rows = scopes(None)
            .into_iter()
            .map(|format| {
                let totals = match_totals(
                    manifest
                        .cases
                        .iter()
                        .filter(|c| format.is_none_or(|f| c.format == f)),
                );
                let mut row = vec![
                    format_name(format).into(),
                    totals.accepted.to_string(),
                    rank_mean_text(totals.average_rank),
                ];
                row.extend(totals.ranks.iter().map(usize::to_string));
                row
            })
            .collect();
        table(
            &mut out,
            &[
                "形式",
                "全員AC",
                "平均順位",
                "1位",
                "2位",
                "3位",
                "4位",
                "5位",
            ],
            rows,
        );
        out.push_str(&match_totals_text(&match_totals(manifest.cases.iter())));
    }
    out
}

#[derive(Default, Debug, Serialize, PartialEq, Eq)]
pub struct Outcomes {
    pub wins: usize,
    pub draws: usize,
    pub losses: usize,
}
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScoreDelta {
    pub unique_brands: i128,
    pub daily_brands: i128,
    pub udon: i128,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInfo {
    pub id: String,
    pub label: String,
    pub imported_from: Option<String>,
    pub status: String,
    pub verdict: Option<String>,
    pub total: u32,
    pub completed: u32,
    pub verdicts: VerdictCounts,
    pub execution: Option<Execution>,
    pub solver_parallel: bool,
    pub arena_maps: bool,
    pub time_limit_ms: Option<u64>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game: Option<MatchTotals>,
    pub id: String,
    pub outcomes: Option<Outcomes>,
    pub scores: Option<ScoreTotals>,
    pub delta: Option<ScoreDelta>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonScope {
    pub format: Option<MatchFormat>,
    pub seeds: usize,
    pub matched: usize,
    pub accepted: usize,
    pub different: usize,
    pub missing: usize,
    pub unavailable: usize,
    pub non_ac: usize,
    pub results: Vec<ComparisonResult>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonReport {
    pub schema_version: u32,
    pub baseline: String,
    pub runs: Vec<RunInfo>,
    pub scopes: Vec<ComparisonScope>,
}
pub fn comparison_report(
    comparison: &Comparison,
    baseline: usize,
    format: Option<MatchFormat>,
) -> Result<ComparisonReport> {
    ensure!(baseline < comparison.runs.len(), "比較の基準が不正です");
    ensure!(
        comparison
            .rows
            .iter()
            .all(|r| r.cases.len() == comparison.runs.len()),
        "比較する履歴の件数が一致しません"
    );
    let summaries = scopes(format)
        .into_iter()
        .map(|format| {
            let rows = comparison
                .rows
                .iter()
                .filter(|r| format.is_none_or(|f| r.cases.iter().flatten().any(|c| c.format == f)))
                .collect::<Vec<_>>();
            let count = |alignment| rows.iter().filter(|r| r.alignment == alignment).count();
            let accepted = rows
                .iter()
                .filter(|r| {
                    r.alignment == "matched"
                        && r.cases
                            .iter()
                            .all(|c| c.as_ref().is_some_and(|c| c.status == "AC"))
                })
                .collect::<Vec<_>>();
            let mut base = ScoreTotals::default();
            for row in &accepted {
                base.add(row.cases[baseline].as_ref().unwrap());
            }
            let results = comparison
                .runs
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let mut total = ScoreTotals::default();
                    let mut outcomes = Outcomes::default();
                    for row in &accepted {
                        let candidate = row.cases[i].as_ref().unwrap();
                        let reference = row.cases[baseline].as_ref().unwrap();
                        total.add(candidate);
                        let a = &candidate.score;
                        let b = &reference.score;
                        match (a.unique_brands, a.daily_brands, a.udon).cmp(&(
                            b.unique_brands,
                            b.daily_brands,
                            b.udon,
                        )) {
                            Ordering::Greater => outcomes.wins += 1,
                            Ordering::Equal => outcomes.draws += 1,
                            Ordering::Less => outcomes.losses += 1,
                        }
                    }
                    let delta = (i != baseline && !accepted.is_empty()).then(|| ScoreDelta {
                        unique_brands: total.unique_brands as i128 - base.unique_brands as i128,
                        daily_brands: total.daily_brands as i128 - base.daily_brands as i128,
                        udon: total.udon as i128 - base.udon as i128,
                    });
                    ComparisonResult {
                        game: (m.config.mode == Mode::Game).then(|| {
                            match_totals(accepted.iter().map(|r| r.cases[i].as_ref().unwrap()))
                        }),
                        id: m.id.clone(),
                        outcomes: (i != baseline && !accepted.is_empty()).then_some(outcomes),
                        scores: (!accepted.is_empty()).then_some(total),
                        delta,
                    }
                })
                .collect();
            ComparisonScope {
                format,
                seeds: rows.len(),
                matched: count("matched"),
                accepted: accepted.len(),
                different: count("different"),
                missing: count("missing"),
                unavailable: count("unavailable"),
                non_ac: count("matched") - accepted.len(),
                results,
            }
        })
        .collect();
    Ok(ComparisonReport {
        schema_version: 1,
        baseline: comparison.runs[baseline].id.clone(),
        scopes: summaries,
        runs: comparison
            .runs
            .iter()
            .map(|m| RunInfo {
                id: m.id.clone(),
                label: m.config.label.clone(),
                imported_from: m.imported_from.clone(),
                status: m.status.clone(),
                verdict: m.verdict.clone(),
                total: m.total,
                completed: m.completed,
                verdicts: case_totals(m.cases.iter()).verdicts,
                execution: m.execution.clone(),
                solver_parallel: m.config.solver_parallel,
                arena_maps: m.config.arena_maps,
                time_limit_ms: m.config.time_limit_ms,
            })
            .collect(),
    })
}
pub fn comparison_text(report: &ComparisonReport) -> String {
    let mut out = String::from("実行履歴の比較\n");
    for (i, run) in report.runs.iter().enumerate() {
        let tag = char::from(b'A' + i as u8);
        writeln!(
            out,
            "[{tag}] {}{}\n  ID: {} / {} / 完了 {} / {}件",
            single_line(&run.label),
            if run.id == report.baseline {
                "（比較の基準）"
            } else {
                ""
            },
            run.id,
            state_name(&run.status),
            run.completed,
            run.total
        )
        .unwrap();
        if let Some(source) = &run.imported_from {
            writeln!(out, "  取込元の実行ID: {source}").unwrap();
        }
        let v = &run.verdicts;
        writeln!(
            out,
            "  判定: AC={} WA={} TLE={} RE={} / CE={}（実行単位）",
            v.ac,
            v.wa,
            v.tle,
            v.re,
            usize::from(run.verdict.as_deref() == Some("CE"))
        )
        .unwrap();
        writeln!(
            out,
            "  同時seed={} / ソルバ内並列={} / 制限時間={} / 非公式練習場のマップ={}",
            run.execution
                .as_ref()
                .map(|e| e.jobs.to_string())
                .unwrap_or_else(|| "記録なし".into()),
            if run.solver_parallel {
                "あり"
            } else {
                "なし"
            },
            run.time_limit_ms
                .map(|ms| format!("{ms} ms"))
                .unwrap_or_else(|| "公式条件".into()),
            if run.arena_maps {
                "あり（1/4）"
            } else {
                "なし"
            }
        )
        .unwrap();
    }
    out.push_str("\n勝敗・合計は同一問題かつ全履歴ACのケースが対象。勝敗は種類数 → 日別種類 → 玉数の順。\nスコアの括弧内は基準からの差分。対象なしは - で表示します。\n");
    for scope in &report.scopes {
        writeln!(
            out,
            "\n{}: {}seed / 同一問題={} / 共通AC={}",
            format_name(scope.format),
            scope.seeds,
            scope.matched,
            scope.accepted
        )
        .unwrap();
        writeln!(
            out,
            "集計対象外: 問題相違={} 未実行あり={} 問題確認不可={} 同一問題でAC以外={}",
            scope.different, scope.missing, scope.unavailable, scope.non_ac
        )
        .unwrap();
        let rows = scope
            .results
            .iter()
            .enumerate()
            .map(|(i, result)| {
                let mut row = vec![char::from(b'A' + i as u8).to_string()];
                row.extend(match &result.outcomes {
                    Some(o) => vec![
                        o.wins.to_string(),
                        o.draws.to_string(),
                        o.losses.to_string(),
                    ],
                    None => vec!["-".into(); 3],
                });
                if let Some(scores) = &result.scores {
                    let values = [scores.unique_brands, scores.daily_brands, scores.udon];
                    let deltas = result
                        .delta
                        .as_ref()
                        .map(|d| [d.unique_brands, d.daily_brands, d.udon]);
                    row.extend(values.into_iter().enumerate().map(|(i, v)| {
                        deltas.map_or_else(|| v.to_string(), |d| format!("{v} ({:+})", d[i]))
                    }));
                } else {
                    row.extend(vec!["-".into(); 3]);
                }
                row
            })
            .collect();
        table(
            &mut out,
            &[
                "履歴",
                "勝",
                "同点",
                "敗",
                "種類数合計",
                "日別種類合計",
                "玉数合計",
            ],
            rows,
        );
    }
    for scope in &report.scopes {
        for result in &scope.results {
            if let Some(game) = &result.game {
                writeln!(out, "{} / {}", format_name(scope.format), result.id).unwrap();
                out.push_str(&match_totals_text(game));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{comparison::ComparisonRow, model::Score};

    #[test]
    fn match_averages_preserve_ties_and_weight_cases_across_formats() {
        let make = |format, ranks: [usize; 5]| {
            let mut c = case(format, (1, 0, 0), "AC");
            c.players = ranks
                .into_iter()
                .enumerate()
                .map(|(id, rank)| PlayerSummary {
                    id,
                    name: format!("player-{id}"),
                    rank,
                    score: Score {
                        unique_brands: 6 - rank,
                        daily_brands: 0,
                        udon: 0,
                    },
                    status: "AC".into(),
                    answer_time_ms: 0,
                    warnings: 0,
                })
                .collect();
            c
        };
        let mut cases = vec![
            make(MatchFormat::First, [1, 1, 3, 4, 5]),
            make(MatchFormat::First, [3, 1, 2, 4, 5]),
            make(MatchFormat::Semifinal, [5, 4, 3, 2, 1]),
        ];
        let mut failed = make(MatchFormat::Final, [1; 5]);
        failed.players[3].status = "TLE".into();
        cases.push(failed);
        let mut invalid = make(MatchFormat::Final, [1; 5]);
        invalid.players[0].rank = 0;
        cases.push(invalid);
        cases.push(case(MatchFormat::Final, (1, 0, 0), "AC"));
        let total = match_totals(cases.iter());
        assert_eq!(total.accepted, 3);
        assert_eq!(total.ranks, [1, 0, 1, 0, 1]);
        assert_eq!(total.average_rank, Some(3.0));
        assert_eq!(
            total.player_average_ranks,
            [
                Some(3.0),
                Some(2.0),
                Some(8.0 / 3.0),
                Some(10.0 / 3.0),
                Some(11.0 / 3.0)
            ]
        );
        let scope_means: Vec<_> = scopes(None)
            .into_iter()
            .map(|format| {
                match_totals(
                    cases
                        .iter()
                        .filter(|c| format.is_none_or(|f| c.format == f)),
                )
                .average_rank
            })
            .collect();
        assert_eq!(scope_means, [Some(3.0), Some(2.0), Some(5.0), None]);
        let empty = match_totals([].iter());
        assert_eq!(empty.average_rank, None);
        assert_eq!(
            serde_json::to_value(&empty).unwrap()["averageRank"],
            serde_json::Value::Null
        );
        assert!(match_totals_text(&empty).contains("対象の平均順位 —"));
        assert!(match_totals_text(&total).contains("平均順位=2.67位"));
        let mut manifest = fixture().runs.remove(0);
        manifest.config.mode = Mode::Game;
        manifest.cases = cases;
        let text = run_summary(&manifest)
            .lines()
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join("\n");
        for row in [
            "全体 3 3.00位",
            "1回戦 2 2.00位",
            "準決勝 1 5.00位",
            "決勝 0 —",
        ] {
            assert!(text.contains(row), "{text}");
        }
    }

    fn case(format: MatchFormat, score: (usize, usize, u32), status: &str) -> CaseSummary {
        CaseSummary {
            players: vec![],

            seed: "0".into(),
            format,
            status: status.into(),
            score: Score {
                unique_brands: score.0,
                daily_brands: score.1,
                udon: score.2,
            },
            answer_time_ms: 0,
            warnings: 0,
            error: None,
        }
    }
    fn fixture() -> Comparison {
        use MatchFormat::*;
        let candidates = [
            (First, (11, 1, 1)),
            (First, (10, 41, 1)),
            (Semifinal, (10, 40, 101)),
            (Semifinal, (10, 40, 100)),
            (Final, (9, 100, 999)),
        ];
        let mut rows = candidates
            .into_iter()
            .enumerate()
            .map(|(i, (format, score))| ComparisonRow {
                seed: i.to_string(),
                alignment: "matched",
                cases: vec![
                    Some(case(format, (10, 40, 100), "AC")),
                    Some(case(format, score, "AC")),
                    Some(case(format, (10, 40, 100), "AC")),
                ],
            })
            .collect::<Vec<_>>();
        for (i, alignment) in ["matched", "different", "missing", "unavailable", "matched"]
            .into_iter()
            .enumerate()
        {
            let mut cases = vec![Some(case(Final, (10, 40, 100), "AC")); 3];
            if i == 0 {
                cases[2].as_mut().unwrap().status = "RE".into();
            }
            if i == 2 {
                cases[1] = None;
            }
            if i == 4 {
                cases[1].as_mut().unwrap().status = "WA".into();
            }
            rows.push(ComparisonRow {
                seed: (i + 5).to_string(),
                alignment,
                cases,
            });
        }
        let runs = (0..3)
            .map(|i| {
                let cases = rows
                    .iter()
                    .filter_map(|r| r.cases[i].clone())
                    .collect::<Vec<_>>();
                Manifest {
                    opponent_identities: vec![],

                    schema_version: 3,
                    id: format!("10{i}-1"),
                    status: "completed".into(),
                    config: RunConfig {
                        label: format!("solver {i}"),
                        ..RunConfig::default()
                    },
                    execution: None,
                    verdict: Some(batch_verdict(&cases).into()),
                    total: 10,
                    completed: cases.len() as u32,
                    cases,
                    error: None,
                    imported_from: None,
                }
            })
            .collect();
        Comparison { runs, rows }
    }

    #[test]
    fn totals_include_non_ac_and_do_not_overflow_a_single_case_score() {
        let cases = [
            case(MatchFormat::First, (2, 5, u32::MAX), "AC"),
            case(MatchFormat::Final, (3, 8, u32::MAX), "RE"),
        ];
        let total = case_totals(cases.iter());
        assert_eq!(total.count, 2);
        assert_eq!(
            total.scores,
            ScoreTotals {
                unique_brands: 5,
                daily_brands: 13,
                udon: 2 * u128::from(u32::MAX)
            }
        );
        assert_eq!(
            total.verdicts,
            VerdictCounts {
                ac: 1,
                re: 1,
                ..VerdictCounts::default()
            }
        );
        assert_eq!(case_totals([].iter()), CaseTotals::default());
    }

    #[test]
    fn comparisons_use_score_priority_all_run_acceptance_and_format_groups() {
        let comparison = fixture();
        let report = comparison_report(&comparison, 0, None).unwrap();
        let all = &report.scopes[0];
        assert_eq!(
            (all.seeds, all.matched, all.accepted, all.non_ac),
            (10, 7, 5, 2)
        );
        assert_eq!((all.different, all.missing, all.unavailable), (1, 1, 1));
        assert_eq!(
            all.results[1].outcomes,
            Some(Outcomes {
                wins: 3,
                draws: 1,
                losses: 1
            })
        );
        assert_eq!(
            all.results[1].scores,
            Some(ScoreTotals {
                unique_brands: 50,
                daily_brands: 222,
                udon: 1202
            })
        );
        assert_eq!(
            all.results[1].delta,
            Some(ScoreDelta {
                unique_brands: 0,
                daily_brands: 22,
                udon: 702
            })
        );
        assert!(all.results[0].outcomes.is_none());
        for (scope, expected) in report.scopes[1..]
            .iter()
            .zip([(2, 0, 0), (1, 1, 0), (0, 0, 1)])
        {
            let result = scope.results[1].outcomes.as_ref().unwrap();
            assert_eq!((result.wins, result.draws, result.losses), expected);
        }
        let filtered = comparison_report(&comparison, 1, Some(MatchFormat::First)).unwrap();
        assert_eq!(filtered.scopes.len(), 1);
        assert_eq!(filtered.scopes[0].accepted, 2);
        assert_eq!(
            filtered.scopes[0].results[0].outcomes,
            Some(Outcomes {
                wins: 0,
                draws: 0,
                losses: 2
            })
        );
        assert_eq!(
            filtered.scopes[0].results[0].delta,
            Some(ScoreDelta {
                unique_brands: -1,
                daily_brands: 38,
                udon: 198
            })
        );
    }

    #[test]
    fn empty_comparisons_and_ce_are_not_reported_as_draws_or_zero_scores() {
        let mut comparison = fixture();
        comparison.rows.retain(|r| r.seed == "7");
        comparison.runs[1].verdict = Some("CE".into());
        comparison.runs[1].cases.clear();
        let report = comparison_report(&comparison, 0, Some(MatchFormat::Final)).unwrap();
        let scope = &report.scopes[0];
        assert_eq!(scope.accepted, 0);
        assert_eq!(scope.missing, 1);
        assert!(
            scope
                .results
                .iter()
                .all(|r| r.scores.is_none() && r.outcomes.is_none() && r.delta.is_none())
        );
        assert!(comparison_text(&report).contains("CE=1"));
        let json = serde_json::to_value(&report).unwrap();
        assert!(json["scopes"][0]["results"][1]["scores"].is_null());
        assert!(comparison_report(&comparison, 3, None).is_err());
    }
}

#[derive(Default, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MatchTotals {
    pub ranks: [usize; 5],
    pub average_rank: Option<f64>,
    pub player_average_ranks: [Option<f64>; 5],
    pub opponents: [Outcomes; 4],
    pub scores: [ScoreTotals; 5],
    pub accepted: usize,
}
pub fn match_totals<'a>(cases: impl Iterator<Item = &'a CaseSummary>) -> MatchTotals {
    let mut totals = MatchTotals::default();
    let mut rank_sums = [0_u64; 5];
    for case in cases.filter(|c| {
        c.status == "AC"
            && c.players.len() == 5
            && c.players
                .iter()
                .all(|p| p.status == "AC" && (1..=5).contains(&p.rank))
    }) {
        totals.accepted += 1;
        let target = &case.players[0];
        if (1..=5).contains(&target.rank) {
            totals.ranks[target.rank - 1] += 1;
        }
        for (id, p) in case.players.iter().enumerate() {
            rank_sums[id] += p.rank as u64;
            totals.scores[id].unique_brands += p.score.unique_brands as u128;
            totals.scores[id].daily_brands += p.score.daily_brands as u128;
            totals.scores[id].udon += u128::from(p.score.udon);
            if id > 0 {
                let o = &mut totals.opponents[id - 1];
                match crate::competition::score_order(&target.score, &p.score) {
                    Ordering::Greater => o.wins += 1,
                    Ordering::Equal => o.draws += 1,
                    Ordering::Less => o.losses += 1,
                }
            }
        }
    }
    if totals.accepted > 0 {
        totals.player_average_ranks =
            rank_sums.map(|sum| Some(sum as f64 / totals.accepted as f64));
        totals.average_rank = totals.player_average_ranks[0];
    }
    totals
}
fn rank_mean_text(value: Option<f64>) -> String {
    value.map_or_else(|| "—".into(), |rank| format!("{rank:.2}位"))
}
fn match_totals_text(t: &MatchTotals) -> String {
    let mut out = format!(
        "対戦成績（全員AC {}件）\n対象の平均順位 {}\n対象順位 1位={} 2位={} 3位={} 4位={} 5位={}\n",
        t.accepted,
        rank_mean_text(t.average_rank),
        t.ranks[0],
        t.ranks[1],
        t.ranks[2],
        t.ranks[3],
        t.ranks[4]
    );
    for (i, s) in t.scores.iter().enumerate() {
        writeln!(
            out,
            "プレイヤー{i}: 平均順位={} 種類={} 日別種類={} 玉数={}",
            rank_mean_text(t.player_average_ranks[i]),
            s.unique_brands,
            s.daily_brands,
            s.udon
        )
        .unwrap();
    }
    for (i, o) in t.opponents.iter().enumerate() {
        writeln!(
            out,
            "対象 vs プレイヤー{}: {}勝 {}同点 {}敗",
            i + 1,
            o.wins,
            o.draws,
            o.losses
        )
        .unwrap();
    }
    out
}
