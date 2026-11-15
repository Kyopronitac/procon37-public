//! Phase-relative checkpoints with time reserved for transmission before the server deadline.
use super::*;
use tokio::time::{sleep, timeout};

const FINAL_CHECKPOINT_MARGIN_MS: u64 = 5_000;
const BACKUP_CHECKPOINT_MARGIN_MS: u64 = 3_000;
const SUBMISSION_MARGIN_MS: u64 = 2_000;

fn final_checkpoints(start_ms: u64, deadline_ms: u64) -> (u64, u64) {
    let duration = deadline_ms.saturating_sub(start_ms);
    (deadline_ms.saturating_sub(FINAL_CHECKPOINT_MARGIN_MS.min(duration / 2)).max(start_ms),
     deadline_ms.saturating_sub(BACKUP_CHECKPOINT_MARGIN_MS.min(duration / 3)).max(start_ms))
}

fn submission_deadline(start_ms: u64, deadline_ms: u64) -> u64 {
    // Very short local tests still get a send window; official phases are >=20s.
    let margin = SUBMISSION_MARGIN_MS.min(deadline_ms.saturating_sub(start_ms) / 4);
    deadline_ms.saturating_sub(margin)
}

#[derive(Default)]
pub(super) struct PhaseResult {
    pub answer: Option<Value>,
    pub outcome: Option<bool>,
    pub from_solver: bool,
    pub uncertain_answers: Vec<Value>,
}

impl PhaseResult {
    fn record(&mut self, answer: Value, from_solver: bool, outcome: Option<bool>) {
        match outcome {
            Some(true) => {
                self.answer = Some(answer);
                self.outcome = Some(true);
                self.from_solver = from_solver;
                self.uncertain_answers.clear();
            }
            Some(false) => {} // A rejected replacement does not erase a prior acceptance.
            None => {
                for candidate in self.answer.iter().chain(std::iter::once(&answer)) {
                    if !self.uncertain_answers.contains(candidate) {
                        self.uncertain_answers.push(candidate.clone());
                    }
                }
                self.answer = Some(answer);
                self.outcome = None;
                self.from_solver = from_solver;
            }
        }
    }
}

pub(super) fn targets(start_ms: u64, offsets: &[u64], deadline_ms: u64) -> Vec<u64> {
    let mut targets: Vec<_> = offsets.iter().map(|ms| start_ms.saturating_add(*ms)).collect();
    let (safe_final, backup) = final_checkpoints(start_ms, deadline_ms);
    if targets.last().is_some_and(|last| *last > safe_final) {
        // Selection and action phases can have different lengths. A 90/120s
        // selection schedule must finish at 55s and 57s on a 60s day, never 59.5s.
        targets.retain(|target| *target < safe_final);
        targets.push(safe_final);
    }
    if targets.last() == Some(&safe_final) && backup > safe_final {
        targets.push(backup);
    }
    targets
}

pub(super) fn budget(targets: &[u64], now: u64) -> u64 {
    targets.last().copied().unwrap_or(now)
        .saturating_sub(now.saturating_add(NETWORK_MARGIN_MS)).max(1)
}

