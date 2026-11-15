//! 結果閲覧・実行用のローカルHTTP API。`common/src/server.rs` と同じ構成で、
//! 画面は共通の盤面・再生・GIF・画面遷移を使い、対戦サーバー固有部だけ `web/` に置く。

use crate::runner::{Manifest, Progress, ServerConfig, generate_case, run_batch};
use crate::state::SharedState;
use anyhow::Result;
use axum::{
    Json, Router,
    extract::{Path, Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use procon37_common::{comparison, model::MatchFormat, runner::run_id};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path as FsPath, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt},
    sync::watch,
};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunView {
    manifest: Manifest,
}
struct Active {
    view: RunView,
    cancel: watch::Sender<bool>,
    finished: watch::Receiver<bool>,
}
/// 対戦窓口の状態。開けなかった場合も履歴の閲覧はできる。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    pub port: u16,
    pub address: Option<String>,
    pub error: Option<String>,
}
#[derive(Clone)]
struct AppState {
    root: PathBuf,
    workspace: PathBuf,
    shutdown: watch::Sender<bool>,
    config: ServerConfig,
    shared: SharedState,
    window: Window,
    active: Arc<Mutex<Option<Active>>>,
}
struct ApiError(StatusCode, String);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({"error":self.1}))).into_response()
    }
}
fn bad(e: impl std::fmt::Display) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, e.to_string())
}
fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() < 80 && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}
fn asset(
    content_type: &'static str,
    source: &'static str,
) -> axum::routing::MethodRouter<AppState> {
    get(move || async move { ([("content-type", content_type)], source) })
}
fn javascript(source: &'static str) -> axum::routing::MethodRouter<AppState> {
    asset("text/javascript; charset=utf-8", source)
}

pub async fn serve(
    root: PathBuf,
    port: u16,
    config: ServerConfig,
    shared: SharedState,
    window: Window,
) -> Result<()> {
    tokio::fs::create_dir_all(&root).await?;
    let (shutdown, mut shutdown_rx) = watch::channel(false);
    let state = AppState {
        root: root.canonicalize()?,
        workspace: std::env::current_dir()?.canonicalize()?,
        shutdown,
        config,
        shared,
        window,
        active: Arc::new(Mutex::new(None)),
    };
    let app = Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("../web/index.html")) }),
        )
        .route("/app.js", javascript(include_str!("../web/app.js")))
        .route(
            "/match-ui.js",
            javascript(include_str!("../web/match-ui.js")),
        )
        .route(
            "/server.css",
            asset("text/css; charset=utf-8", include_str!("../web/server.css")),
        )
        // 盤面の描画と共通部品は単独・対戦テスタと同じファイルを配信する。
        .route(
            "/style.css",
            asset(
                "text/css; charset=utf-8",
                include_str!("../../../common/web/style.css"),
            ),
        )
        .route(
            "/board.js",
            javascript(include_str!("../../../common/web/board.js")),
        )
        .route(
            "/viewer.js",
            javascript(include_str!("../../../common/web/viewer.js")),
        )
        .route(
            "/navigation.js",
            javascript(include_str!("../../../common/web/navigation.js")),
        )
        .route(
            "/record-stats.js",
            javascript(include_str!("../../../common/web/record-stats.js")),
        )
        .route(
            "/playback.js",
            javascript(include_str!("../../../common/web/playback.js")),
        )
        .route(
            "/gif-export.js",
            javascript(include_str!("../../../common/web/gif-export.js")),
        )
        .route(
            "/gif-worker.js",
            javascript(include_str!("../../../common/web/gif-worker.js")),
        )
        .route(
            "/gif-codec.js",
            javascript(include_str!("../../../common/web/gif-codec.js")),
        )
        .route(
            "/vendor/gifenc.js",
            javascript(include_str!("../../../common/web/vendor/gifenc.js")),
        )
        .route(
            "/vendor/gifenc.LICENSE.md",
            get(|| async { include_str!("../../../common/web/vendor/gifenc.LICENSE.md") }),
        )
        .route("/api/config", get(default_config))
        .route("/api/launcher", get(launcher_identity))
        .route("/api/shutdown", post(shutdown_idle))
        .route("/api/window", get(window_status))
        .route("/api/compare", post(compare_runs))
        .route("/api/active", get(active_run))
        .route("/api/generate", get(preview))
        .route("/api/runs", get(list_runs).post(start_run))
        .route("/api/runs/{id}", get(run_view).delete(delete_run))
        .route("/api/runs/{id}/cancel", post(cancel_run))
        .route("/api/runs/{id}/{seed}/replay", get(replay))
        .route("/api/runs/{id}/{seed}/team-log", get(team_log))
        .route("/api/runs/{id}/{seed}/server-log", get(server_log))
        .layer(middleware::from_fn(local_only))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    eprintln!(
        "ビジュアライザ: http://127.0.0.1:{}/",
        listener.local_addr()?.port()
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = shutdown_rx.changed() => {},
            }
            let finished = state.active.lock().unwrap().as_ref().map(|active| {
                let _ = active.cancel.send(true);
                active.finished.clone()
            });
            if let Some(mut finished) = finished {
                let _ = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    if !*finished.borrow() {
                        let _ = finished.changed().await;
                    }
                })
                .await;
            }
        })
        .await?;
    Ok(())
}

