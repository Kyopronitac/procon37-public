//! 対戦窓口。ソルバPCが接続するHTTPの受け口(既定 :11451)。
//!
//! 公式対戦サーバーの `/setting` `/agent` `/` へ要求を中継しつつ、提出内容と採否を記録する。
//! ソルバPCから見えるのは公式APIだけで、本番の競技サーバーと同じ手順で接続できる。
//! 練習用の追加ヘッダは次のとおり。本番のサーバーはこれらを返さない。
//! - `/setting` に全チーム共通の車種選択開始・締切の UNIX ミリ秒を添える。取得が遅れても提出時刻をそろえる。
//! - 制限時間を短縮したときだけ、`/setting` の応答に車種選択の残り時間(`Procon37-Select-Remaining-Ms`)を添える。
//! - 試合を続けて開くので、進行中の試合への応答に試合ID(`Procon37-Match-Id`)を添える。同じ問題の試合が
//!   続いても、server-bridge が前の試合と次の試合を取り違えないように。

use crate::state::{InFlight, SharedState, Submission, now_ms};
use anyhow::{Context, Result};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{RawQuery, State},
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::net::TcpListener;

/// (試合ID, チーム番号) -> 車種選択の順番待ち。
type AgentOrder = std::sync::Arc<
    std::sync::Mutex<
        std::collections::HashMap<(String, usize), std::sync::Arc<tokio::sync::Mutex<()>>>,
    >,
>;

/// 制限時間を短縮したときに `/setting` へ添える、車種選択の残り時間(ミリ秒)。
const SELECT_HINT_HEADER: &str = "procon37-select-remaining-ms";
/// 公式サーバー起動時に確定した、短縮・正式共通の車種選択時刻。
const SELECT_STARTED_HEADER: &str = "procon37-select-started-at-ms";
const SELECT_DEADLINE_HEADER: &str = "procon37-select-deadline-ms";
/// 進行中の試合への応答に添える試合ID。
const MATCH_ID_HEADER: &str = "procon37-match-id";

#[derive(Clone)]
struct ProxyState {
    shared: SharedState,
    client: reqwest::Client,
    /// チームごとに車種選択の中継を1件ずつ行い、記録の順を公式対戦サーバーの処理順(最後の受理が採用される)と
    /// そろえる。チーム同士は待たせない。
    agent_order: AgentOrder,
}

/// 待受を先に確保し、使用中なら起動時に失敗させる。
pub async fn bind(port: u16) -> Result<TcpListener> {
    TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, port))
        .await
        .with_context(|| {
            format!("対戦窓口のポート {port} を開けません。別の server が起動していないか確認してください")
        })
}

pub async fn serve(listener: TcpListener, shared: SharedState) -> Result<()> {
    let state = ProxyState {
        shared,
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?,
        agent_order: Default::default(),
    };
    let app = Router::new()
        .route("/setting", get(setting))
        .route("/agent", axum::routing::post(agent))
        .route("/", get(match_state).post(plan))
        .with_state(state);
    axum::serve(listener, app).await?;
    Ok(())
}

/// 公式対戦サーバーと同じく、クエリの `token` と `Procon-Token` ヘッダの両方を受け付ける。
/// クエリはパーセントエンコードを解いて読む(`token%2Dp0` も `token-p0` として試合・チームを見分ける)。
fn token(query: &Option<String>, headers: &HeaderMap) -> Option<String> {
    headers
        .get("procon-token")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .or_else(|| {
            form_urlencoded::parse(query.as_deref()?.as_bytes())
                .find(|(key, _)| key == "token")
                .map(|(_, value)| value.into_owned())
        })
}

