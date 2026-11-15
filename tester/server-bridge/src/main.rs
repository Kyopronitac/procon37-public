//! server-bridge: 公式対戦サーバー(HTTP) と JSON Lines v1 ソルバの変換器。
//!
//! 各チームの PC 上で実行し、自身のソルバを公式対戦サーバーへ接続する。
//! 公式API(`/setting` `/agent` `/`)だけを使うので、本番の競技サーバーと
//! 練習用の対戦サーバー(`tester/server`)のどちらにも同じ手順で接続できる。
//! 要求の翻訳と提出だけを行い、結果の計算は行わない(結果はサーバー側で再現される)。
//!
//! 1回の起動で1試合を担当する。試合が公開されるまで待ち、終了を確認してから終わる。

use anyhow::{Context, Result, bail, ensure};
use procon37_common::engine::{Game, validate_kinds};
use procon37_common::model::*;
use procon37_common::process::{CommandSpec, SolverProcess};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

mod record;
mod schedule;
use record::MatchRecord;

/// リクエストの最小間隔。公式の目安「秒間5リクエストほど」(問い合わせと回答その2 A42)に合わせる。
const MIN_INTERVAL: Duration = Duration::from_millis(200);
/// 1回のリクエストの待ち時間。ケーブルが抜けたときなどに、再接続を早く検知できるよう短めにする。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// 締切から差し引く通信の余裕。状態・問題を受け取るまでの遅れと、提出がサーバーへ届くまでの時間を見込む。
/// ソルバ自身の余裕(制限時間の1%)はソルバ内の処理だけを見込むので、LAN越しの通信はここで吸収する。
const NETWORK_MARGIN_MS: u64 = 500;
/// 行動計画の再送を打ち切る、endsAt(秒未満を切り捨てた締切で、実際の締切として最も早い時刻)の手前の時間。
/// 送ってから届くまでの時間を見込み、再送が翌日の計画として受理されないようにする。
const SUBMIT_CUTOFF_MS: u64 = 100;
/// 練習用の対戦サーバーが制限時間を短縮しているときに付ける、車種選択の残り時間。本番のサーバーは付けない。
const SELECT_HINT_HEADER: &str = "procon37-select-remaining-ms";
/// 練習サーバーの選択開始・締切。問題の取得遅延によって提出時刻を後ろへずらさない。
const SELECT_STARTED_HEADER: &str = "procon37-select-started-at-ms";
const SELECT_DEADLINE_HEADER: &str = "procon37-select-deadline-ms";
/// 練習用の対戦サーバーが進行中の試合への応答に付ける試合ID。試合を続けて開くので、同じ問題の試合が続いても
/// 前の試合と次の試合を見分けられる。本番のサーバーは付けない。
const MATCH_ID_HEADER: &str = "procon37-match-id";
/// 車種選択の締切から1日目の開始までの時間は公表されていないので、試合の終了予定を見積もるときは多めに見込む。
const START_DELAY_ALLOWANCE_MS: u64 = 60_000;
/// 試合の終了予定を過ぎても接続できなければ、試合は終わったものとみなす。
const END_ALLOWANCE_MS: u64 = 30_000;
/// トークンが認められないときの終了コード。run.sh はこれを見て再起動をやめる。
const EXIT_AUTH: i32 = 2;

struct Args {
    server: String,
    token: String,
    solver: String,
    solver_args: Vec<String>,
    cwd: PathBuf,
    select_ms: Option<u64>,
    submit_at_ms: Vec<u64>,
    stream_responses: bool,
    record_dir: PathBuf,
}

fn parse_args() -> Result<Args> {
    parse(std::env::args().skip(1).collect())
}

/// `--args` はハイフンで始まらない値だけを取る。ハイフンで始まる引数は `--` 以降に書くと、そのままソルバへ渡す。
fn parse(args: Vec<String>) -> Result<Args> {
    let mut a = Args {
        server: String::new(),
        token: String::new(),
        solver: String::new(),
        solver_args: vec![],
        cwd: std::env::current_dir()?,
        select_ms: None,
        submit_at_ms: vec![],
        stream_responses: false,
        record_dir: PathBuf::from(".server/submissions"),
    };
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        i += 1;
        let value = |name: &str| -> Result<String> {
            args.get(i)
                .cloned()
                .with_context(|| format!("Missing value for {name}"))
        };
        match arg.as_str() {
            "--server" => {
                a.server = value("--server")?;
                i += 1;
            }
            "--token" => {
                a.token = value("--token")?;
                i += 1;
            }
            "--solver" => {
                a.solver = value("--solver")?;
                i += 1;
            }
            "--args" => {
                while i < args.len() && !args[i].starts_with('-') {
                    a.solver_args.push(args[i].clone());
                    i += 1;
                }
            }
            "--cwd" => {
                a.cwd = PathBuf::from(value("--cwd")?);
                i += 1;
            }
            "--record-dir" => {
                a.record_dir = PathBuf::from(value("--record-dir")?);
                ensure!(
                    !a.record_dir.as_os_str().is_empty(),
                    "--record-dir must not be empty"
                );
                i += 1;
            }
            "--select-ms" => {
                a.select_ms = Some(value("--select-ms")?.parse()?);
                i += 1;
            }
            "--submit-at-ms" => {
                a.submit_at_ms = value("--submit-at-ms")?.split(',')
                    .map(str::parse).collect::<std::result::Result<Vec<u64>, _>>()?;
                ensure!(!a.submit_at_ms.is_empty() && a.submit_at_ms[0] > 0
                    && a.submit_at_ms.windows(2).all(|pair| pair[0] < pair[1]),
                    "--submit-at-ms must be a strictly increasing list of positive milliseconds");
                i += 1;
            }
            "--stream-responses" => a.stream_responses = true,
            "--" => {
                a.solver_args.extend(args[i..].iter().cloned());
                break;
            }
            other => bail!("Unknown option: {other}"),
        }
    }
    ensure!(!a.server.is_empty(), "Specify --server URL");
    ensure!(!a.token.is_empty(), "Specify --token TOKEN");
    ensure!(!a.solver.is_empty(), "Specify --solver PATH");
    ensure!(!a.stream_responses || !a.submit_at_ms.is_empty(),
        "--stream-responses requires --submit-at-ms");
    Ok(a)
}

struct Reply {
    status: u16,
    select_hint_ms: Option<u64>,
    select_started_ms: Option<u64>,
    select_deadline_ms: Option<u64>,
    match_id: Option<String>,
    body: Vec<u8>,
}
impl Reply {
    fn selection_window(&self) -> Option<(u64, u64)> {
        self.select_started_ms.zip(self.select_deadline_ms)
            .filter(|(start, end)| *start > 0 && start < end)
    }
    /// 参加した試合(`ours`)と別の試合への応答か。どちらの試合IDも分かるときだけ判断できる。
    fn for_other_match(&self, ours: Option<&str>) -> bool {
        matches!((self.match_id.as_deref(), ours), (Some(id), Some(ours)) if id != ours)
    }
    fn message(&self) -> String {
        serde_json::from_slice::<Value>(&self.body)
            .ok()
            .and_then(|v| v.get("message")?.as_str().map(str::to_owned))
            .unwrap_or_else(|| String::from_utf8_lossy(&self.body).into_owned())
    }
    fn ended(&self) -> bool {
        self.status != 200 && self.message().contains("ended")
    }
    /// 行動計画の提出に対する revision。公式仕様では0以上なら受理、0未満なら却下。
    fn revision(&self) -> Option<i64> {
        serde_json::from_slice::<Value>(&self.body)
            .ok()
            .and_then(|v| v.get("revision").and_then(Value::as_i64))
    }
    fn accepted(&self) -> bool {
        self.status == 200 && self.revision().is_some_and(|r| r >= 0)
    }
    fn not_started(&self) -> bool {
        self.status != 200 && self.message().contains("not started")
    }
    /// 中継(対戦サーバーの対戦窓口など)が上流と通信できなかったことを示す応答。通信エラーと同じく再試行する。
    fn gateway_error(&self) -> bool {
        matches!(self.status, 502..=504)
    }
}

struct Client {
    base: String,
    token: String,
    http: reqwest::Client,
    last: Mutex<Option<Instant>>,
    /// 通信できなくなった時刻。途絶と復旧を1回ずつ表示する。
    outage_since: Mutex<Option<Instant>>,
    connection_seen: AtomicBool,
    /// 締切・送信間隔の確認を通過し、HTTP送信を開始したPOSTだけを数える。
    post_attempts: AtomicU64,
    record: Mutex<Option<MatchRecord>>,
}