async fn local_only(request: Request, next: Next) -> Response {
    let headers = request.headers();
    let host = headers
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let hostname = host.split(':').next().unwrap_or("");
    if hostname != "localhost" && hostname != "127.0.0.1" {
        return (StatusCode::FORBIDDEN, "localhost only").into_response();
    }
    if let Some(origin) = headers.get("origin")
        && origin.to_str().ok() != Some(format!("http://{host}").as_str())
    {
        return (StatusCode::FORBIDDEN, "same-origin requests only").into_response();
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
}

async fn default_config(State(state): State<AppState>) -> Json<ServerConfig> {
    Json(state.config)
}
// A line-based identity lets the Bash launcher verify both paths without jq.
async fn launcher_identity(State(state): State<AppState>) -> String {
    format!(
        "procon37-server-launcher-v1\n{}\n{}\n",
        state.workspace.display(),
        state.root.display()
    )
}
async fn window_status(State(state): State<AppState>) -> Json<Window> {
    Json(state.window)
}
async fn shutdown_idle(State(state): State<AppState>) -> Result<Json<ValueMessage>, ApiError> {
    let active = state.active.lock().unwrap();
    if active.as_ref().is_some_and(|a| !*a.finished.borrow()) {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "対戦の実行中は停止できません。GUIで完了または中断を待ってください。".into(),
        ));
    }
    state.shutdown.send_replace(true);
    Ok(Json(ValueMessage {
        message: "サーバーを停止します",
    }))
}
async fn active_run(State(state): State<AppState>) -> Json<Option<RunView>> {
    Json(
        state
            .active
            .lock()
            .unwrap()
            .as_ref()
            .map(|active| active.view.clone()),
    )
}
#[derive(Deserialize)]
struct Preview {
    #[serde(default)]
    seed: String,
    format: Option<String>,
    #[serde(rename = "arenaMaps")]
    arena_maps: Option<bool>,
}
async fn preview(
    State(state): State<AppState>,
    Query(query): Query<Preview>,
) -> Result<Json<procon37_common::model::Case>, ApiError> {
    let seed = if query.seed.is_empty() {
        0
    } else {
        query.seed.parse::<u64>().map_err(bad)?
    };
    let format = match query.format.as_deref() {
        None | Some("") | Some("auto") => None,
        Some(s) => Some(MatchFormat::parse(s).map_err(bad)?),
    };
    let config = ServerConfig {
        format,
        arena_maps: query.arena_maps.unwrap_or(state.config.arena_maps),
        ..state.config.clone()
    };
    Ok(Json(generate_case(seed, &config).map_err(internal)?))
}