struct Target {
    /// 要求を受けた時点の試合。記録・観測はこの試合にだけ書き込む(試合が切り替わった後に完了した要求のため)。
    id: String,
    port: u16,
    team: Option<usize>,
    /// 制限時間を短縮しているときの車種選択の締切(UNIXミリ秒)。残り時間は応答を返す直前に計算する。
    select_ends_ms: Option<u64>,
    select_started_ms: u64,
    select_deadline_ms: u64,
    day_seconds: Vec<u32>,
}
fn target(state: &ProxyState, token: &Option<String>) -> Option<Target> {
    let guard = state.shared.lock().unwrap();
    Some(target_of(guard.active.as_ref()?, token))
}
fn target_of(active: &crate::state::ActiveMatch, token: &Option<String>) -> Target {
    Target {
        id: active.id.clone(),
        port: active.internal_port,
        team: token.as_deref().and_then(|t| active.team(t)),
        select_ends_ms: active.shortened.then_some(active.select_ends_ms),
        select_started_ms: active.select_started_ms,
        select_deadline_ms: active.select_ends_ms,
        day_seconds: active.day_seconds.clone(),
    }
}
/// 提出を受けた試合を特定し、同じロックの中でその試合の処理中の提出として数える(記録が済むまで、実行処理が
/// 試合の記録を取り出さない)。特定と数えるのを分けると、その間に試合が取り下げられて記録し損ねうるため。
fn target_in_flight(state: &ProxyState, token: &Option<String>) -> Option<(Target, InFlight)> {
    let mut guard = state.shared.lock().unwrap();
    let active = guard.active.as_mut()?;
    let target = target_of(active, token);
    Some((target, InFlight::enter(&state.shared, active)))
}

/// 応答に、要求を受けた試合のIDを添える。
fn tag(mut response: Response, target: &Target) -> Response {
    if let Ok(id) = header::HeaderValue::from_str(&target.id) {
        response.headers_mut().insert(MATCH_ID_HEADER, id);
    }
    response
}

/// 試合が登録されていない間は、終了済みの試合と同じ応答を返す(接続中のソルバが待ち続けないように)。
fn no_match() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({"code":"AccessTimeError","message":"match has ended"})),
    )
        .into_response()
}

struct Upstream {
    status: StatusCode,
    content_type: Option<header::HeaderValue>,
    body: Bytes,
    /// 公式対戦サーバーの応答を受け取れなかった(接続失敗・打ち切り)。要求が処理されたかは分からない。
    lost: bool,
}
impl IntoResponse for Upstream {
    fn into_response(self) -> Response {
        let mut response = (self.status, self.body).into_response();
        if let Some(value) = self.content_type {
            response.headers_mut().insert(header::CONTENT_TYPE, value);
        }
        response
    }
}

async fn forward(
    state: &ProxyState,
    port: u16,
    method: Method,
    path: &str,
    query: &Option<String>,
    token: &Option<String>,
    body: Option<Bytes>,
) -> Upstream {
    let mut url = format!("http://127.0.0.1:{port}{path}");
    if let Some(q) = query.as_deref().filter(|q| !q.is_empty()) {
        url.push('?');
        url.push_str(q);
    }
    let mut request = state.client.request(method, &url);
    if let Some(t) = token {
        request = request.header("Procon-Token", t);
    }
    if let Some(b) = body {
        request = request
            .header(header::CONTENT_TYPE, "application/json")
            .body(b);
    }
    // 本文の読み取りに失敗した場合も、応答を受け取れなかったものとして扱う(ヘッダだけで採否は分からない)。
    let response = match request.send().await {
        Ok(response) => {
            let status =
                StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let content_type = response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| header::HeaderValue::from_bytes(v.as_bytes()).ok());
            response
                .bytes()
                .await
                .map(|body| (status, content_type, body))
        }
        Err(e) => Err(e),
    };
    match response {
        Ok((status, content_type, body)) => Upstream {
            status,
            content_type,
            body,
            lost: false,
        },
        Err(e) => Upstream {
            lost: true,
            status: StatusCode::BAD_GATEWAY,
            content_type: Some(header::HeaderValue::from_static("application/json")),
            body: Bytes::from(
                json!({"code":"ProxyError","message":format!("公式対戦サーバーの応答を受け取れません: {e}")})
                    .to_string(),
            ),
        },
    }
}

/// 試合・チームごとの車種選択の順番待ち。前の試合の要求が残っていても、次の試合を待たせない。
/// 使われていない前の試合の順番待ちは片付ける。
fn agent_lock(
    state: &ProxyState,
    match_id: &str,
    team: usize,
) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    let mut order = state.agent_order.lock().unwrap();
    order.retain(|(id, _), lock| id == match_id || std::sync::Arc::strong_count(lock) > 1);
    order
        .entry((match_id.to_owned(), team))
        .or_default()
        .clone()
}

/// 要求を受けた時点の試合(`target`)がまだ進行中なら、その試合の状態を返す。
fn active_match<'a>(
    guard: &'a mut std::sync::MutexGuard<'_, crate::state::Shared>,
    target: &Target,
) -> Option<&'a mut crate::state::ActiveMatch> {
    guard.active.as_mut().filter(|a| a.id == target.id)
}