impl Client {
    fn new(base: &str, token: &str) -> Result<Self> {
        Ok(Self {
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()?,
            last: Mutex::new(None),
            outage_since: Mutex::new(None),
            connection_seen: AtomicBool::new(false),
            post_attempts: AtomicU64::new(0),
            record: Mutex::new(None),
        })
    }
    async fn start_recording(
        &self,
        args: &Args,
        reply: &Reply,
        seen_ms: u64,
        deadline: u64,
        late: bool,
    ) {
        let problem = serde_json::from_slice::<Value>(&reply.body).unwrap_or(Value::Null);
        let metadata = serde_json::json!({
            "schemaVersion": 1, "receivedAtMs": seen_ms, "server": args.server,
            "matchId": reply.match_id, "selectionDeadlineMs": deadline, "lateJoin": late,
            "selectionStartedAtMs": reply.select_started_ms,
            "selectionDeadlineHeaderMs": reply.select_deadline_ms,
            "selectionStartMs": reply.selection_window().map_or(seen_ms, |(start, _)| start),
            "selectionAnchorSource": if reply.selection_window().is_some() { "server_header" } else { "first_receipt" },
            "submitAtMs": args.submit_at_ms, "streamResponses": args.stream_responses,
        });
        match MatchRecord::create(&args.record_dir, &problem, &metadata, seen_ms) {
            Ok(record) => {
                eprintln!(
                    "[bridge] recording_started directory={}",
                    record.directory.display()
                );
                *self.record.lock().await = Some(record);
                self.record_reply("GET", "/setting", reply).await;
            }
            Err(error) => eprintln!(
                "[bridge] recording_failed directory={} error={error} submissions_continue=true",
                args.record_dir.display()
            ),
        }
    }
    async fn record_event(&self, event: Value) {
        let mut record = self.record.lock().await;
        if let Some(writer) = record.as_mut() {
            if let Err(error) = writer.append(&event) {
                eprintln!(
                    "[bridge] recording_failed directory={} error={error} submissions_continue=true",
                    writer.directory.display()
                );
                *record = None;
            }
        }
    }
    async fn record_reply(&self, method: &str, path: &str, reply: &Reply) {
        self.record_event(serde_json::json!({
            "event": "response", "receivedAtMs": now_ms(), "method": method, "path": path,
            "httpStatus": reply.status, "matchId": reply.match_id, "selectRemainingMs": reply.select_hint_ms,
            "body": serde_json::from_slice::<Value>(&reply.body).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&reply.body).into_owned())),
        })).await;
    }
    /// トークンは `&` や `#` を含んでもそのまま届くよう、クエリとしてエスケープして付ける。
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{}", self.base, path))
            .query(&[("token", &self.token)])
    }
    /// すべてのリクエストを `MIN_INTERVAL` 以上空けて送る。
    async fn pace(&self) {
        let mut last = self.last.lock().await;
        if let Some(at) = *last {
            tokio::time::sleep_until(at + MIN_INTERVAL).await;
        }
        *last = Some(Instant::now());
    }
    /// 送信する。通信の失敗は呼び出し側で再試行する。`deadline_ms` を過ぎていれば、間隔を空けた後に送らずに終える。
    async fn send(
        &self,
        request: reqwest::RequestBuilder,
        deadline_ms: Option<u64>,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Reply> {
        self.pace().await;
        if deadline_ms.is_some_and(|deadline| now_ms() >= deadline) {
            bail!("Submission deadline passed; request was not sent");
        }
        let method = if body.is_some() { "POST" } else { "GET" };
        self.record_event(
            serde_json::json!({"event":"request", "attemptedAtMs":now_ms(),
            "method":method, "path":path, "body":body}),
        )
        .await;
        // Record writes must never allow a late submission to cross into the next day.
        if deadline_ms.is_some_and(|deadline| now_ms() >= deadline) {
            self.record_event(
                serde_json::json!({"event":"not_sent", "receivedAtMs":now_ms(),
                "method":method, "path":path, "reason":"deadline_before_send"}),
            )
            .await;
            bail!("Submission deadline passed; request was not sent");
        }
        // 応答が無いまま打ち切られた場合も途絶に含めるよう、送り始めた時刻から数える。
        let sent = Instant::now();
        if body.is_some() {
            self.post_attempts.fetch_add(1, Ordering::Relaxed);
        }
        let result = self.exchange(request).await;
        match &result {
            Ok(reply) => self.record_reply(method, path, reply).await,
            Err(_) => self.record_event(serde_json::json!({"event":"transport_error", "receivedAtMs":now_ms(), "method":method, "path":path})).await,
        }
        let mut outage = self.outage_since.lock().await;
        match (&result, *outage) {
            (Err(e), None) => {
                eprintln!("[bridge] connection_lost retrying=true error={e}");
                *outage = Some(sent);
            }
            (Ok(reply), None) if reply.gateway_error() => {
                eprintln!(
                    "[bridge] connection_lost retrying=true reason=upstream_unavailable http_status={}",
                    reply.status
                );
                *outage = Some(sent);
            }
            (Ok(reply), Some(since)) if !reply.gateway_error() => {
                eprintln!(
                    "[bridge] connection_restored outage_ms={}",
                    since.elapsed().as_millis()
                );
                *outage = None;
            }
            _ => {}
        }
        if let Ok(reply) = &result {
            if !reply.gateway_error() && !self.connection_seen.swap(true, Ordering::Relaxed) {
                eprintln!(
                    "[bridge] server_connection result=reachable http_status={} request_ms={}",
                    reply.status,
                    sent.elapsed().as_millis()
                );
            }
        }
        result
    }
    async fn exchange(&self, request: reqwest::RequestBuilder) -> Result<Reply> {
        let resp = request.send().await?;
        let status = resp.status().as_u16();
        let header = |name: &str| {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let select_hint_ms = header(SELECT_HINT_HEADER).and_then(|v| v.parse().ok());
        let select_started_ms = header(SELECT_STARTED_HEADER).and_then(|v| v.parse().ok());
        let select_deadline_ms = header(SELECT_DEADLINE_HEADER).and_then(|v| v.parse().ok());
        let match_id = header(MATCH_ID_HEADER);
        let body = resp.bytes().await?.to_vec();
        Ok(Reply {
            status,
            select_hint_ms,
            select_started_ms,
            select_deadline_ms,
            match_id,
            body,
        })
    }
    async fn get(&self, path: &str) -> Result<Reply> {
        self.send(self.request(reqwest::Method::GET, path), None, path, None)
            .await
    }
    async fn post(&self, path: &str, body: &Value, deadline_ms: u64) -> Result<Reply> {
        self.send(
            self.request(reqwest::Method::POST, path)
                .header("Content-Type", "application/json")
                .body(body.to_string()),
            Some(deadline_ms),
            path,
            Some(body),
        )
        .await
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// 本選の車種選択時間。マップの大きさで形式が決まる(本選マップに関するお知らせ)。
fn official_select_ms(problem: &Problem) -> u64 {
    [
        MatchFormat::First,
        MatchFormat::Semifinal,
        MatchFormat::Final,
    ]
    .into_iter()
    .map(MatchFormat::parameters)
    .find(|p| p.0 == problem.map.width && p.0 == problem.map.height)
    .map_or(60_000, |p| p.4)
}

/// その日の締切(`endsAt`)までの残り時間から通信の余裕を引いたもの。締切を過ぎていても1 msは渡す。
fn remaining_ms(state: &Observation, problem: &Problem, day_index: usize) -> u64 {
    let limit = u64::from(problem.day_seconds[day_index]) * 1000;
    (state.ends_at * 1000)
        .saturating_sub(now_ms() + NETWORK_MARGIN_MS)
        .clamp(1, limit.max(1))
}

/// 1日目が始まった後か。`startsAt` は車種選択中は0、試合開始後は1日目の開始時刻(UNIX秒)。
/// 再起動などで途中から参加した場合は、車種選択はもう締め切られている。
fn day_started(problem: &Problem, now_ms: u64) -> bool {
    problem.starts_at != 0 && problem.starts_at * 1000 <= now_ms
}

/// 試合が公開される(`/setting` が200を返す)まで待ち、問題と車種選択の締切(UNIXミリ秒)を返す。
/// 問題は試合開始と同時に取得可能になり、その時点から車種選択の時間が始まる。
/// 1日目が始まった後に参加した場合は、締切を取得した時刻にし、途中参加(true)を返す。試合IDも返す(練習用のみ)。
async fn wait_for_match(
    client: &Client,
    args: &Args,
) -> Result<(Problem, u64, bool, Option<String>, u64)> {
    let mut last_status = None;
    loop {
        match client.get("/setting").await {
            Ok(reply) if reply.status == 200 => {
                let seen_ms = now_ms();
                let problem: Problem = serde_json::from_slice(&reply.body)
                    .context("Failed to parse the problem from /setting")?;
                let late = day_started(&problem, seen_ms);
                let exact_schedule = reply.selection_window();
                let phase_start = exact_schedule.map_or(seen_ms, |(start, _)| start);
                let deadline = match (args.select_ms, reply.select_hint_ms, exact_schedule) {
                    _ if late => seen_ms,
                    (_, _, Some((_, end))) => end,
                    (Some(ms), _, None) => seen_ms + ms,
                    (None, Some(remaining), None) => seen_ms + remaining,
                    (None, None, None) => seen_ms + official_select_ms(&problem),
                };
                client
                    .start_recording(args, &reply, seen_ms, deadline, late)
                    .await;
                return Ok((problem, deadline, late, reply.match_id, phase_start));
            }
            Ok(reply) if reply.status == 401 => {
                eprintln!(
                    "[bridge] authentication_failed http_status={} action=check_TOKEN_or_TEAM",
                    reply.status
                );
                std::process::exit(EXIT_AUTH);
            }
            Ok(reply) => {
                if last_status != Some(reply.status) {
                    eprintln!(
                        "[bridge] problem_wait http_status={} state={}",
                        reply.status,
                        if reply.gateway_error() {
                            "upstream_unavailable"
                        } else {
                            "not_ready"
                        }
                    );
                    last_status = Some(reply.status);
                }
            }
            Err(_) => {}
        }
    }
}

/// `day_index` 日目以降の状態を取得する。試合が終わっていれば None を返す。
/// 通信できない間は再接続を待ち続ける(ケーブルの抜けなどから復旧すれば、その時点の日から回答を続ける)。
/// 最後に回答した状態(`last`: 日と締切 endsAt)と同じ日・締切の状態なら、日が進むのを待つ。それ以外の前の日の状態や、
/// 試合が始まった後の開始前(403)は、対戦サーバーが続けて開いた次の試合のものなので、この試合は終わったものとみなす。
/// 最初の状態(`last` が無い)を含め、締切がこの試合の日程から遅すぎる状態も次の試合のものとみなす(`owns`)。
/// 試合ID(`match_id`、練習用の対戦サーバーだけが付ける)が違う応答も次の試合のものとみなす。
/// どの応答でも、試合の終了予定(`end_ms`)を過ぎれば終わったものとみなす(次の試合の状態を待ち続けないように)。
async fn fetch_state(
    client: &Client,
    day_index: usize,
    last: Option<(usize, u64)>,
    schedule: (&[u32], u64),
    match_id: Option<&str>,
    end_ms: u64,
) -> Option<Observation> {
    loop {
        match client.get("/").await {
            Ok(reply) if reply.for_other_match(match_id) => return None,
            Ok(reply) if reply.status == 200 => {
                match serde_json::from_slice::<Observation>(&reply.body) {
                    Ok(obs) if obs.day >= day_index => return owns(schedule, &obs).then_some(obs),
                    Ok(obs) if Some((obs.day, obs.ends_at)) != last => return None,
                    _ => {}
                }
            }
            Ok(reply) if reply.ended() => return None,
            Ok(reply) if last.is_some() && reply.not_started() => return None,
            _ => {}
        }
        if now_ms() > end_ms {
            eprintln!("[bridge] match_end reason=expected_end_time_passed");
            return None;
        }
    }
}

/// 対戦サーバーが公開中の試合が、参加した試合(`problem`)と同じか。`/setting` の問題で比べる
/// (`startsAt` は試合開始で変わるので除く)。問題を取得できなければ同じとみなす。試合IDの無いサーバー(本番)用。
async fn same_match(client: &Client, problem: &Problem) -> bool {
    match client.get("/setting").await {
        Ok(reply) if reply.status == 200 => {
            serde_json::from_slice::<Problem>(&reply.body).map_or(true, |mut current| {
                current.starts_at = problem.starts_at;
                current == *problem
            })
        }
        _ => true,
    }
}

/// 状態がこの試合のものか。次の試合はこの試合が終わった後に始まるので、同じ日の締切がこの試合の全日程ぶん以上遅い。
/// そこで、この試合の開始の遅い方の見積もり(`start_ms`)に全日程とその日までの日程(`day_seconds`)を足した時刻より
/// 締切が前なら、この試合の状態とする(日と日の間が空いても、全日程ぶん空かない限りこの試合を取り違えない)。
fn owns((day_seconds, start_ms): (&[u32], u64), obs: &Observation) -> bool {
    let total: u64 = day_seconds.iter().map(|&s| u64::from(s)).sum();
    let until: u64 = day_seconds
        .iter()
        .take(obs.day + 1)
        .map(|&s| u64::from(s))
        .sum();
    obs.ends_at * 1000 < start_ms + (total + until) * 1000
}

/// 応答を受け取れず採否が分からない計画が、いつ実行されたか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Applied {
    /// 実行されなかった(全車待機と区別できない場合を含む)。
    No,
    /// 提出した日から何日後(0 は提出した日)に実行されたか。1日後以降は、送信が遅れて締切の後に処理され、
    /// その日を観測できず全車待機で上書きできなかった場合。
    On(usize),
    /// 実行されたかもしれないが、判断できない。全車待機でも計画の実行でも観測を説明できる(往復して獲得した場合
    /// など、観測に表れない獲得・在庫が変わる)か、対戦で道路状態の分からない飛ばした日に実行されたかもしれない
    /// (観測できなかった日の道路状態は他チームの滞在で決まるので、その日の車の動きを再現できない)。
    Unknown,
}

/// 控えの状態が観測した状態と一致するか。比べるのは車の位置・燃料と、1チームの試合(`others` が空)では道路状態
/// (ソルバが自分の滞在から求めて検査するため)。
fn reproduces(game: &Game, observed: &Observation) -> bool {
    game.agents == observed.agents
        && (!observed.others.is_empty() || game.traffic() == observed.traffics)
}

/// 応答を受け取れず採否が分からない計画が、いつ実行されたか。控え(`shadow`、前日までの履歴を持つ)を前日の状態
/// (`before`)にそろえ、実行しなかった場合と、次に観測した日(`after`)の前日までの各日に実行した場合のそれぞれに
/// ついて、ほかの日(通信の途絶で飛ばした日を含む)を全車待機で進め、次に観測した状態と比べる。比べるのは車の位置・
/// 燃料と、1チームの試合(`others` が空)では道路状態(ソルバが自分の滞在から求めて検査するため。同じ位置・燃料に
/// 戻っても道路の滞在が変われば見分けられる)。実行しなかった場合でも説明できるときは、実行されなかったものとして
/// 進める(ソルバは待機として状態を検査できるので止まらない)。計画の実行でも説明できれば、判断できない
/// (`Applied::Unknown`)として知らせる。複数の日で一致するときは
/// 最も早い日とする(提出した日なら、ソルバに受理と伝えればソルバの状態が観測と一致する)。対戦(`others` あり)では、
/// 道路状態を観測した提出した日だけを試し、日を飛ばしていれば、提出した日に実行されたと分かる場合のほかは判断しない
/// (`Applied::Unknown`。全車待機で観測と一致しても、飛ばした日に往復して獲得した場合を除けない)。道路状態は移動の
/// 所要時間と燃料に効くが、観測できなかった日の道路状態は他チームの滞在で決まり、自チームの予測では再現できない。
fn plan_applied(shadow: &Game, before: &Observation, plan: &Value, after: &Observation) -> Applied {
    let Ok(plan) = serde_json::from_value::<Plan>(plan.clone()) else {
        return Applied::No;
    };
    // 提出した日から `on` 日後に計画を実行し(None は実行しない)、ほかの日を全車待機で進めて観測と一致するか。
    let matches = |on: Option<usize>| -> bool {
        let mut game = shadow.clone();
        game.agents = before.agents.clone();
        if !before.others.is_empty() {
            game.set_observed_traffic(before.traffics.clone());
        }
        let mut offset = 0;
        while game.day < after.day {
            let today = if on == Some(offset) {
                plan.clone()
            } else {
                game.waiting_plan()
            };
            if game.play_day(today).is_err() {
                return false;
            }
            offset += 1;
        }
        reproduces(&game, after)
    };
    if after.day <= before.day {
        return Applied::No;
    }
    let versus = !before.others.is_empty();
    // 道路状態の分からない飛ばした日(対戦で日を飛ばした場合)があり、そこで実行されたかは試せない。
    let hidden = versus && after.day > before.day + 1;
    // 実行された日として試せるのは、対戦では道路状態を観測した提出した日だけ。
    let tried = if versus {
        0..1
    } else {
        0..after.day - before.day
    };
    if matches(None) {
        // 全車待機の計画そのものなら、どちらでも同じ。
        let ambiguous = plan != shadow.waiting_plan()
            && (hidden || tried.into_iter().any(|o| matches(Some(o))));
        return if ambiguous {
            Applied::Unknown
        } else {
            Applied::No
        };
    }
    match tried.into_iter().find(|&offset| matches(Some(offset))) {
        Some(offset) => Applied::On(offset),
        None if hidden => Applied::Unknown,
        None => Applied::No,
    }
}

/// 提出の応答から、その日の計画としての採否を決める。受理の応答を受け取ったのが endsAt(実際の締切として
/// 最も早い時刻)以降なら、公式対戦サーバーは翌日の計画として適用したかもしれないので、不明(None)として
/// 翌日の状態で確かめる。送信の打ち切りは送り始める時刻だけなので、応答が締切をまたぐことがある。
fn outcome_of(accepted: bool, received_ms: u64, ends_at: u64) -> Option<bool> {
    (!accepted || received_ms < ends_at * 1000).then_some(accepted)
}

/// 控えを1日進める。その日の始めの状態(`observed`)にそろえ、受理された計画(無ければ全車待機)で進める。
/// 対戦(他チームあり)では、その日に観測した道路状態を使う。
fn advance(game: &mut Game, observed: &Observation, plan: Option<Plan>) {
    game.agents = observed.agents.clone();
    if !observed.others.is_empty() {
        game.set_observed_traffic(observed.traffics.clone());
    }
    if plan.is_none_or(|plan| game.play_day(plan).is_err()) {
        let wait = game.waiting_plan();
        let _ = game.play_day(wait);
    }
}

/// 飛ばした日(控えの日から `state.day` の前日まで)を全車待機で進めた状態を控えで作り、ソルバへ順に渡す要求。
/// ソルバは日を1日ずつ進めるので、途中から参加した・何日も通信が途絶えた場合もこれで実際の日まで追いつかせる。
/// 回答は使わないので、制限時間は最小にする。最初の要求だけ、直前に回答した日の採否(`previous_accepted`)を伝える。
/// 前に回答した計画が飛ばした日に実行された(`retained`: その日と計画)ときは、控えではその日を計画どおりに進める。
fn catch_up_requests(
    game: &mut Game,
    state: &Observation,
    mut previous_accepted: bool,
    mut retained: Option<(usize, Plan)>,
) -> Vec<Request> {
    let mut requests = vec![];
    while game.day < state.day {
        let observed = Observation {
            ends_at: state.ends_at,
            day: game.day,
            agents: game.agents.clone(),
            others: state.others.clone(),
            traffics: if state.others.is_empty() {
                game.traffic()
            } else {
                state.traffics.clone()
            },
        };
        requests.push(Request {
            protocol_version: PROTOCOL_VERSION,
            request_id: (game.day + 1) as u64,
            phase: "plan".into(),
            time_limit_ms: 1,
            problem: None,
            state: Some(observed.clone()),
            steps: Some(game.problem.day_steps[game.day]),
            previous_accepted: Some(previous_accepted),
        });
        previous_accepted = false;
        let plan = retained
            .take_if(|(day, _)| *day == game.day)
            .map(|(_, plan)| plan);
        advance(game, &observed, plan);
    }
    requests
}

async fn catch_up(
    proc: &mut SolverProcess,
    game: &mut Game,
    state: &Observation,
    previous_accepted: bool,
    retained: Option<(usize, Plan)>,
    streaming: bool,
    mut previous_answer: Option<&Value>,
) {
    for request in catch_up_requests(game, state, previous_accepted, retained) {
        if streaming {
            let mut wire = serde_json::to_value(&request).unwrap();
            wire["previousAnswer"] = previous_answer.take().cloned().unwrap_or(Value::Null);
            let exchange = async {
                proc.begin_stream(&wire).await.ok()?;
                loop {
                    let response = proc.next_stream_response().await.ok()?;
                    if response.request_id == request.request_id && response.is_final { return Some(()); }
                }
            };
            let _ = tokio::time::timeout(Duration::from_millis(500), exchange).await;
        } else {
            let _ = proc.exchange(&request).await;
        }
    }
}

/// 最後に回答した日(`last`: 日と締切 endsAt)の状態が返る間だけ待ち、この試合の終了を確認する。
/// 終了の応答に限らず、次の試合の開始前(403)や別の日・締切の状態が返れば、この試合は終わっている
/// (対戦サーバーは試合を続けて開くため、次の試合の状態を待ち続けてその試合に参加し損ねないように)。
/// 試合の終了予定(`end_ms`)を過ぎても通信できなければ、終わったものとみなす。
async fn wait_for_end(
    client: &Client,
    last: Option<(usize, u64)>,
    match_id: Option<&str>,
    end_ms: u64,
) {
    loop {
        match client.get("/").await {
            Ok(reply) if reply.for_other_match(match_id) => return,
            Ok(reply) if reply.status == 200 => {
                let same = serde_json::from_slice::<Observation>(&reply.body)
                    .is_ok_and(|obs| Some((obs.day, obs.ends_at)) == last);
                if !same {
                    return;
                }
            }
            Ok(reply) if !reply.gateway_error() => return,
            _ if now_ms() > end_ms => return,
            _ => {}
        }
    }
}

/// 提出の結果。
struct Submitted {
    /// 最後に受け取った応答と、受け取った時刻。締切まで受け取れなければ None。
    reply: Option<(Reply, u64)>,
    /// それより前に、応答を受け取れず採否が分からない試みがあったか。
    lost: bool,
    elapsed_ms: u64,
    attempts: u64,
    /// 送信前の締切判定だけで終わった試行は含まない。
    http_attempts: u64,
}
impl Submitted {
    /// その日の計画としての採否(`outcome_of`)。応答を受け取れなかった場合は不明(None)。却下の応答でも、
    /// 先に応答を受け取れなかった試みがあれば、それが受理されたかもしれないので不明とする(受理の応答なら、
    /// 同じ内容の再送が受理されたので、先の試みの採否によらない)。
    fn outcome(&self, ends_at: u64) -> Option<bool> {
        let (reply, received_ms) = self.reply.as_ref()?;
        let accepted = reply.accepted();
        if !accepted && self.lost {
            return None;
        }
        outcome_of(accepted, *received_ms, ends_at)
    }
}

/// 提出する。通信できない間(中継の502〜504を含む)は締切(`deadline_ms`)まで再送する。再提出は公式に認められている。
/// 締切を過ぎてからは送らない。
async fn submit(client: &Client, path: &str, body: &Value, deadline_ms: u64) -> Submitted {
    let started = Instant::now();
    let http_before = client.post_attempts.load(Ordering::Relaxed);
    let mut attempts = 0;
    let mut lost = false;
    loop {
        if now_ms() >= deadline_ms {
            eprintln!(
                "[bridge] submission_end path={path} result={} reason=deadline attempts={attempts} submit_ms={}",
                if attempts == 0 {
                    "not_submitted"
                } else {
                    "unknown"
                },
                started.elapsed().as_millis()
            );
            return Submitted {
                reply: None,
                lost,
                elapsed_ms: started.elapsed().as_millis() as u64,
                attempts,
                http_attempts: client.post_attempts.load(Ordering::Relaxed) - http_before,
            };
        }
        attempts += 1;
        eprintln!(
            "[bridge] submission_attempt path={path} attempt={attempts} remaining_ms={}",
            deadline_ms.saturating_sub(now_ms())
        );
        match client.post(path, body, deadline_ms).await {
            Ok(reply) if !reply.gateway_error() => {
                let received_ms = now_ms();
                let elapsed_ms = started.elapsed().as_millis() as u64;
                eprintln!(
                    "[bridge] submission_response path={path} http_status={} revision={} attempts={attempts} submit_ms={}",
                    reply.status,
                    reply.revision().map_or("none".into(), |r| r.to_string()),
                    elapsed_ms
                );
                return Submitted {
                    reply: Some((reply, received_ms)),
                    lost,
                    elapsed_ms,
                    attempts,
                    http_attempts: client.post_attempts.load(Ordering::Relaxed) - http_before,
                };
            }
            Ok(reply) => {
                eprintln!(
                    "[bridge] submission_retry path={path} http_status={} reason=upstream_unavailable",
                    reply.status
                );
                lost = true;
            }
            Err(_) => {
                eprintln!(
                    "[bridge] submission_retry path={path} reason=transport_error_or_deadline"
                );
                lost = true;
            }
        }
    }
}

fn acceptance_label(outcome: Option<bool>) -> &'static str {
    match outcome {
        Some(true) => "accepted",
        Some(false) => "rejected",
        None => "unknown",
    }
}

/// 時刻 `from_ms` から `next_day` 日目以降をすべて進めたときの、試合の終了予定。
fn match_end_ms(problem: &Problem, next_day: usize, from_ms: u64) -> u64 {
    let rest: u64 = problem.day_seconds[next_day.min(problem.day_seconds.len())..]
        .iter()
        .map(|&s| u64::from(s))
        .sum();
    from_ms + rest * 1000 + END_ALLOWANCE_MS
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args()?;
    let client = Client::new(&args.server, &args.token)?;

    // 1. 試合の公開を待って問題を取得する
    eprintln!(
        "[bridge] connection_check server={} path=/setting",
        args.server
    );
    eprintln!("[bridge] waiting_for_problem");
    let (problem, select_deadline_ms, late, match_id, selection_start_ms) = wait_for_match(&client, &args).await?;
    let match_started = Instant::now();
    let match_id = match_id.as_deref();
    let brands = problem
        .spots
        .iter()
        .map(|s| s.brand)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    eprintln!(
        "[bridge] problem_received width={} height={} agents={} days={} players={} spots={} brands={} fuel_limit={} busy_threshold={} jammed_threshold={}",
        problem.map.width,
        problem.map.height,
        problem.agents.len(),
        problem.day_steps.len(),
        problem.players,
        problem.spots.len(),
        brands,
        problem.fuel_limits,
        problem.busy_threshold,
        problem.jammed_threshold
    );
    eprintln!(
        "[bridge] schedule day_steps={:?} day_seconds={:?} selection_remaining_ms={} starts_at={} late_join={} match_id={}",
        problem.day_steps,
        problem.day_seconds,
        select_deadline_ms.saturating_sub(now_ms()),
        problem.starts_at,
        late,
        match_id.unwrap_or("none")
    );

    // 2. ソルバ起動
    let spec = CommandSpec {
        program: args.solver.clone(),
        args: args.solver_args.clone(),
        cwd: args.cwd.clone(),
        env: BTreeMap::new(),
    };
    let mut proc = SolverProcess::spawn_inherit_stderr(&spec)
        .await
        .context("Failed to start the solver")?;
    eprintln!("[bridge] solver_started program={}", args.solver);

    // 3. 車種選択。1日目が始まった後に参加した場合は、ソルバへ問題を渡すだけで提出しない(締切済み)。
    let mut previous_answer: Option<Value> = None;
    let selection_accepted = if !args.submit_at_ms.is_empty() && !late {
        let targets = schedule::targets(selection_start_ms, &args.submit_at_ms, select_deadline_ms);
        let request = Request {
            protocol_version: PROTOCOL_VERSION, request_id: 0, phase: "select".into(),
            time_limit_ms: schedule::budget(&targets, now_ms()), problem: Some(problem.clone()),
            state: None, steps: None, previous_accepted: None,
        };
        eprintln!("[bridge] solve_start phase=select request_id=0 limit_ms={}", request.time_limit_ms);
        let result = schedule::run(&mut proc, &client, &request, &targets, selection_start_ms, select_deadline_ms,
            args.stream_responses, None, serde_json::json!(vec![0; problem.agents.len()]), None).await?;
        let accepted = result.outcome == Some(true) && result.from_solver;
        if accepted { previous_answer = result.answer; }
        eprintln!("[bridge] selection_submission result={} scheduled=true", acceptance_label(result.outcome));
        accepted
    } else {
    let select_limit_ms = select_deadline_ms
        .saturating_sub(now_ms() + NETWORK_MARGIN_MS)
        .max(1);
    let select_request = Request {
        protocol_version: PROTOCOL_VERSION,
        request_id: 0,
        phase: "select".into(),
        time_limit_ms: select_limit_ms,
        problem: Some(problem.clone()),
        state: None,
        steps: None,
        previous_accepted: None,
    };
    eprintln!("[bridge] solve_start phase=select request_id=0 limit_ms={select_limit_ms}");
    let (answer, failure, elapsed) = proc.exchange(&select_request).await;
    let chosen = answer
        .as_ref()
        .and_then(|v| validate_kinds(v, problem.agents.len()).ok());
    eprintln!(
        "[bridge] solve_end phase=select solver_ms={elapsed} limit_ms={select_limit_ms} result={} fallback={} failure_kind={}",
        if chosen.is_some() { "ok" } else { "failed" },
        chosen.is_none(),
        failure.as_ref().map_or(
            if answer.is_some() {
                "invalid_answer"
            } else {
                "none"
            },
            |f| f.kind.as_str()
        )
    );
    let kinds = serde_json::to_value(
        chosen
            .clone()
            .unwrap_or_else(|| vec![0; problem.agents.len()]),
    )?;
    eprintln!(
        "[bridge] vehicle_selection kinds={kinds} fallback={}",
        chosen.is_none()
    );
    // ソルバが選んだ車種がそのまま受理されたときだけ true。全車0で代替した・却下された・提出しなかった場合は
    // 1日目の previousAccepted を false にし、ソルバが状態から実際の車種を読み直せるようにする。
    let mut selection_accepted = false;
    match (!late).then_some(submit(&client, "/agent", &kinds, select_deadline_ms)) {
        None => {
            eprintln!(
                "[bridge] selection_submission result=not_submitted reason=late_join solver_ms={elapsed}"
            )
        }
        Some(submission) => match submission.await {
            Submitted {
                reply: Some((reply, _)),
                elapsed_ms,
                attempts,
                ..
            } if reply.status == 200 => {
                selection_accepted = chosen.is_some();
                eprintln!(
                    "[bridge] selection_submission result=accepted http_status={} solver_ms={elapsed} submit_ms={elapsed_ms} attempts={attempts} fallback={}",
                    reply.status,
                    chosen.is_none()
                )
            }
            // 先に応答を受け取れなかった提出が受理されたかもしれないが、1日目の previousAccepted を false にするので、
            // ソルバは状態から実際の車種を読み直す。
            Submitted {
                reply: Some((reply, _)),
                lost,
                elapsed_ms,
                attempts,
                ..
            } => eprintln!(
                "[bridge] selection_submission result={} http_status={} solver_ms={elapsed} submit_ms={elapsed_ms} attempts={attempts} reread_vehicle_kinds=true",
                if lost { "unknown" } else { "rejected" },
                reply.status
            ),
            Submitted {
                reply: None,
                elapsed_ms,
                attempts,
                ..
            } => {
                eprintln!(
                    "[bridge] selection_submission result={} reason=deadline solver_ms={elapsed} submit_ms={elapsed_ms} attempts={attempts}",
                    if attempts == 0 {
                        "not_submitted"
                    } else {
                        "unknown"
                    }
                )
            }
        },
    }

        selection_accepted
    };

    // 4. 日別ループ。回答が遅れて日が進んだ場合は、その日を飛ばして現在の日に回答する。
    let days = problem.day_steps.len();
    let start_ms = if late {
        problem.starts_at * 1000
    } else {
        select_deadline_ms + START_DELAY_ALLOWANCE_MS
    };
    let mut end_ms = match_end_ms(&problem, 0, start_ms);
    let mut previous_accepted = selection_accepted;
    let mut day_index = 0;
    // 最後に回答した日の状態(日と締切)。状態の取得と終了の確認で、この試合の状態かを見分ける。
    let mut last = None;
    let mut ended = false;
    // 直前に回答した日の状態・提出した計画・採否(None は応答を受け取れず不明)。翌日の状態で採否を確かめる。
    let mut answered: Option<(Observation, Option<Value>, Option<bool>)> = None;
    let mut uncertain_answers: Vec<Value> = Vec::new();
    // ソルバの内部と同じく試合を進める控え。日を飛ばしたとき、飛ばした日の状態を作ってソルバを追いつかせる。
    let mut shadow: Option<Game> = None;
    while day_index < days {
        let schedule = (problem.day_seconds.as_slice(), start_ms);
        let Some(state) = fetch_state(&client, day_index, last, schedule, match_id, end_ms).await
        else {
            eprintln!("[bridge] match_end unanswered_from_day={}", day_index + 1);
            ended = true;
            break;
        };
        if state.day >= days {
            ended = true;
            break;
        }
        // 最初の状態は、公開中の試合が参加した試合と同じかを問題で確かめる(途絶の間に次の試合が始まっていないか)。
        // 試合IDが分かる練習用の対戦サーバーでは状態の取得で確かめ済みなので、問題の取り直しを省いて時間を節約する。
        if last.is_none() && match_id.is_none() && !same_match(&client, &problem).await {
            eprintln!("[bridge] match_end reason=next_match_started");
            ended = true;
            break;
        }
        last = Some((state.day, state.ends_at));
        client
            .record_event(
                serde_json::json!({"event":"observation", "usedAtMs":now_ms(), "state":state}),
            )
            .await;
        let kinds: Vec<u8> = state.agents.iter().map(|a| a.kind).collect();
        let game = shadow.get_or_insert_with(|| Game::new(problem.clone(), &kinds));
        // 前に回答した提出の採否が不明なら、送信が遅れて今日の計画として受理されている(まだ実行されていない)
        // かもしれない。日を何日飛ばしても、終わった日に実行されたと分かっても(再送した別の送信が遅れたかもしれない)
        // 同じ。
        let mut guard_next = false;
        // 前に回答した計画が、飛ばした日に実行された場合のその日と計画。
        let mut retained = None;
        if let Some((before, mut plan, outcome)) = answered.take() {
            let had_unknown_submission = outcome.is_none() && plan.is_some();
            if outcome.is_none() && uncertain_answers.len() > 1 {
                let matching: Vec<_> = uncertain_answers.iter()
                    .map(|answer| (answer, plan_applied(game, &before, answer, &state))).collect();
                let definite: Vec<_> = matching.iter().filter(|(_, applied)| *applied == Applied::On(0)).collect();
                if definite.len() == 1 && matching.iter().all(|(_, applied)| *applied != Applied::Unknown) {
                    plan = Some(definite[0].0.clone());
                    // Keep unknown through guard_next so delayed network writes are overwritten.
                } else {
                    // Different plans can end at identical positions while collecting different shops.
                    plan = None;
                    eprintln!("[bridge] warning day={} multiple_submission_outcomes_unknown=true solver_state_may_differ=true", before.day + 1);
                }
            }
            uncertain_answers.clear();
            guard_next = had_unknown_submission && state.day > before.day;
            let applied = match (outcome, &plan) {
                (Some(true), _) => Applied::On(0),
                (Some(false), _) | (None, None) => Applied::No,
                (None, Some(plan)) => {
                    let applied = plan_applied(game, &before, plan, &state);
                    eprintln!(
                        "[bridge] submission_confirmed day={} result={}",
                        before.day + 1,
                        match applied {
                            Applied::No => "not_applied".into(),
                            Applied::On(0) => "accepted".into(),
                            Applied::On(offset) =>
                                format!("applied_late_on_day_{}", before.day + offset + 1),
                            Applied::Unknown =>
                                "unknown; indistinguishable_from_waiting_or_unobserved_day".into(),
                        }
                    );
                    applied
                }
            };
            previous_accepted = applied == Applied::On(0);
            previous_answer = if previous_accepted { plan.clone() } else { None };
            let plan = plan.and_then(|plan| serde_json::from_value::<Plan>(plan).ok());
            let (today, later) = match applied {
                Applied::No | Applied::Unknown => (None, None),
                Applied::On(0) => (plan, None),
                Applied::On(offset) => (None, plan.map(|plan| (before.day + offset, plan))),
            };
            if let Some((day, _)) = &later {
                // その日に全車待機で上書きする機会が無かった。同梱ソルバには自分の前日の回答の採否しか伝えられず、
                // 前に回答した計画が後の日に実行されたことは伝えられない。控えは実際どおりに進める。
                eprintln!(
                    "[bridge] warning day={} delayed_plan_applied=true solver_state_may_differ=true",
                    day + 1
                );
            }
            if applied == Applied::Unknown {
                // 実行された日も動きも分からないので、控えとソルバは全車待機として進める。
                eprintln!(
                    "[bridge] warning day={} acceptance_unknown=true assuming_wait=true solver_state_may_differ=true",
                    before.day + 1
                );
            }
            advance(game, &before, today);
            retained = later;
        }
        if state.day > game.day {
            eprintln!(
                "[bridge] catch_up skipped_days={}-{} assuming_wait=true resume_day={}",
                game.day + 1,
                state.day,
                state.day + 1
            );
            catch_up(&mut proc, game, &state, previous_accepted, retained, args.stream_responses, previous_answer.as_ref()).await;
            previous_accepted = false;
            previous_answer = None;
            day_index = state.day;
            if !reproduces(game, &state) {
                // 途中で再起動した場合など、この変換器が知らない計画が飛ばした日より前に実行されていた。
                // ソルバには自分の前日の回答の採否しか伝えられず、実行された計画を伝える手段が無い。
                eprintln!(
                    "[bridge] warning day={} observed_state_mismatch=true solver_may_fail_remaining_days=true",
                    state.day + 1
                );
            }
        }

        // endsAt は秒未満が切り捨てられているので、実際の締切は endsAt から最大1秒遅い。
        // 終了予定は遅い方で見積もり、提出は早い方の手前で打ち切る(翌日の計画として受理されないように)。
        let day_end_ms = state.ends_at * 1000 + 1000;
        let submit_deadline_ms = (state.ends_at * 1000).saturating_sub(SUBMIT_CUTOFF_MS);
        let ends_at = state.ends_at;
        end_ms = match_end_ms(&problem, day_index + 1, day_end_ms);
        // 前に回答した提出の採否が不明だった(`guard_next`)ときは、それがこの日の計画として残っていないよう、まず
        // 全車待機を提出して上書きしておく。ソルバの回答が受理されればそれが優先され、回答が無い・却下されても実行されるのは
        // 全車待機になり、控えとソルバの想定(待機)と一致する。
        if guard_next {
            let wait = serde_json::to_value(game.waiting_plan())?;
            let placed = submit(&client, "/", &wait, submit_deadline_ms)
                .await
                .outcome(ends_at)
                == Some(true);
            if placed {
                eprintln!(
                    "[bridge] safety_submission day={} result=accepted plan=all_wait reason=previous_acceptance_unknown",
                    day_index + 1
                );
            } else {
                // 同梱ソルバには自分の前日の回答の採否しか伝えられないため、前に回答した計画が今日実行されると
                // ソルバの状態を合わせる手段が無い。上書きを確かめられなかったことをはっきり残す。
                eprintln!(
                    "[bridge] warning day={} safety_submission_result=unknown solver_state_may_differ=true",
                    day_index + 1
                );
            }
        }
        if !args.submit_at_ms.is_empty() {
            eprintln!("[bridge] day_state day={} ends_at={} remaining_ms={} previous_accepted={previous_accepted}",
                day_index + 1, state.ends_at, (state.ends_at * 1000).saturating_sub(now_ms()));
            let phase_start = (state.ends_at * 1000)
                .saturating_sub(u64::from(problem.day_seconds[day_index]) * 1000);
            let targets = schedule::targets(phase_start, &args.submit_at_ms, state.ends_at * 1000);
            let request = Request {
                protocol_version: PROTOCOL_VERSION, request_id: (day_index + 1) as u64,
                phase: "plan".into(), time_limit_ms: schedule::budget(&targets, now_ms()),
                problem: None, state: Some(state.clone()), steps: Some(problem.day_steps[day_index]),
                previous_accepted: Some(previous_accepted),
            };
            eprintln!("[bridge] solve_start phase=plan day={} request_id={} limit_ms={}",
                day_index + 1, request.request_id, request.time_limit_ms);
            let result = schedule::run(&mut proc, &client, &request, &targets, phase_start, submit_deadline_ms,
                args.stream_responses, previous_answer.as_ref(), serde_json::to_value(game.waiting_plan())?, Some(game)).await?;
            eprintln!("[bridge] plan_submission day={} result={} scheduled=true", day_index + 1, acceptance_label(result.outcome));
            uncertain_answers = result.uncertain_answers;
            // All-wait fallback is equivalent to rejecting the solver's answer.
            let outcome = if result.from_solver { result.outcome } else { Some(false) };
            answered = Some((state, result.answer, outcome));
            day_index += 1;
            continue;
        }
        let limit = remaining_ms(&state, &problem, day_index);
        eprintln!(
            "[bridge] day_state day={} ends_at={} remaining_ms={} previous_accepted={previous_accepted}",
            day_index + 1,
            state.ends_at,
            (state.ends_at * 1000).saturating_sub(now_ms())
        );
        let before = state.clone();
        let request = Request {
            protocol_version: PROTOCOL_VERSION,
            request_id: (day_index + 1) as u64,
            phase: "plan".into(),
            time_limit_ms: limit,
            problem: None,
            state: Some(state),
            steps: Some(problem.day_steps[day_index]),
            previous_accepted: Some(previous_accepted),
        };
        eprintln!(
            "[bridge] solve_start phase=plan day={} request_id={} steps={} limit_ms={limit}",
            day_index + 1,
            request.request_id,
            problem.day_steps[day_index]
        );
        let (answer, failure, elapsed) = proc.exchange(&request).await;
        eprintln!(
            "[bridge] solve_end phase=plan day={} solver_ms={elapsed} limit_ms={limit} result={} failure_kind={}",
            day_index + 1,
            if answer.is_some() { "ok" } else { "failed" },
            failure.as_ref().map_or("none", |f| f.kind.as_str())
        );
        let mut outcome = Some(false);
        if let Some(value) = &answer {
            let submitted = submit(&client, "/", value, submit_deadline_ms).await;
            outcome = submitted.outcome(ends_at);
            match &submitted.reply {
                Some((reply, _)) => {
                    let revision = reply.revision();
                    eprintln!(
                        "[bridge] plan_submission day={} solver_ms={} http_status={} revision={} result={} http_accepted={} submit_ms={} attempts={}",
                        day_index + 1,
                        elapsed,
                        reply.status,
                        revision.map_or("-".into(), |r| r.to_string()),
                        acceptance_label(outcome),
                        reply.accepted(),
                        submitted.elapsed_ms,
                        submitted.attempts
                    );
                }
                None => {
                    eprintln!(
                        "[bridge] plan_submission day={} result={} reason=deadline solver_ms={elapsed} submit_ms={} attempts={} confirm_from_next_state=true",
                        day_index + 1,
                        if submitted.attempts == 0 {
                            "not_submitted"
                        } else {
                            "unknown"
                        },
                        submitted.elapsed_ms,
                        submitted.attempts
                    );
                }
            }
        } else if let Some(f) = &failure {
            eprintln!(
                "[bridge] plan_submission day={} result=not_submitted reason=solver_failed failure_kind={} solver_ms={elapsed}",
                day_index + 1,
                f.kind
            );
        }
        answered = Some((before, answer, outcome));
        day_index += 1;
    }
    proc.finish().await;
    eprintln!("[bridge] solver_exited");

    // 5. 試合の終了を確認してから終わる(同じ試合に再参加しないように)。
    if !ended {
        wait_for_end(&client, last, match_id, end_ms).await;
    }
    eprintln!(
        "[bridge] match_complete elapsed_ms={}",
        match_started.elapsed().as_millis()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_deadline_while_pacing_is_not_an_actual_submission_attempt() {
        let (base, received) = scripted(vec![(200, r#"{"revision":1}"#)]);
        let client = Client::new(&base, "test").unwrap();
        *client.last.lock().await = Some(Instant::now());
        let answer = serde_json::json!([[-1]]);
        let not_sent = submit(&client, "/", &answer, now_ms() + 10).await;
        assert_eq!(not_sent.http_attempts, 0);
        assert!(received.try_recv().is_err());
        // The same content must still be eligible once an actual send is possible.
        let sent = submit(&client, "/", &answer, now_ms() + 1000).await;
        assert_eq!(sent.http_attempts, 1);
        assert!(sent.reply.as_ref().unwrap().0.accepted());
        assert!(received.recv().unwrap().starts_with("POST /?"));
    }

    #[test]
    fn select_time_follows_the_official_map_sizes() {
        let mut problem = procon37_common::generator::generate(0, Some(MatchFormat::First))
            .unwrap()
            .problem;
        let mut sized = |n: usize| {
            problem.map.width = n;
            problem.map.height = n;
            official_select_ms(&problem)
        };
        assert_eq!(sized(16), 60_000);
        assert_eq!(sized(24), 90_000);
        assert_eq!(sized(32), 120_000);
        assert_eq!(sized(20), 60_000);
    }

    #[test]
    fn joining_after_day_one_skips_the_closed_selection() {
        let mut problem = procon37_common::generator::generate(0, Some(MatchFormat::First))
            .unwrap()
            .problem;
        problem.starts_at = 0;
        assert!(!day_started(&problem, 1_000_000));
        // 本番で車種選択中に1日目の開始予定が入っていても、始まるまでは選択を待つ。
        problem.starts_at = 2_000;
        assert!(!day_started(&problem, 1_999_999));
        assert!(day_started(&problem, 2_000_000));
    }

    /// 決めた応答を順に1つずつ返すHTTPサーバー。受けた要求の1行目を記録する。
    pub(super) fn scripted<S: Into<String> + Send + 'static>(
        responses: Vec<(u16, S)>,
    ) -> (String, std::sync::mpsc::Receiver<String>) {
        scripted_with("", responses)
    }

    /// `scripted` と同じく、すべての応答に `header`(例: `"Name: value\r\n"`)を付ける。
    fn scripted_with<S: Into<String> + Send + 'static>(
        header: &'static str,
        responses: Vec<(u16, S)>,
    ) -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for (status, body) in responses {
                let body: String = body.into();
                let Ok((stream, _)) = listener.accept() else {
                    return;
                };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let _ = tx.send(line.trim_end().to_owned());
                while reader.read_line(&mut line).unwrap_or(0) > 2 {
                    line.clear();
                }
                let mut stream = stream;
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\n{header}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        (base, rx)
    }

    #[test]
    fn tokens_with_reserved_characters_are_sent_intact() {
        let client = Client::new("http://127.0.0.1:1/", "a&b#c=d e").unwrap();
        let request = client
            .request(reqwest::Method::GET, "/setting")
            .build()
            .unwrap();
        assert_eq!(request.url().path(), "/setting");
        let token = request
            .url()
            .query_pairs()
            .find(|(k, _)| k == "token")
            .map(|(_, v)| v.into_owned());
        assert_eq!(token.as_deref(), Some("a&b#c=d e"));
    }

    #[tokio::test]
    async fn the_final_wait_stops_at_the_next_match() {
        const LAST: &str = r#"{"endsAt":100,"day":4,"agents":[],"others":[],"traffics":[]}"#;
        const NEXT: &str = r#"{"endsAt":300,"day":0,"agents":[],"others":[],"traffics":[]}"#;
        let far = now_ms() + 600_000;
        for (responses, requests) in [
            // 最終日の状態が続いた後、次の試合が公開されて開始前(403)を返した。
            (
                vec![
                    (200, LAST),
                    (200, LAST),
                    (403, r#"{"message":"match has not started"}"#),
                ],
                3,
            ),
            // 次の試合がすでに始まっていた。
            (vec![(200, NEXT)], 1),
            (vec![(403, r#"{"message":"match has ended"}"#)], 1),
        ] {
            let (base, rx) = scripted(responses);
            let client = Client::new(&base, "t").unwrap();
            tokio::time::timeout(
                Duration::from_secs(10),
                wait_for_end(&client, Some((4, 100)), None, far),
            )
            .await
            .expect("次の試合の状態で待ち終える");
            assert_eq!(rx.try_iter().count(), requests);
        }
    }

    #[tokio::test]
    async fn gateway_failures_are_retried_like_lost_connections() {
        const GATEWAY: &str =
            r#"{"code":"ProxyError","message":"公式対戦サーバーに接続できません"}"#;
        let far = now_ms() + 600_000;
        // 締切を過ぎていれば送らない。
        let (base, rx) = scripted::<&str>(vec![]);
        let client = Client::new(&base, "t").unwrap();
        let submitted = submit(&client, "/", &serde_json::json!([[0]]), now_ms()).await;
        assert!(submitted.reply.is_none() && !submitted.lost);
        assert_eq!(rx.try_iter().count(), 0);
        let (base, rx) = scripted(vec![
            (502, GATEWAY),
            (504, GATEWAY),
            (200, r#"{"revision":3}"#),
        ]);
        let client = Client::new(&base, "t").unwrap();
        let submitted = submit(&client, "/", &serde_json::json!([[0]]), far).await;
        let (reply, _) = submitted.reply.as_ref().unwrap();
        assert_eq!((reply.status, rx.try_iter().count()), (200, 3));
        // 同じ内容の再送が受理されたので、先の試みの採否によらず受理。
        assert_eq!(submitted.outcome(far / 1000), Some(true));
        // 却下の応答でも、先に応答を受け取れなかった試みがあれば採否は不明。無ければ却下。
        let rejected = r#"{"code":"InvalidPlan","message":"rejected"}"#;
        for (script, outcome) in [
            (vec![(502, GATEWAY), (400, rejected)], None),
            (vec![(400, rejected)], Some(false)),
        ] {
            let (base, _rx) = scripted(script);
            let client = Client::new(&base, "t").unwrap();
            let submitted = submit(&client, "/", &serde_json::json!([[0]]), far).await;
            assert_eq!(submitted.outcome(far / 1000), outcome);
        }
        // 終了確認でも、中継の失敗では待ち終えない。
        let (base, rx) = scripted(vec![
            (502, GATEWAY),
            (403, r#"{"message":"match has ended"}"#),
        ]);
        let client = Client::new(&base, "t").unwrap();
        wait_for_end(&client, Some((4, 100)), None, far).await;
        assert_eq!(rx.try_iter().count(), 2);
    }

    /// どの締切もこの試合のものとみなす日程(開始の見積もりを遠い未来にする)。
    const ANY: (&[u32], u64) = (&[10, 10, 10], u64::MAX / 4);

    #[tokio::test]
    async fn a_different_published_problem_means_the_next_match() {
        let problem = procon37_common::generator::generate(0, Some(MatchFormat::First))
            .unwrap()
            .problem;
        let other = procon37_common::generator::generate(1, Some(MatchFormat::First))
            .unwrap()
            .problem;
        let started = Problem {
            starts_at: 1234,
            ..problem.clone()
        };
        let body = |p: &Problem| serde_json::to_string(p).unwrap();
        for (reply, same) in [
            ((200, body(&started)), true),
            ((200, body(&other)), false),
            ((403, r#"{"message":"match has ended"}"#.to_owned()), true),
        ] {
            let (base, _) = scripted(vec![reply]);
            let client = Client::new(&base, "t").unwrap();
            assert_eq!(same_match(&client, &problem).await, same);
        }
    }

    #[tokio::test]
    async fn another_match_id_ends_the_match_even_for_the_same_problem() {
        const DAY0: &str = r#"{"endsAt":100,"day":0,"agents":[],"others":[],"traffics":[]}"#;
        let far = now_ms() + 600_000;
        // 同じ問題の試合が続いても、試合IDが違えば次の試合。
        let (base, _) = scripted_with("Procon37-Match-Id: next\r\n", vec![(200, DAY0)]);
        let client = Client::new(&base, "t").unwrap();
        assert!(
            fetch_state(&client, 0, None, ANY, Some("ours"), far)
                .await
                .is_none()
        );
        let (base, _) = scripted_with("Procon37-Match-Id: ours\r\n", vec![(200, DAY0)]);
        let client = Client::new(&base, "t").unwrap();
        assert!(
            fetch_state(&client, 0, None, ANY, Some("ours"), far)
                .await
                .is_some()
        );
    }

    #[test]
    fn states_from_a_later_match_are_told_apart_by_their_deadlines() {
        // 各日10秒で、開始は遅くとも1000秒。この試合の2日目の締切は1020秒前後。
        let schedule: (&[u32], u64) = (&[10, 10, 10], 1_000_000);
        let state = |day, ends_at| Observation {
            ends_at,
            day,
            agents: vec![],
            others: vec![],
            traffics: vec![],
        };
        assert!(owns(schedule, &state(1, 1020)));
        // 日と日の間が空いても、全日程(30秒)ぶん空かない限りこの試合。
        assert!(owns(schedule, &state(1, 1045)));
        // 次の試合は、この試合が終わった後に始まる。
        assert!(!owns(schedule, &state(0, 1040)));
        assert!(!owns(schedule, &state(1, 1060)));
    }

    #[tokio::test]
    async fn fetching_the_next_day_gives_up_on_another_match_or_after_the_end() {
        const DAY0: &str = r#"{"endsAt":100,"day":0,"agents":[],"others":[],"traffics":[]}"#;
        const DAY1: &str = r#"{"endsAt":100,"day":1,"agents":[],"others":[],"traffics":[]}"#;
        const DAY2: &str = r#"{"endsAt":110,"day":2,"agents":[],"others":[],"traffics":[]}"#;
        let far = now_ms() + 600_000;
        // 前日のままなら日が進むのを待つ。
        let (base, _) = scripted(vec![(200, DAY1), (200, DAY2)]);
        let client = Client::new(&base, "t").unwrap();
        assert_eq!(
            fetch_state(&client, 2, Some((1, 100)), ANY, None, far)
                .await
                .map(|o| o.day),
            Some(2)
        );
        // 待っている日の前日より前の日は、次の試合の状態。
        let (base, rx) = scripted(vec![(200, DAY0)]);
        let client = Client::new(&base, "t").unwrap();
        assert!(
            fetch_state(&client, 2, Some((1, 100)), ANY, None, far)
                .await
                .is_none()
        );
        assert_eq!(rx.try_iter().count(), 1);
        // 終了予定を過ぎれば、正常な応答でも待ち終える。
        let (base, rx) = scripted(vec![(200, DAY1)]);
        let client = Client::new(&base, "t").unwrap();
        assert!(
            fetch_state(&client, 2, Some((1, 100)), ANY, None, now_ms())
                .await
                .is_none()
        );
        assert_eq!(rx.try_iter().count(), 1);
    }

    #[tokio::test]
    async fn the_previous_day_of_another_match_ends_the_wait() {
        const OTHER: &str = r#"{"endsAt":400,"day":1,"agents":[],"others":[],"traffics":[]}"#;
        let far = now_ms() + 600_000;
        // 最後に回答した2日目(締切100)と、日は同じでも締切が違う状態は次の試合。
        let (base, rx) = scripted(vec![(200, OTHER)]);
        let client = Client::new(&base, "t").unwrap();
        assert!(
            fetch_state(&client, 2, Some((1, 100)), ANY, None, far)
                .await
                .is_none()
        );
        assert_eq!(rx.try_iter().count(), 1);
        // 試合が始まった後の開始前は次の試合。
        let (base, _) = scripted(vec![(403, r#"{"message":"match has not started"}"#)]);
        let client = Client::new(&base, "t").unwrap();
        assert!(
            fetch_state(&client, 2, Some((1, 100)), ANY, None, far)
                .await
                .is_none()
        );
    }

    /// 1台だけ隣のセルへ動いて残りを待つ計画と、そのときの翌日の試合。
    fn moving_plan(game: &Game) -> (Value, Game) {
        let steps = i64::from(game.problem.day_steps[game.day]);
        for direction in 0..6 {
            for cost in 1..=4 {
                let mut plan = game.waiting_plan();
                plan[0] = vec![direction, -(steps - cost)];
                let mut trial = game.clone();
                if trial.play_day(plan.clone()).is_ok() && trial.agents != game.agents {
                    return (serde_json::to_value(plan).unwrap(), trial);
                }
            }
        }
        panic!("動ける計画が見つかりません");
    }

    fn observe(game: &Game) -> Observation {
        Observation {
            ends_at: 100 * (game.day as u64 + 1),
            day: game.day,
            agents: game.agents.clone(),
            others: vec![],
            traffics: game.traffic(),
        }
    }

    #[test]
    fn unknown_submissions_are_reconciled_with_the_next_state() {
        let problem = procon37_common::generator::generate(0, Some(MatchFormat::First))
            .unwrap()
            .problem;
        let game = Game::new(problem.clone(), &vec![0; problem.agents.len()]);
        let before = observe(&game);
        let mut waited = game.clone();
        waited.play_day(game.waiting_plan()).unwrap();
        let unmoved = observe(&waited);
        // 待機と区別できない場合は、実行されなかったとみなす。
        let wait = serde_json::to_value(game.waiting_plan()).unwrap();
        assert_eq!(plan_applied(&game, &before, &wait, &unmoved), Applied::No);
        // 計画どおりに動いた翌日の状態なら実行された。動いていなければ実行されなかった。
        let (plan, moved) = moving_plan(&game);
        let after = observe(&moved);
        assert_eq!(plan_applied(&game, &before, &plan, &after), Applied::On(0));
        assert_eq!(plan_applied(&game, &before, &plan, &unmoved), Applied::No);
        // 通信が途絶えて次に観測したのが3日目でも、その間を全車待機で進めて確かめる。
        let mut later = moved.clone();
        let wait = later.waiting_plan();
        later.play_day(wait).unwrap();
        assert_eq!(
            plan_applied(&game, &before, &plan, &observe(&later)),
            Applied::On(0)
        );
        let broken = serde_json::json!("broken");
        assert_eq!(plan_applied(&game, &before, &broken, &after), Applied::No);
    }

    #[test]
    fn a_plan_delayed_to_a_skipped_day_is_replayed_on_that_day() {
        let problem = procon37_common::generator::generate(0, Some(MatchFormat::First))
            .unwrap()
            .problem;
        let game = Game::new(problem.clone(), &vec![0; problem.agents.len()]);
        let before = observe(&game);
        // 1日目の提出の送信が遅れ、`offset` 日後の締切の後に処理されてその次の日に実行された。その間の日は観測できず、
        // 次に観測したのは実行された日の翌日。
        for offset in 1..=2 {
            let mut waited = game.clone();
            for _ in 0..offset {
                let wait = waited.waiting_plan();
                waited.play_day(wait).unwrap();
            }
            let (plan, moved) = moving_plan(&waited);
            let after = observe(&moved);
            assert_eq!(
                plan_applied(&game, &before, &plan, &after),
                Applied::On(offset)
            );
            // 控えは実行された日だけを計画どおり、ほかの日を全車待機で進め、観測と一致する。
            let mut shadow = game.clone();
            advance(&mut shadow, &before, None);
            let retained = serde_json::from_value::<Plan>(plan.clone())
                .ok()
                .map(|p| (offset, p));
            let requests = catch_up_requests(&mut shadow, &after, false, retained);
            assert_eq!(requests.len(), offset);
            assert_eq!((shadow.day, &shadow.agents), (offset + 1, &moved.agents));
            assert_eq!(shadow.traffic(), moved.traffic());
            // 対戦では、飛ばした日の道路状態を観測できないので、その日に実行されたかは判断しない。
            let other = serde_json::json!({"agents": []});
            let versus = |obs: Observation| Observation {
                others: vec![other.clone()],
                ..obs
            };
            assert_eq!(
                plan_applied(&game, &versus(before.clone()), &plan, &versus(after)),
                Applied::Unknown
            );
            // 全車待機で観測と一致しても、飛ばした日に往復した場合を除けないので判断しない。
            let mut idle = waited.clone();
            let wait = idle.waiting_plan();
            idle.play_day(wait).unwrap();
            assert_eq!(
                plan_applied(
                    &game,
                    &versus(before.clone()),
                    &plan,
                    &versus(observe(&idle))
                ),
                Applied::Unknown
            );
        }
    }

    #[test]
    fn a_round_trip_is_told_apart_from_waiting_by_the_roads() {
        // 1チームの試合で、補給車(燃料が減らない)が道路を行って戻り、同じ位置・燃料で終わる計画を探す。
        let found = (0..40u64).find_map(|seed| {
            let problem = procon37_common::generator::generate(seed, Some(MatchFormat::First))
                .ok()?
                .problem;
            let mut kinds = vec![0; problem.agents.len()];
            kinds[0] = 1;
            let game = Game::new(problem.clone(), &kinds);
            let steps = i64::from(problem.day_steps[0]);
            let mut waited = game.clone();
            waited.play_day(game.waiting_plan()).ok()?;
            (0..6)
                .flat_map(|d| (1..=4).flat_map(move |a| (1..=4).map(move |b| (d, a, b))))
                .find_map(|(d, a, b)| {
                    let mut plan = game.waiting_plan();
                    plan[0] = vec![d, (d + 3) % 6, -(steps - a - b)];
                    let mut trial = game.clone();
                    (trial.play_day(plan.clone()).is_ok()
                        && trial.agents == waited.agents
                        && trial.traffic() != waited.traffic())
                    .then(|| (game.clone(), serde_json::to_value(plan).unwrap(), trial))
                })
        });
        let (game, plan, trip) = found.expect("道路の滞在が変わる往復が見つかりません");
        // 車は待機と同じでも、翌日の道路状態が往復の場合と一致すれば実行されたと分かる。
        assert_eq!(
            plan_applied(&game, &observe(&game), &plan, &observe(&trip)),
            Applied::On(0)
        );
        // 対戦では道路状態を比べないので、全車待機と区別できず、判断できないとして知らせる。
        let versus = |obs: Observation| Observation {
            others: vec![serde_json::json!({"agents": []})],
            ..obs
        };
        assert_eq!(
            plan_applied(
                &game,
                &versus(observe(&game)),
                &plan,
                &versus(observe(&trip))
            ),
            Applied::Unknown
        );
    }

    #[test]
    fn skipped_days_are_replayed_as_waiting() {
        let problem = procon37_common::generator::generate(0, Some(MatchFormat::First))
            .unwrap()
            .problem;
        let kinds = vec![0; problem.agents.len()];
        let mut truth = Game::new(problem.clone(), &kinds);
        for _ in 0..2 {
            let wait = truth.waiting_plan();
            truth.play_day(wait).unwrap();
        }
        let state = Observation {
            ends_at: 0,
            day: 2,
            agents: truth.agents.clone(),
            others: vec![],
            traffics: truth.traffic(),
        };
        let mut shadow = Game::new(problem.clone(), &kinds);
        let requests = catch_up_requests(&mut shadow, &state, true, None);
        assert_eq!(
            requests
                .iter()
                .map(|r| (r.request_id, r.previous_accepted))
                .collect::<Vec<_>>(),
            [(1, Some(true)), (2, Some(false))]
        );
        assert_eq!((shadow.day, &shadow.agents), (2, &truth.agents));
        assert_eq!(shadow.traffic(), truth.traffic());
        assert!(reproduces(&shadow, &state));
        // 途中で再起動した場合など、知らない計画が実行されていれば、全車待機で進めた状態は観測と一致しない。
        let start = Game::new(problem.clone(), &kinds);
        let (_, mut moved) = moving_plan(&start);
        let wait = moved.waiting_plan();
        moved.play_day(wait).unwrap();
        let mut shadow = Game::new(problem.clone(), &kinds);
        catch_up_requests(&mut shadow, &observe(&moved), true, None);
        assert!(!reproduces(&shadow, &observe(&moved)));
    }

    /// 同梱ソルバ(`target/release/solver`)が、途中から参加したときも追いついて回答できる。未ビルドなら確かめない。
    #[tokio::test]
    async fn the_bundled_solver_catches_up_after_joining_late() {
        let solver =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/release/solver");
        if !solver.is_file() {
            eprintln!("Skipping optional solver check: target/release/solver is missing");
            return;
        }
        let problem = procon37_common::generator::generate(0, Some(MatchFormat::First))
            .unwrap()
            .problem;
        let dir = std::env::temp_dir().join(format!("server-bridge-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let spec = CommandSpec {
            program: solver.to_string_lossy().into_owned(),
            args: vec![],
            cwd: dir.clone(),
            env: BTreeMap::new(),
        };
        let mut proc = SolverProcess::spawn(&spec, &dir).await.unwrap();
        let select = Request {
            protocol_version: PROTOCOL_VERSION,
            request_id: 0,
            phase: "select".into(),
            time_limit_ms: 2000,
            problem: Some(problem.clone()),
            state: None,
            steps: None,
            previous_accepted: None,
        };
        assert!(proc.exchange(&select).await.0.is_some());
        // 車種は提出されず全車0のまま、2日目まで全車待機で進んだ後に参加した。
        let kinds = vec![0; problem.agents.len()];
        let mut truth = Game::new(problem.clone(), &kinds);
        for _ in 0..2 {
            let wait = truth.waiting_plan();
            truth.play_day(wait).unwrap();
        }
        let state = Observation {
            ends_at: 0,
            day: 2,
            agents: truth.agents.clone(),
            others: vec![],
            traffics: truth.traffic(),
        };
        let mut shadow = Game::new(problem.clone(), &kinds);
        catch_up(&mut proc, &mut shadow, &state, false, None, false, None).await;
        let request = Request {
            protocol_version: PROTOCOL_VERSION,
            request_id: 3,
            phase: "plan".into(),
            time_limit_ms: 3000,
            problem: None,
            state: Some(state),
            steps: Some(problem.day_steps[2]),
            previous_accepted: Some(false),
        };
        let (answer, failure, _) = proc.exchange(&request).await;
        proc.finish().await;
        let _ = std::fs::remove_dir_all(&dir);
        assert!(answer.is_some(), "{failure:?}");
    }

    #[test]
    fn acceptances_received_after_the_deadline_are_confirmed_later() {
        assert_eq!(outcome_of(true, 99_999, 100), Some(true));
        // 締切(最も早い見積もり)以降に受け取った受理は、翌日の計画だったかもしれない。
        assert_eq!(outcome_of(true, 100_000, 100), None);
        assert_eq!(outcome_of(false, 100_500, 100), Some(false));
    }

    #[test]
    fn arguments_after_the_separator_go_to_the_solver_verbatim() {
        let args = |list: &[&str]| parse(list.iter().map(|s| s.to_string()).collect());
        let base = [
            "--server", "http://s", "--token", "t", "--solver", "./solver",
        ];
        let parsed =
            args(&[&base[..], &["--", "--threads", "4", "--token", "x"]].concat()).unwrap();
        assert_eq!(parsed.solver_args, ["--threads", "4", "--token", "x"]);
        assert_eq!(parsed.token, "t");
        let parsed = args(&[&["--args", "fast", "8"][..], &base[..]].concat()).unwrap();
        assert_eq!(parsed.solver_args, ["fast", "8"]);
        assert!(args(&[&base[..], &["--args", "--threads"]].concat()).is_err());
    }
}