async fn start_run(
    State(state): State<AppState>,
    Json(mut config): Json<ServerConfig>,
) -> Result<Json<RunView>, ApiError> {
    config.validate().map_err(bad)?;
    if let Some(error) = &state.window.error {
        return Err(ApiError(StatusCode::CONFLICT, error.clone()));
    }
    let id = run_id();
    let (tx, rx) = watch::channel(false);
    let (finished_tx, finished_rx) = watch::channel(false);
    let view = RunView {
        manifest: Manifest {
            schema_version: crate::runner::SCHEMA_VERSION,
            id: id.clone(),
            status: "running".into(),
            verdict: None,
            total: config.total(),
            completed: 0,
            cases: vec![],
            error: None,
            config: config.clone(),
        },
    };
    {
        let mut active = state.active.lock().unwrap();
        if *state.shutdown.borrow() {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "サーバーを停止しています".into(),
            ));
        }
        if active.as_ref().is_some_and(|a| !*a.finished.borrow()) {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "実行中の対戦が完了するか、中断してから開始してください".into(),
            ));
        }
        *active = Some(Active {
            view: view.clone(),
            cancel: tx,
            finished: finished_rx,
        });
    }
    let shared = state.active.clone();
    let event_id = id.clone();
    let progress: Progress = Arc::new(move |manifest: &Manifest| {
        let mut guard = shared.lock().unwrap();
        if let Some(active) = guard.as_mut().filter(|a| a.view.manifest.id == event_id) {
            active.view.manifest = manifest.clone();
        }
    });
    tokio::spawn(async move {
        if let Err(e) = run_batch(id.clone(), config, state.root, state.shared, progress, rx).await
            && let Some(active) = state
                .active
                .lock()
                .unwrap()
                .as_mut()
                .filter(|a| a.view.manifest.id == id)
        {
            active.view.manifest.status = "failed".into();
            active.view.manifest.error = Some(format!("{e:#}"));
        }
        let _ = finished_tx.send(true);
    });
    Ok(Json(view))
}

async fn read_manifest(dir: &FsPath) -> Result<Manifest> {
    Ok(serde_json::from_slice(
        &tokio::fs::read(dir.join("run.json")).await?,
    )?)
}

async fn list_runs(State(state): State<AppState>) -> Result<Json<Vec<Manifest>>, ApiError> {
    let mut dirs = tokio::fs::read_dir(&state.root).await.map_err(internal)?;
    let mut paths = vec![];
    while let Some(entry) = dirs.next_entry().await.map_err(internal)? {
        if entry.file_type().await.map_err(internal)?.is_dir()
            && entry.file_name().to_str().is_some_and(valid_id)
        {
            paths.push(entry.path());
        }
    }
    paths.sort();
    paths.reverse();
    let mut results = Vec::new();
    for p in paths.into_iter().take(100) {
        if let Ok(m) = read_manifest(&p).await {
            results.push(m);
        }
    }
    if let Some(active) = state.active.lock().unwrap().as_ref() {
        results.retain(|m| m.id != active.view.manifest.id);
        results.insert(0, active.view.manifest.clone());
    }
    Ok(Json(results))
}

async fn saved_manifest(state: &AppState, id: &str) -> Result<Manifest, ApiError> {
    if !valid_id(id) {
        return Err(bad("実行IDが不正です"));
    }
    read_manifest(&state.root.join(id)).await.map_err(|e| {
        ApiError(
            StatusCode::NOT_FOUND,
            format!("実行結果を読み込めません: {e}"),
        )
    })
}