/// 状態の日と締切を、その試合の直近の観測として覚える。
fn observe(state: &ProxyState, target: &Target, upstream: &Upstream) {
    let Some(observed) = day_of(upstream).zip(deadline_of(upstream)) else {
        return;
    };
    if let Some(active) = active_match(&mut state.shared.lock().unwrap(), target) {
        active.observed = Some(observed);
    }
}
fn observed(state: &ProxyState, target: &Target) -> Option<(usize, u64)> {
    active_match(&mut state.shared.lock().unwrap(), target)?.observed
}

fn record(state: &ProxyState, target: &Target, f: impl FnOnce(&mut crate::state::TeamRecord)) {
    let Some(team) = target.team else { return };
    let mut guard = state.shared.lock().unwrap();
    if let Some(record) = active_match(&mut guard, target).and_then(|a| a.records.get_mut(team)) {
        f(record);
    }
}

fn body_value(body: &Bytes) -> Value {
    serde_json::from_slice(body)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(body).into_owned()))
}

/// 完了済み日数(`day`)。試合開始前・終了後は None。
fn day_of(upstream: &Upstream) -> Option<usize> {
    state_field(upstream, "day").map(|d| d as usize)
}

/// その日の締切(`endsAt`、UNIX秒で秒未満は切り捨て)。
fn deadline_of(upstream: &Upstream) -> Option<u64> {
    state_field(upstream, "endsAt")
}

fn state_field(upstream: &Upstream, key: &str) -> Option<u64> {
    if upstream.status != StatusCode::OK {
        return None;
    }
    serde_json::from_slice::<Value>(&upstream.body)
        .ok()?
        .get(key)?
        .as_u64()
}

async fn setting(
    State(state): State<ProxyState>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    let token = token(&query, &headers);
    let Some(target) = target(&state, &token) else {
        return no_match();
    };
    let upstream = forward(
        &state,
        target.port,
        Method::GET,
        "/setting",
        &query,
        &token,
        None,
    )
    .await;
    if upstream.status != StatusCode::OK {
        return tag(upstream.into_response(), &target);
    }
    let at_ms = now_ms();
    record(&state, &target, |r| {
        r.setting_at_ms.get_or_insert(at_ms);
    });
    selection_headers(tag(upstream.into_response(), &target), &target)
}

/// The explicit origin is stable for all teams and late /setting reads. It uses
/// the same launch epoch as the existing selection deadline, without changing
/// either the official server's time budget or the short-only remaining hint.
fn selection_headers(mut response: Response, target: &Target) -> Response {
    response.headers_mut().insert(SELECT_STARTED_HEADER,
        header::HeaderValue::from(target.select_started_ms));
    response.headers_mut().insert(SELECT_DEADLINE_HEADER,
        header::HeaderValue::from(target.select_deadline_ms));
    if let Some(ends_ms) = target.select_ends_ms {
        let remaining = ends_ms.saturating_sub(now_ms());
        response.headers_mut().insert(SELECT_HINT_HEADER, header::HeaderValue::from(remaining));
    }
    response
}

async fn agent(
    State(state): State<ProxyState>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let token = token(&query, &headers);
    let Some((target, _flight)) = target_in_flight(&state, &token) else {
        return no_match();
    };
    // 提出時刻は、同じチームの先の提出を待つ前に記録する(回答時間に順番待ちを含めない)。
    let arrived_ms = now_ms();
    let _order = match target.team {
        Some(team) => Some(agent_lock(&state, &target.id, team).lock_owned().await),
        None => None,
    };
    let upstream = forward(
        &state,
        target.port,
        Method::POST,
        "/agent",
        &query,
        &token,
        Some(body.clone()),
    )
    .await;
    let submission = Submission {
        body: body_value(&body),
        at_ms: arrived_ms,
        status: upstream.status.as_u16(),
        revision: None,
        accepted: upstream.status == StatusCode::OK,
        uncertain_day: false,
        unknown: upstream.lost,
    };
    record(&state, &target, |r| r.kinds.push(submission));
    tag(upstream.into_response(), &target)
}

async fn match_state(
    State(state): State<ProxyState>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    let token = token(&query, &headers);
    let Some(target) = target(&state, &token) else {
        return no_match();
    };
    let upstream = forward(&state, target.port, Method::GET, "/", &query, &token, None).await;
    observe(&state, &target, &upstream);
    if let Some(day) = day_of(&upstream) {
        let at_ms = now_ms();
        record(&state, &target, |r| {
            r.day_seen_ms.entry(day).or_insert(at_ms);
        });
    }
    remember_state(&state, &target, &upstream);
    tag(upstream.into_response(), &target)
}