fn valid_answer(value: &Value, request: &Request, game: Option<&Game>) -> bool {
    if let Some(problem) = &request.problem {
        validate_kinds(value, problem.agents.len()).is_ok()
    } else if let (Some(game), Some(state)) = (game, &request.state) {
        let Ok(plan) = serde_json::from_value::<Plan>(value.clone()) else { return false; };
        let mut check = game.clone();
        check.agents = state.agents.clone();
        if !state.others.is_empty() {
            check.set_observed_traffic(state.traffics.clone());
        }
        check.play_day(plan).is_ok()
    } else {
        false
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    proc: &mut SolverProcess,
    client: &Client,
    request: &Request,
    targets: &[u64],
    phase_start_ms: u64,
    deadline_ms: u64,
    streaming: bool,
    previous_answer: Option<&Value>,
    fallback: Value,
    game: Option<&Game>,
) -> Result<PhaseResult> {
    let started = Instant::now();
    let now = now_ms();
    let send_deadline_ms = submission_deadline(phase_start_ms, deadline_ms);
    let recovery = targets.iter().all(|target| *target <= now) && now < send_deadline_ms;
    let targets = if recovery {
        eprintln!("[bridge] checkpoint_recovery phase={} request_id={} reason=all_checkpoints_elapsed", request.phase, request.request_id);
        vec![now]
    } else {
        targets.to_vec()
    };
    let mut wire_request = serde_json::to_value(request)?;
    if streaming {
        wire_request["previousAnswer"] = previous_answer.cloned().unwrap_or(Value::Null);
    }
    let search_end_ms = now_ms().saturating_add(request.time_limit_ms);
    let mut collecting = match timeout(Duration::from_millis(request.time_limit_ms),
        proc.begin_stream(&wire_request)).await {
        Ok(Ok(())) => true,
        other => {
            eprintln!("[bridge] stream_request_failed request_id={} detail={other:?}", request.request_id);
            false
        }
    };
    let mut latest: Option<Value> = None;
    let mut last_submitted_answer: Option<Value> = None;
    let mut result = PhaseResult { outcome: Some(false), ..Default::default() };
    let path = if request.phase == "select" { "/agent" } else { "/" };
    eprintln!("[bridge] scheduled_phase phase={} request_id={} targets_ms={targets:?} limit_ms={} stream_responses={streaming}",
        request.phase, request.request_id, request.time_limit_ms);
    for (index, &target) in targets.iter().enumerate() {
        if target < now_ms() && !recovery {
            eprintln!("[bridge] checkpoint_skipped phase={} request_id={} checkpoint={} reason=already_elapsed target_ms={target}",
                request.phase, request.request_id, index + 1);
            continue;
        }
        loop {
            let now = now_ms();
            if now >= target { break; }
            if collecting && now < search_end_ms {
                let stop = target.min(search_end_ms);
                match timeout(Duration::from_millis(stop - now), proc.next_stream_response()).await {
                    Ok(Ok(response)) => {
                        if response.request_id != request.request_id {
                            eprintln!("[bridge] stream_response_ignored request_id={} received_id={} reason=wrong_request", request.request_id, response.request_id);
                            continue;
                        }
                        if !streaming && !response.is_final {
                            eprintln!("[bridge] stream_response_ignored request_id={} reason=streaming_disabled", request.request_id);
                            continue;
                        }
                        if valid_answer(&response.answer, request, game) {
                            latest = Some(response.answer);
                        } else {
                            eprintln!("[bridge] stream_response_ignored request_id={} reason=invalid_answer", request.request_id);
                        }
                        if response.is_final { collecting = false; }
                    }
                    Ok(Err(failure)) => {
                        eprintln!("[bridge] stream_response_ignored request_id={} reason={} detail={}", request.request_id, failure.kind, failure.message);
                        if failure.kind == "process" { collecting = false; }
                    }
                    Err(_) => { if now_ms() >= search_end_ms { collecting = false; } }
                }
            } else {
                sleep(Duration::from_millis(target - now)).await;
            }
        }
        if now_ms() >= send_deadline_ms {
            eprintln!("[bridge] checkpoint_skipped phase={} request_id={} checkpoint={} reason=insufficient_submission_margin target_ms={target}",
                request.phase, request.request_id, index + 1);
            break;
        }
        let answer = latest.clone().unwrap_or_else(|| fallback.clone());
        let from_solver = latest.is_some();
        if last_submitted_answer.as_ref() == Some(&answer) {
            eprintln!("[bridge] checkpoint_skipped phase={} request_id={} checkpoint={} reason=unchanged_answer target_ms={target}",
                request.phase, request.request_id, index + 1);
            client.record_event(serde_json::json!({"event":"scheduledSubmissionSkipped", "requestId":request.request_id,
                "phase":request.phase, "checkpoint":index + 1, "targetMs":target, "reason":"unchanged_answer",
                "answer":answer, "fromSolver":from_solver})).await;
            continue;
        }
        // A failed early checkpoint cannot consume the next checkpoint's entire interval.
        let checkpoint_deadline = targets.get(index + 1).copied()
            .map_or(send_deadline_ms, |next| next.saturating_sub(SUBMIT_CUTOFF_MS).min(send_deadline_ms));
        eprintln!("[bridge] checkpoint_submit phase={} request_id={} checkpoint={} target_ms={} actual_ms={} elapsed_ms={} fallback={}",
            request.phase, request.request_id, index + 1, target, now_ms(), now_ms().saturating_sub(phase_start_ms), !from_solver);
        let submitted = submit(client, path, &answer, checkpoint_deadline).await;
        // Definite rejections and unknown network results count as attempted submissions too.
        // Retries within this call retain their existing behavior; a pre-send deadline does not count.
        if submitted.http_attempts > 0 {
            last_submitted_answer = Some(answer.clone());
        }
        let outcome = if submitted.http_attempts == 0 {
            Some(false)
        } else if request.phase == "select" {
            match &submitted.reply {
                Some((reply, received)) if reply.status == 200 && *received < deadline_ms => Some(true),
                Some(_) if !submitted.lost => Some(false),
                _ if submitted.attempts == 0 => Some(false),
                _ => None,
            }
        } else if submitted.attempts == 0 {
            Some(false)
        } else {
            submitted.outcome(request.state.as_ref().unwrap().ends_at)
        };
        eprintln!("[bridge] checkpoint_result phase={} request_id={} checkpoint={} result={} attempts={} submit_ms={}",
            request.phase, request.request_id, index + 1, acceptance_label(outcome), submitted.attempts, submitted.elapsed_ms);
        client.record_event(serde_json::json!({"event":"scheduledSubmission", "requestId":request.request_id,
            "phase":request.phase, "checkpoint":index + 1, "targetMs":target, "outcome":outcome,
            "answer":answer, "fromSolver":from_solver, "httpAttempts":submitted.http_attempts})).await;
        result.record(answer, from_solver, outcome);
    }
    eprintln!("[bridge] solve_end phase={} day={} solver_ms={} limit_ms={} result={} scheduled=true",
        request.phase, request.state.as_ref().map_or(0, |state| state.day + 1),
        started.elapsed().as_millis(), request.time_limit_ms,
        if latest.is_some() { "ok" } else { "failed" });
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_are_phase_relative_and_budget_ends_before_final_checkpoint() {
        let offsets = [5_000, 15_000, 25_000, 35_000, 45_000, 55_000];
        let targets = targets(1_000_000, &offsets, 1_060_000);
        assert_eq!(targets, [1_005_000, 1_015_000, 1_025_000, 1_035_000, 1_045_000, 1_055_000, 1_057_000]);
        assert_eq!(budget(&targets, 1_002_300), 54_200);
        assert_eq!(budget(&targets, 1_057_100), 1);
    }

    #[test]
    fn shortened_phases_keep_early_checkpoints_and_two_final_opportunities() {
        let fixed = targets(1_000_000, &[55_000], 1_010_000);
        let distributed = targets(1_000_000, &[5_000, 15_000, 25_000, 35_000, 45_000, 55_000], 1_010_000);
        assert_eq!(fixed, [1_005_000, 1_007_000]);
        assert_eq!(distributed, [1_005_000, 1_007_000]);
        assert_eq!(targets(1_000_000, &[500, 1000], 1_001_000), [1_000_500, 1_000_667]);
        assert_eq!(targets(1_000_000, &[5000, 55_000], 1_005_400), [1_002_700, 1_003_600]);
        assert_eq!(targets(1_000_000, &[55_000], 1_000_200), [1_000_100, 1_000_134]);
    }

    #[test]
    fn all_match_modes_submit_at_55_and_57_seconds() {
        for selection in [60_000, 90_000, 120_000] {
            let offsets: Vec<_> = (5_000..selection).step_by(10_000).collect();
            assert_eq!(targets(1_000_000, &offsets, 1_060_000),
                       [1_005_000, 1_015_000, 1_025_000, 1_035_000, 1_045_000, 1_055_000, 1_057_000]);
            assert_eq!(targets(1_000_000, &offsets, 1_000_000 + selection).last(),
                       Some(&(1_000_000 + selection - 3_000)));
        }
        let offsets = [5_000, 15_000, 25_000, 35_000, 45_000, 55_000];
        assert_eq!(targets(0, &offsets, 20_000), [5_000, 15_000, 17_000]);
        assert_eq!(targets(0, &offsets, 30_000), [5_000, 15_000, 25_000, 27_000]);
        assert_eq!(targets(0, &[59_500], 60_000), [55_000, 57_000]);
        assert_eq!(targets(0, &[5_000, 15_000], 60_000), [5_000, 15_000]);
    }

    #[test]
    fn retries_stop_before_the_server_deadline() {
        assert_eq!(submission_deadline(1_000_000, 1_060_000), 1_058_000);
        assert_eq!(submission_deadline(1_000_000, 1_059_900), 1_057_900);
        assert_eq!(submission_deadline(1_000_000, 1_001_000), 1_000_750);
    }

    #[tokio::test]
    async fn final_answer_can_send_at_55_seconds_but_not_at_59_5_seconds() {
        for (elapsed_ms, expected_posts) in [(55_000, 1), (57_000, 1), (59_500, 0)] {
            let (base, received) = super::super::tests::scripted(vec![(200, r#"{"revision":1}"#)]);
            let client = Client::new(&base, "fixture").unwrap();
            let start = now_ms() - elapsed_ms;
            let submitted = submit(&client, "/", &serde_json::json!([[-1]]),
                                   submission_deadline(start, start + 60_000)).await;
            assert_eq!(submitted.http_attempts, expected_posts);
            assert_eq!(received.try_iter().count(), expected_posts as usize);
        }
    }

    #[tokio::test]
    async fn checkpoint_57_sends_only_when_answer_changed_since_55() {
        for changed in [false, true] {
            let expected_posts = if changed { 2 } else { 1 };
            let (base, received) = super::super::tests::scripted(
                vec![(200, r#"{"revision":1}"#); expected_posts]);
            let client = Client::new(&base, "fixture").unwrap();
            let script = r#"
import json, sys, time
request = json.loads(sys.stdin.readline())
answer = [0] * len(request['problem']['agents'])
print(json.dumps({'requestId': request['requestId'], 'answer': answer, 'final': False}), flush=True)
time.sleep(1)
if sys.argv[1] == 'true':
    answer[0] = 1
print(json.dumps({'requestId': request['requestId'], 'answer': answer, 'final': True}), flush=True)
"#;
            let spec = CommandSpec {
                program: "python3".into(),
                args: vec!["-u".into(), "-c".into(), script.into(), changed.to_string()],
                cwd: std::env::temp_dir(), env: BTreeMap::new(),
            };
            let mut proc = SolverProcess::spawn_inherit_stderr(&spec).await.unwrap();
            // Enter the real 60s schedule just before its last two checkpoints.
            let start = now_ms() - 54_500;
            let deadline = start + 60_000;
            let targets = targets(start, &[5_000, 15_000, 25_000, 35_000, 45_000, 55_000], deadline);
            let problem = procon37_common::generator::generate(0, Some(MatchFormat::First)).unwrap().problem;
            let fallback = serde_json::json!(vec![0; problem.agents.len()]);
            let request = Request {
                protocol_version: PROTOCOL_VERSION, request_id: 0, phase: "select".into(),
                time_limit_ms: budget(&targets, now_ms()), problem: Some(problem),
                state: None, steps: None, previous_accepted: None,
            };
            let result = run(&mut proc, &client, &request, &targets, start, deadline,
                             true, None, fallback, None).await.unwrap();
            proc.finish().await;
            assert_eq!(result.outcome, Some(true));
            assert!(result.from_solver);
            assert_eq!(client.post_attempts.load(Ordering::Relaxed), expected_posts as u64);
            assert_eq!(received.try_iter().count(), expected_posts);
            assert_eq!(result.answer.unwrap()[0], if changed { 1 } else { 0 });
        }
    }

    #[test]
    fn failed_final_replacement_preserves_accepted_interim() {
        let mut result = PhaseResult { outcome: Some(false), ..Default::default() };
        result.record(serde_json::json!([[1]]), true, Some(true));
        result.record(serde_json::json!([[2]]), true, Some(false));
        assert_eq!(result.answer, Some(serde_json::json!([[1]])));
        assert_eq!(result.outcome, Some(true));
        result.record(serde_json::json!([[3]]), true, None);
        assert_eq!(result.outcome, None);
        assert_eq!(result.uncertain_answers.len(), 2);
        result.record(serde_json::json!([[4]]), true, Some(true));
        assert_eq!(result.answer, Some(serde_json::json!([[4]])));
        assert!(result.uncertain_answers.is_empty());
    }
}