async fn run_view(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RunView>, ApiError> {
    if !valid_id(&id) {
        return Err(bad("実行IDが不正です"));
    }
    if let Some(active) = state
        .active
        .lock()
        .unwrap()
        .as_ref()
        .filter(|a| a.view.manifest.id == id)
    {
        return Ok(Json(active.view.clone()));
    }
    Ok(Json(RunView {
        manifest: saved_manifest(&state, &id).await?,
    }))
}

async fn cancel_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ValueMessage>, ApiError> {
    let guard = state.active.lock().unwrap();
    let active = guard
        .as_ref()
        .filter(|a| a.view.manifest.id == id)
        .ok_or_else(|| bad("このサーバーで実行中の対戦ではありません"))?;
    active.cancel.send(true).map_err(bad)?;
    Ok(Json(ValueMessage {
        message: "中断を要求しました",
    }))
}

async fn delete_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ValueMessage>, ApiError> {
    if !valid_id(&id) {
        return Err(bad("実行IDが不正です"));
    }
    // A final status may be published before the last files have been written.
    if state
        .active
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|a| a.view.manifest.id == id && !*a.finished.borrow())
    {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "実行中の履歴は削除できません。完了または中断を待ってください。".into(),
        ));
    }
    let dir = state.root.join(&id);
    let metadata = tokio::fs::symlink_metadata(&dir).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ApiError(
                StatusCode::NOT_FOUND,
                "この実行履歴は既に削除されています".into(),
            )
        } else {
            internal(e)
        }
    })?;
    if !metadata.is_dir() || metadata.is_symlink() {
        return Err(bad("実行履歴のディレクトリではありません"));
    }
    let manifest = read_manifest(&dir).await.map_err(bad)?;
    if manifest.id != id {
        return Err(bad("保存された実行IDが一致しません"));
    }
    if !matches!(
        manifest.status.as_str(),
        "completed" | "failed" | "cancelled"
    ) {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "実行中の履歴は削除できません。完了または中断を待ってください。".into(),
        ));
    }
    tokio::fs::remove_dir_all(&dir).await.map_err(internal)?;
    let mut active = state.active.lock().unwrap();
    if active.as_ref().is_some_and(|a| a.view.manifest.id == id) {
        *active = None;
    }
    Ok(Json(ValueMessage {
        message: "実行履歴を削除しました",
    }))
}
#[derive(Serialize)]
struct ValueMessage {
    message: &'static str,
}