/// 状態で観測した自チームの車と道路状態を、その日の最初の観測として覚える。
fn remember_state(state: &ProxyState, target: &Target, upstream: &Upstream) {
    let Some(day) = day_of(upstream) else { return };
    let Ok(value) = serde_json::from_slice::<Value>(&upstream.body) else {
        return;
    };
    let field = |key: &str| value.get(key).cloned();
    let agents = field("agents")
        .and_then(|v| serde_json::from_value::<Vec<procon37_common::model::Agent>>(v).ok());
    let traffics = field("traffics")
        .and_then(|v| serde_json::from_value::<Vec<procon37_common::model::Traffic>>(v).ok());
    record(state, target, |r| {
        if let Some(agents) = agents {
            r.day_agents.entry(day).or_insert(agents);
        }
        if let Some(traffics) = traffics {
            r.day_traffics.entry(day).or_insert(traffics);
        }
    });
}

async fn plan(
    State(state): State<ProxyState>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let token = token(&query, &headers);
    let Some((target, _flight)) = target_in_flight(&state, &token) else {
        return no_match();
    };
    // 提出はすぐ中継する(締切直前の提出を遅らせない)。受け取った時刻を回答時間の終点にする。
    // 直近の観測は中継より前に処理された状態なので、提出を処理した日はその日以降になる。
    let before = observed(&state, &target);
    let sent_ms = now_ms();
    let upstream = forward(
        &state,
        target.port,
        Method::POST,
        "/",
        &query,
        &token,
        Some(body.clone()),
    )
    .await;
    let received_ms = now_ms();
    // 公式対戦サーバーは提出を処理した時点の日に計画を適用する。ソルバが取得した状態の日とは
    // 日の境界をまたぐとずれるため、提出の直後に日を確認し、観測した日程と合わせて記録先を決める(plan_day)。
    let after = forward(&state, target.port, Method::GET, "/", &query, &token, None).await;
    observe(&state, &target, &after);
    remember_state(&state, &target, &after);
    let day = plan_day(
        before,
        &target.day_seconds,
        day_of(&after),
        sent_ms,
        received_ms,
    );
    // 公式仕様: revision が0以上なら受理、0未満なら却下。値が無い応答(403など)は却下扱い。
    let revision = serde_json::from_slice::<Value>(&upstream.body)
        .ok()
        .and_then(|v| v.get("revision").and_then(Value::as_i64));
    let submission = Submission {
        body: body_value(&body),
        at_ms: sent_ms,
        status: upstream.status.as_u16(),
        revision,
        accepted: upstream.status == StatusCode::OK && revision.is_some_and(|r| r >= 0),
        uncertain_day: day.is_some_and(|(_, uncertain)| uncertain),
        unknown: upstream.lost,
    };
    if let Some((day, _)) = day {
        record(&state, &target, |r| {
            r.plans.entry(day).or_default().push(submission)
        });
    }
    tag(upstream.into_response(), &target)
}

