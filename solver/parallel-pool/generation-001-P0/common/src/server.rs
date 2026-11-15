use crate::{
    archive,
    comparison::{self, read_manifest},
    model::MatchFormat,
    runner::*,
};
use anyhow::Result;
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
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
    build_log: String,
}
struct Active {
    view: RunView,
    cancel: watch::Sender<bool>,
    finished: watch::Receiver<bool>,
}
#[derive(Clone)]
struct AppState {
    root: PathBuf,
    workspace: PathBuf,
    shutdown: watch::Sender<bool>,
    config: RunConfig,
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
fn javascript(source: &'static str) -> axum::routing::MethodRouter<AppState> {
    get(move || async move { ([("content-type", "text/javascript; charset=utf-8")], source) })
}

pub async fn serve(root: PathBuf, port: u16, config: RunConfig) -> Result<()> {
    tokio::fs::create_dir_all(&root).await?;
    let (shutdown, mut shutdown_rx) = watch::channel(false);
    let state = AppState {
        root: root.canonicalize()?,
        workspace: std::env::current_dir()?.canonicalize()?,
        shutdown,
        config,
        active: Arc::new(Mutex::new(None)),
    };
    let config_mode = state.config.mode;
    let app = Router::new()
        .route(
            "/",
            get(move || async move {
                Html(if config_mode == Mode::Game {
                    include_str!("../../tester/game/web/index.html")
                } else {
                    include_str!("../web/index.html")
                })
            }),
        )
        .route(
            "/app.js",
            get(|| async {
                (
                    [("content-type", "text/javascript; charset=utf-8")],
                    include_str!("../web/app.js"),
                )
            }),
        )
        .route(
            "/style.css",
            get(|| async {
                (
                    [("content-type", "text/css; charset=utf-8")],
                    include_str!("../web/style.css"),
                )
            }),
        )
        .route(
            "/navigation.js",
            get(|| async {
                (
                    [("content-type", "text/javascript; charset=utf-8")],
                    include_str!("../web/navigation.js"),
                )
            }),
        )
        .route(
            "/record-stats.js",
            get(|| async {
                (
                    [("content-type", "text/javascript; charset=utf-8")],
                    include_str!("../web/record-stats.js"),
                )
            }),
        )
        .route(
            "/playback.js",
            get(|| async {
                (
                    [("content-type", "text/javascript; charset=utf-8")],
                    include_str!("../web/playback.js"),
                )
            }),
        )
        .route("/board.js", javascript(include_str!("../web/board.js")))
        .route("/viewer.js", javascript(include_str!("../web/viewer.js")))
        .route(
            "/gif-export.js",
            javascript(include_str!("../web/gif-export.js")),
        )
        .route(
            "/gif-worker.js",
            javascript(include_str!("../web/gif-worker.js")),
        )
        .route(
            "/gif-codec.js",
            javascript(include_str!("../web/gif-codec.js")),
        )
        .route(
            "/vendor/gifenc.js",
            javascript(include_str!("../web/vendor/gifenc.js")),
        )
        .route(
            "/vendor/gifenc.LICENSE.md",
            get(|| async { include_str!("../web/vendor/gifenc.LICENSE.md") }),
        )
        .route(
            "/run-config.js",
            javascript(include_str!("../web/run-config.js")),
        )
        .route(
            "/match-ui.js",
            javascript(include_str!("../../tester/game/web/match-ui.js")),
        )
        .route("/api/config", get(default_config))
        .route("/api/launcher", get(launcher_identity))
        .route("/api/shutdown", post(shutdown_idle))
        .route(
            "/api/system",
            get(|| async { Json(serde_json::json!({"availableCpus": available_cpus()})) }),
        )
        .route("/api/compare", post(compare_runs))
        .route("/api/active", get(active_run))
        .route("/api/generate", get(preview))
        .route("/api/runs", get(list_runs).post(start_run))
        .route(
            "/api/runs/import",
            post(import_run).layer(DefaultBodyLimit::max(archive::MAX_ARCHIVE_BYTES as usize)),
        )
        .route("/api/runs/{id}/export", get(export_run))
        .route("/api/runs/{id}", get(run_view).delete(delete_run))
        .route("/api/runs/{id}/cancel", post(cancel_run))
        .route("/api/runs/{id}/{seed}/replay", get(replay))
        .route("/api/runs/{id}/{seed}/stderr", get(stderr_log))
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

async fn default_config(State(state): State<AppState>) -> Json<RunConfig> {
    Json(state.config)
}
// A line-based identity lets the Bash launcher verify both paths without jq.
async fn launcher_identity(State(state): State<AppState>) -> String {
    format!(
        "{}\n{}\n{}\n",
        if state.config.mode == Mode::Game {
            "procon37-game-launcher-v1"
        } else {
            "procon37-launcher-v1"
        },
        state.workspace.display(),
        state.root.display()
    )
}
async fn shutdown_idle(State(state): State<AppState>) -> Result<Json<ValueMessage>, ApiError> {
    let active = state.active.lock().unwrap();
    if active.as_ref().is_some_and(|a| !*a.finished.borrow()) {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "テスト実行中は停止できません。GUIで完了または中断を待ってください。".into(),
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
) -> Result<Json<crate::model::Case>, ApiError> {
    let seed = if query.seed.is_empty() {
        0
    } else {
        query.seed.parse::<u64>().map_err(bad)?
    };
    let format = match query.format.as_deref() {
        None | Some("") | Some("auto") => None,
        Some(s) => Some(MatchFormat::parse(s).map_err(bad)?),
    };
    Ok(Json(
        crate::generator::generate_for_players_with_arena_maps(
            seed,
            format,
            state.config.mode.players(),
            query.arena_maps.unwrap_or(state.config.arena_maps),
        )
        .map_err(internal)?,
    ))
}

async fn start_run(
    State(state): State<AppState>,
    Json(mut config): Json<RunConfig>,
) -> Result<Json<RunView>, ApiError> {
    config.mode = state.config.mode;
    config.validate().map_err(bad)?;
    let id = run_id();
    let (tx, rx) = watch::channel(false);
    let (finished_tx, finished_rx) = watch::channel(false);
    let view = RunView {
        manifest: Manifest {
            opponent_identities: vec![],

            imported_from: None,
            schema_version: 3,
            execution: Some(config.execution(available_cpus())),
            verdict: None,
            id: id.clone(),
            status: "building".into(),
            total: config.total(),
            completed: 0,
            cases: vec![],
            error: None,
            config: config.clone(),
        },
        build_log: String::new(),
    };
    {
        let mut active = state.active.lock().unwrap();
        if *state.shutdown.borrow() {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "サーバーを停止しています".into(),
            ));
        }
        if active
            .as_ref()
            .is_some_and(|a| matches!(a.view.manifest.status.as_str(), "building" | "running"))
        {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "実行中のバッチが完了するか、中断してから開始してください".into(),
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
    let progress: Progress = Arc::new(move |event| {
        let mut guard = shared.lock().unwrap();
        if let Some(active) = guard.as_mut().filter(|a| a.view.manifest.id == event_id) {
            match event {
                ProgressEvent::Status(manifest) => active.view.manifest = *manifest,
                ProgressEvent::BuildLog(text) => {
                    active.view.build_log.push_str(&text);
                    if active.view.build_log.len() > 256_000 {
                        let mut start = active.view.build_log.len() - 200_000;
                        while !active.view.build_log.is_char_boundary(start) {
                            start += 1;
                        }
                        active.view.build_log.drain(..start);
                    }
                }
            }
        }
    });
    tokio::spawn(async move {
        if let Err(e) = run_batch(id.clone(), config, state.root, progress, rx).await
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
        if let Ok(m) = read_manifest(&p).await
            && m.config.mode == state.config.mode
        {
            results.push(m);
        }
    }
    if let Some(active) = state.active.lock().unwrap().as_ref() {
        results.retain(|m| m.id != active.view.manifest.id);
        results.insert(0, active.view.manifest.clone());
    }
    Ok(Json(results))
}

async fn require_mode(state: &AppState, id: &str) -> Result<Manifest, ApiError> {
    if !valid_id(id) {
        return Err(bad("実行IDが不正です"));
    }
    let manifest = read_manifest(&state.root.join(id)).await.map_err(|e| {
        ApiError(
            StatusCode::NOT_FOUND,
            format!("実行結果を読み込めません: {e}"),
        )
    })?;
    if manifest.config.mode != state.config.mode {
        return Err(bad("別のテスタの履歴です"));
    }
    Ok(manifest)
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
    let dir = state.root.join(&id);
    let manifest = require_mode(&state, &id).await?;
    let build_log = format!(
        "{}{}",
        tail(&dir.join("build.stdout.log"))
            .await
            .unwrap_or_default(),
        tail(&dir.join("build.stderr.log"))
            .await
            .unwrap_or_default()
    );
    Ok(Json(RunView {
        manifest,
        build_log,
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
        .ok_or_else(|| bad("このサーバーで実行中のバッチではありません"))?;
    active.cancel.send(true).map_err(bad)?;
    Ok(Json(ValueMessage {
        message: "中断を要求しました",
    }))
}

async fn export_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    if state
        .active
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|a| a.view.manifest.id == id && !*a.finished.borrow())
    {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "実行中の履歴はエクスポートできません。完了または中断を待ってください。".into(),
        ));
    }
    require_mode(&state, &id).await?;
    let file = archive::export_run(&state.root, &id).await.map_err(bad)?;
    let size = file.metadata().map_err(internal)?.len();
    let stream = tokio_util::io::ReaderStream::new(tokio::fs::File::from_std(file));
    Ok((
        [
            ("content-type", "application/zip".into()),
            (
                "content-disposition",
                format!("attachment; filename=\"run-{id}.procon37.zip\""),
            ),
            ("content-length", size.to_string()),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}

#[derive(Deserialize)]
struct ImportOptions {
    label: Option<String>,
}

async fn import_run(
    State(state): State<AppState>,
    Query(options): Query<ImportOptions>,
    bytes: Bytes,
) -> Result<Json<Manifest>, ApiError> {
    tokio::task::spawn_blocking(move || {
        archive::import_run_for_mode(
            &state.root,
            std::io::Cursor::new(bytes),
            options.label,
            state.config.mode,
        )
    })
    .await
    .map_err(internal)?
    .map(Json)
    .map_err(|e| bad(format!("{e:#}")))
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
    // Read only the manifest: a missing replay must not prevent cleanup.
    let manifest: Manifest =
        serde_json::from_slice(&tokio::fs::read(dir.join("run.json")).await.map_err(bad)?)
            .map_err(bad)?;
    if manifest.config.mode != state.config.mode {
        return Err(bad("別のテスタの履歴です"));
    }
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
    require_mode(&state, &id).await?;
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
async fn stderr_log(
    State(state): State<AppState>,
    Path((id, seed)): Path<(String, String)>,
    Query(query): Query<LogQuery>,
) -> Result<String, ApiError> {
    require_mode(&state, &id).await?;
    if query.player >= state.config.mode.players() as usize {
        return Err(bad("プレイヤー番号が不正です"));
    }
    let file = if query.player == 0 {
        "stderr.log".into()
    } else {
        format!("stderr-{}.log", query.player)
    };
    tail(&case_path(&state.root, &id, &seed)?.join(file))
        .await
        .map_err(internal)
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
async fn compare_runs(
    State(state): State<AppState>,
    Json(request): Json<CompareRequest>,
) -> Result<Json<comparison::Comparison>, ApiError> {
    for id in &request.ids {
        require_mode(&state, id).await?;
    }
    comparison::compare(&state.root, &request.ids)
        .await
        .map(Json)
        .map_err(bad)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(AppState);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("hexaudon-delete-{}", run_id()));
            std::fs::create_dir_all(&root).unwrap();
            Self(AppState {
                root,
                workspace: std::env::current_dir().unwrap(),
                shutdown: watch::channel(false).0,
                config: RunConfig::default(),
                active: Arc::new(Mutex::new(None)),
            })
        }
        async fn record(&self, id: &str, status: &str) -> Manifest {
            let dir = self.0.root.join(id);
            tokio::fs::create_dir_all(dir.join("0")).await.unwrap();
            let manifest = Manifest {
                opponent_identities: vec![],

                imported_from: None,
                schema_version: 3,
                id: id.into(),
                status: status.into(),
                config: self.0.config.clone(),
                execution: None,
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
            tokio::fs::write(dir.join("build.stderr.log"), b"test log")
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
    async fn deletion_removes_only_the_selected_record_and_clears_cached_history() {
        let f = Fixture::new();
        let manifest = f.record("101-1", "completed").await;
        f.record("102-1", "completed").await;
        let (cancel, _) = watch::channel(false);
        let (_finished, finished) = watch::channel(true);
        *f.0.active.lock().unwrap() = Some(Active {
            view: RunView {
                manifest,
                build_log: String::new(),
            },
            cancel,
            finished,
        });
        assert_eq!(f.remove("101-1").await, StatusCode::OK);
        assert!(!f.0.root.join("101-1").exists());
        assert!(f.0.root.join("102-1/0/replay.json").exists());
        assert!(active_run(State(f.0.clone())).await.0.is_none());
        let records = list_runs(State(f.0.clone()))
            .await
            .unwrap_or_else(|_| panic!("list failed"))
            .0;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "102-1");
        assert_eq!(f.remove("101-1").await, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn launcher_identifies_the_workspace_and_refuses_shutdown_during_a_test() {
        let f = Fixture::new();
        assert_eq!(
            launcher_identity(State(f.0.clone())).await,
            format!(
                "procon37-launcher-v1\n{}\n{}\n",
                f.0.workspace.display(),
                f.0.root.display()
            )
        );
        let manifest = f.record("101-1", "running").await;
        let (cancel, _) = watch::channel(false);
        let (finished_tx, finished) = watch::channel(false);
        *f.0.active.lock().unwrap() = Some(Active {
            view: RunView {
                manifest,
                build_log: String::new(),
            },
            cancel,
            finished,
        });
        let result = shutdown_idle(State(f.0.clone())).await;
        assert!(matches!(result, Err(ApiError(StatusCode::CONFLICT, _))));
        assert!(!*f.0.shutdown.borrow());
        finished_tx.send(true).unwrap();
        assert!(shutdown_idle(State(f.0.clone())).await.is_ok());
        assert!(*f.0.shutdown.borrow());
        let result = start_run(State(f.0.clone()), Json(f.0.config.clone())).await;
        assert!(matches!(result, Err(ApiError(StatusCode::CONFLICT, _))));
    }

    #[tokio::test]
    async fn deletion_rejects_running_records_and_waits_for_final_writes() {
        let f = Fixture::new();
        for status in ["building", "running"] {
            f.record("101-1", status).await;
            assert_eq!(f.remove("101-1").await, StatusCode::CONFLICT);
            assert!(f.0.root.join("101-1/0/replay.json").exists());
        }
        let manifest = f.record("101-1", "completed").await;
        let (cancel, _) = watch::channel(false);
        let (finished_tx, finished) = watch::channel(false);
        *f.0.active.lock().unwrap() = Some(Active {
            view: RunView {
                manifest,
                build_log: String::new(),
            },
            cancel,
            finished,
        });
        assert_eq!(f.remove("101-1").await, StatusCode::CONFLICT);
        finished_tx.send(true).unwrap();
        assert_eq!(f.remove("101-1").await, StatusCode::OK);
        for status in ["failed", "cancelled"] {
            f.record("101-1", status).await;
            assert_eq!(f.remove("101-1").await, StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn deletion_rejects_unsafe_paths_and_mismatched_manifests() {
        let f = Fixture::new();
        for id in ["", "..", "../101-1", "101-1/0"] {
            assert_eq!(f.remove(id).await, StatusCode::BAD_REQUEST);
        }
        let mut manifest = f.record("101-1", "completed").await;
        manifest.id = "102-1".into();
        write_json(&f.0.root.join("101-1/run.json"), &manifest)
            .await
            .unwrap();
        assert_eq!(f.remove("101-1").await, StatusCode::BAD_REQUEST);
        assert!(f.0.root.join("101-1/0/replay.json").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn deletion_does_not_follow_symlinks() {
        use std::os::unix::fs::symlink;
        let f = Fixture::new();
        let outside = Fixture::new();
        outside.record("101-1", "completed").await;
        symlink(outside.0.root.join("101-1"), f.0.root.join("101-1")).unwrap();
        assert_eq!(f.remove("101-1").await, StatusCode::BAD_REQUEST);
        f.record("102-1", "completed").await;
        symlink(&outside.0.root, f.0.root.join("102-1/external")).unwrap();
        assert_eq!(f.remove("102-1").await, StatusCode::OK);
        assert!(outside.0.root.join("101-1/0/replay.json").exists());
    }
}