fn case_path(root: &FsPath, id: &str, seed: &str) -> Result<PathBuf, ApiError> {
    if !valid_id(id) {
        return Err(bad("実行IDが不正です"));
    }
    let seed = seed.parse::<u64>().map_err(bad)?;
    Ok(root.join(id).join(seed.to_string()))
}
async fn replay(
    State(state): State<AppState>,
    Path((id, seed)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let path = case_path(&state.root, &id, &seed)?.join("replay.json");
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|_| ApiError(StatusCode::NOT_FOUND, "リプレイがありません".into()))?;
    Ok(([("content-type", "application/json")], bytes).into_response())
}
#[derive(Deserialize)]
struct LogQuery {
    #[serde(default)]
    player: usize,
}
async fn team_log(
    State(state): State<AppState>,
    Path((id, seed)): Path<(String, String)>,
    Query(query): Query<LogQuery>,
) -> Result<String, ApiError> {
    if query.player >= crate::runner::MAX_PLAYERS as usize {
        return Err(bad("チーム番号が不正です"));
    }
    tail(&case_path(&state.root, &id, &seed)?.join(format!("team-{}.log", query.player)))
        .await
        .map_err(|_| ApiError(StatusCode::NOT_FOUND, "提出記録がありません".into()))
}
async fn server_log(
    State(state): State<AppState>,
    Path((id, seed)): Path<(String, String)>,
) -> Result<String, ApiError> {
    tail(&case_path(&state.root, &id, &seed)?.join("server.log"))
        .await
        .map_err(|_| ApiError(StatusCode::NOT_FOUND, "ログがありません".into()))
}
async fn tail(path: &FsPath) -> Result<String> {
    let mut f = tokio::fs::File::open(path).await?;
    let len = f.metadata().await?.len();
    let count = len.min(64 * 1024);
    f.seek(std::io::SeekFrom::Start(len - count)).await?;
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[derive(Deserialize)]
struct CompareRequest {
    ids: Vec<String>,
}
#[derive(Serialize)]
struct Comparison {
    runs: Vec<Manifest>,
    rows: Vec<comparison::ComparisonRow>,
}
/// 照合は単独・対戦テスタと同じ実装を使い、履歴だけ server の形式で返す。
async fn compare_runs(
    State(state): State<AppState>,
    Json(request): Json<CompareRequest>,
) -> Result<Json<Comparison>, ApiError> {
    let compared = comparison::compare(&state.root, &request.ids)
        .await
        .map_err(|e| bad(format!("{e:#}")))?;
    let mut runs = Vec::new();
    for id in &request.ids {
        runs.push(saved_manifest(&state, id).await?);
    }
    Ok(Json(Comparison {
        runs,
        rows: compared.rows,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::SCHEMA_VERSION;
    use procon37_common::runner::write_json;

    struct Fixture(AppState);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("hexaudon-server-{}", run_id()));
            std::fs::create_dir_all(&root).unwrap();
            Self(AppState {
                root,
                workspace: std::env::current_dir().unwrap(),
                shutdown: watch::channel(false).0,
                config: ServerConfig::default(),
                shared: SharedState::default(),
                window: Window {
                    port: 0,
                    address: None,
                    error: None,
                },
                active: Arc::new(Mutex::new(None)),
            })
        }
        async fn record(&self, id: &str, status: &str) -> Manifest {
            let dir = self.0.root.join(id);
            tokio::fs::create_dir_all(dir.join("0")).await.unwrap();
            let manifest = Manifest {
                schema_version: SCHEMA_VERSION,
                id: id.into(),
                status: status.into(),
                config: self.0.config.clone(),
                verdict: None,
                total: 1,
                completed: 0,
                cases: vec![],
                error: None,
            };
            write_json(&dir.join("run.json"), &manifest).await.unwrap();
            tokio::fs::write(dir.join("0/replay.json"), b"test replay")
                .await
                .unwrap();
            manifest
        }
        async fn remove(&self, id: &str) -> StatusCode {
            match delete_run(State(self.0.clone()), Path(id.into())).await {
                Ok(_) => StatusCode::OK,
                Err(error) => error.0,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0.root);
        }
    }

    #[tokio::test]
    async fn records_are_listed_compared_and_deleted_like_the_testers() {
        let f = Fixture::new();
        f.record("101-1", "completed").await;
        f.record("102-1", "running").await;
        let records = list_runs(State(f.0.clone()))
            .await
            .unwrap_or_else(|_| panic!("list failed"))
            .0;
        assert_eq!(
            records.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["102-1", "101-1"]
        );
        let request = CompareRequest {
            ids: vec!["101-1".into(), "102-1".into()],
        };
        let compared = compare_runs(State(f.0.clone()), Json(request))
            .await
            .unwrap_or_else(|_| panic!("compare failed"))
            .0;
        assert_eq!(compared.runs[1].config.players, 1);
        assert_eq!(f.remove("102-1").await, StatusCode::CONFLICT);
        for id in ["", "..", "../101-1", "101-1/0"] {
            assert_eq!(f.remove(id).await, StatusCode::BAD_REQUEST);
        }
        assert_eq!(f.remove("101-1").await, StatusCode::OK);
        assert_eq!(f.remove("101-1").await, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn runs_are_refused_without_a_match_window_or_during_a_run() {
        let mut f = Fixture::new();
        f.0.window.error = Some("対戦窓口のポート 11451 を開けません".into());
        let config = ServerConfig {
            count: Some(1),
            ..ServerConfig::default()
        };
        let result = start_run(State(f.0.clone()), Json(config.clone())).await;
        assert!(matches!(result, Err(ApiError(StatusCode::CONFLICT, _))));
        f.0.window.error = None;
        let manifest = f.record("101-1", "running").await;
        let (cancel, _) = watch::channel(false);
        let (finished_tx, finished) = watch::channel(false);
        *f.0.active.lock().unwrap() = Some(Active {
            view: RunView { manifest },
            cancel,
            finished,
        });
        let result = start_run(State(f.0.clone()), Json(config)).await;
        assert!(matches!(result, Err(ApiError(StatusCode::CONFLICT, _))));
        assert!(matches!(
            shutdown_idle(State(f.0.clone())).await,
            Err(ApiError(StatusCode::CONFLICT, _))
        ));
        finished_tx.send(true).unwrap();
        assert!(shutdown_idle(State(f.0.clone())).await.is_ok());
        assert_eq!(
            launcher_identity(State(f.0.clone())).await.lines().next(),
            Some("procon37-server-launcher-v1")
        );
    }
}