/// 計画を記録する日と、その日が推定か(true)を決める。公式対戦サーバーは提出を処理した時点の日に計画を
/// 適用するが、応答に日は含まれない。そこで、提出を中継する前に観測していた状態(`observed`: ある日とその締切
/// endsAt)と各日の秒数から各日の締切を求め、提出を送った・応答を受けた時刻と、提出の直後に確認した日(`after`)で
/// 処理した日を絞り込む。endsAt は秒未満を切り捨てた値で日の長さは整数秒なので、日 k の実際の締切は E_k 秒から
/// 1秒以内にある。
/// - 下限: 観測していた日と、送った時点で確実に締切を過ぎていた日の数の大きい方。
/// - 上限: 応答を受けた時点で確実に締切前だった最初の日と、直後に確認した日の小さい方(最終日で打ち止め)。
/// - 下限と上限が一致すればその日。一致しない(締切の1秒の幅で処理された)なら、ソルバが回答していた下限の日に
///   記録して推定とする。観測が無ければ直後に確認した日を使う。
fn plan_day(
    observed: Option<(usize, u64)>,
    day_seconds: &[u32],
    after: Option<usize>,
    sent_ms: u64,
    received_ms: u64,
) -> Option<(usize, bool)> {
    let days = day_seconds.len();
    let after = after.filter(|&day| day < days);
    let Some((seen, ends_at)) = observed.filter(|&(day, _)| day < days) else {
        return after.map(|day| (day, false));
    };
    let seconds = |from: usize, to: usize| -> i64 {
        day_seconds[from..to].iter().map(|&s| i64::from(s)).sum()
    };
    let deadline_ms = |k: usize| -> i64 {
        let secs = if k >= seen {
            ends_at as i64 + seconds(seen + 1, k + 1)
        } else {
            ends_at as i64 - seconds(k + 1, seen + 1)
        };
        secs * 1000
    };
    let (sent, received) = (sent_ms as i64, received_ms as i64);
    let lower = (0..days)
        .take_while(|&k| sent >= deadline_ms(k) + 1000)
        .count()
        .max(seen);
    let upper = (0..days)
        .find(|&k| received < deadline_ms(k))
        .unwrap_or(days - 1);
    let upper = after.map_or(upper, |day| upper.min(day)).min(days - 1);
    let lower = lower.min(upper);
    Some((lower, lower != upper))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_truncated_response_is_treated_as_lost() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        // 200 のヘッダを返した後、本文の途中で切断する公式対戦サーバー。
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.read(&mut [0; 4096]);
            let _ =
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{\"revision\"");
        });
        let state = ProxyState {
            shared: SharedState::default(),
            client: reqwest::Client::new(),
            agent_order: Default::default(),
        };
        let upstream = forward(
            &state,
            port,
            Method::POST,
            "/",
            &None,
            &None,
            Some(Bytes::from("[]")),
        )
        .await;
        assert!(upstream.lost);
        assert_eq!(upstream.status, StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn selection_epoch_is_stable_for_full_and_short_without_changing_deadlines() {
        use crate::state::{ActiveMatch, TeamRecord};
        // Historical fixed timestamps deliberately precede this read: deriving
        // the origin from response time would move every expected checkpoint.
        let start = 1_234_000;
        for (duration, shortened) in [(60_000,false),(90_000,false),(120_000,false),(10_000,true)] {
            let active = ActiveMatch { id:"epoch-test".into(),tokens:vec!["t".into()],
                records:vec![TeamRecord::default()],internal_port:1,
                select_started_ms:start,select_ends_ms:start+duration,shortened,
                day_seconds:vec![60],observed:None,in_flight:0 };
            for token in [Some("t".into()),None] {
                let target = target_of(&active,&token);
                let response = selection_headers(StatusCode::OK.into_response(),&target);
                let value = |name:&str| response.headers()[name].to_str().unwrap().parse::<u64>().unwrap();
                assert_eq!(value(SELECT_STARTED_HEADER),start);
                assert_eq!(value(SELECT_DEADLINE_HEADER),start+duration);
                assert_eq!(response.headers().contains_key(SELECT_HINT_HEADER),shortened);
                if shortened { assert_eq!(value(SELECT_HINT_HEADER),0); }
                assert_eq!(active.select_ends_ms,start+duration);
            }
        }
    }

    #[test]
    fn late_requests_do_not_write_into_the_next_match() {
        use crate::state::{ActiveMatch, TeamRecord};
        let active = |id: &str| ActiveMatch {
            id: id.into(),
            tokens: vec!["token-p0".into()],
            records: vec![TeamRecord::default()],
            internal_port: 1,
            select_started_ms: 0,
            select_ends_ms: 0,
            shortened: false,
            day_seconds: vec![10],
            observed: None,
            in_flight: 0,
        };
        let shared = SharedState::default();
        shared.lock().unwrap().active = Some(active("old"));
        let state = ProxyState {
            shared: shared.clone(),
            client: reqwest::Client::new(),
            agent_order: Default::default(),
        };
        let old = target(&state, &Some("token-p0".into())).unwrap();
        // 古い試合への要求が完了する前に、同じポートで次の試合が始まった場合。
        shared.lock().unwrap().active = Some(active("new"));
        record(&state, &old, |r| {
            r.day_seen_ms.insert(0, 123);
        });
        let state_body = Upstream {
            status: StatusCode::OK,
            content_type: None,
            body: Bytes::from(r#"{"day":0,"endsAt":100}"#),
            lost: false,
        };
        observe(&state, &old, &state_body);
        let guard = shared.lock().unwrap();
        let next = guard.active.as_ref().unwrap();
        assert!(next.records[0].day_seen_ms.is_empty());
        assert_eq!(next.observed, None);
        drop(guard);
        // 車種選択の順番待ちはチームごと。
        let same = std::sync::Arc::ptr_eq(&agent_lock(&state, "m", 0), &agent_lock(&state, "m", 0));
        let other =
            std::sync::Arc::ptr_eq(&agent_lock(&state, "m", 0), &agent_lock(&state, "m", 1));
        // 前の試合の要求が順番待ちを持ったままでも、次の試合の同じチームは別に待つ。
        let stale = agent_lock(&state, "old", 0);
        assert!(!std::sync::Arc::ptr_eq(&stale, &agent_lock(&state, "m", 0)));
        assert!(same && !other);
    }

    #[test]
    fn plans_are_recorded_on_the_day_they_were_processed() {
        // 各日10秒。2日目(day=1)の締切を100秒と観測したので、締切は 90・100・110 秒(実際は各1秒以内に遅れる)。
        let seconds = [10, 10, 10];
        let day =
            |observed, after, sent, received| plan_day(observed, &seconds, after, sent, received);
        let seen = Some((1, 100));
        assert_eq!(day(seen, Some(1), 95_000, 95_010), Some((1, false)));
        // 応答が締切より前に返った提出は、直後の確認で日が進んでいても2日目。
        assert_eq!(day(seen, Some(2), 99_900, 99_990), Some((1, false)));
        // 締切の1秒の幅でも、直後の確認がまだ2日目なら2日目。
        assert_eq!(day(seen, Some(1), 100_400, 100_420), Some((1, false)));
        // 締切を確実に過ぎてから送った提出は3日目。古い日の観測からでも締切を求められる。
        assert_eq!(day(seen, Some(2), 101_000, 101_010), Some((2, false)));
        assert_eq!(
            day(Some((0, 90)), Some(2), 105_000, 105_010),
            Some((2, false))
        );
        // 締切の1秒の幅で処理され、直後の確認で日が進んでいたら判断できないので、2日目に記録して推定とする。
        assert_eq!(day(seen, Some(2), 100_400, 100_420), Some((1, true)));
        // 1日が1秒しかない(制限時間を短縮した)場合も、観測していた日より前には記録しない。
        let short =
            |observed, after, sent, received| plan_day(observed, &[1, 1, 1], after, sent, received);
        assert_eq!(
            short(Some((1, 1002)), Some(1), 1_001_500, 1_001_510),
            Some((1, false))
        );
        assert_eq!(
            short(Some((0, 1001)), Some(0), 1_001_100, 1_001_110),
            Some((0, false))
        );
        // 最終日の締切をまたいで試合が終わった場合は、最終日のまま。
        assert_eq!(
            day(Some((2, 110)), None, 110_400, 110_420),
            Some((2, false))
        );
        assert_eq!(
            day(Some((2, 110)), Some(3), 110_400, 110_420),
            Some((2, false))
        );
        // 観測が無ければ直後に確認した日。
        assert_eq!(day(None, Some(0), 0, 0), Some((0, false)));
        assert_eq!(day(None, None, 0, 0), None);
    }

    #[test]
    fn token_comes_from_the_header_or_the_query() {
        let mut headers = HeaderMap::new();
        let query = Some("a=1&token=token-p2".to_owned());
        assert_eq!(token(&query, &headers).as_deref(), Some("token-p2"));
        assert_eq!(token(&Some("tokens=x".into()), &headers), None);
        // パーセントエンコードされたトークンも解いて読む。
        let encoded = Some("token=token%2Dp0".to_owned());
        assert_eq!(token(&encoded, &headers).as_deref(), Some("token-p0"));
        headers.insert("procon-token", "token-p1".parse().unwrap());
        assert_eq!(token(&query, &headers).as_deref(), Some("token-p1"));
    }

    #[test]
    fn only_successful_states_have_a_day() {
        let upstream = |status, body: &str| Upstream {
            status,
            content_type: None,
            body: Bytes::from(body.to_owned()),
            lost: false,
        };
        assert_eq!(day_of(&upstream(StatusCode::OK, r#"{"day":2}"#)), Some(2));
        assert_eq!(
            day_of(&upstream(StatusCode::FORBIDDEN, r#"{"day":2}"#)),
            None
        );
        assert_eq!(day_of(&upstream(StatusCode::OK, "{}")), None);
    }
}
