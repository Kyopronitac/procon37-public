use crate::model::*;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdin, Command},
    sync::mpsc,
    task::JoinHandle,
    time::{Duration, Instant, timeout_at},
};
pub type LogProgress = Arc<dyn Fn(String) + Send + Sync>;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandSpec {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: PathBuf,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}
impl CommandSpec {
    pub fn command(&self) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args)
            .current_dir(&self.cwd)
            .envs(&self.env)
            .kill_on_drop(true);
        #[cfg(unix)]
        {
            cmd.process_group(0);
        }
        cmd
    }
    pub fn normalize(&mut self) -> Result<()> {
        ensure!(
            !self.program.trim().is_empty(),
            "実行するプログラムが空です"
        );
        self.cwd = self
            .cwd
            .canonicalize()
            .with_context(|| format!("作業ディレクトリが存在しません: {}", self.cwd.display()))?;
        if Path::new(&self.program).is_relative() && self.program.contains('/') {
            self.program = self.cwd.join(&self.program).to_string_lossy().into_owned();
        }
        Ok(())
    }
}

struct ProcessGroup(Option<u32>);
impl ProcessGroup {
    fn stop(&mut self) {
        if let Some(pid) = self.0.take() {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
    }
}
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn copy_log(
    mut stream: impl AsyncRead + Unpin,
    mut file: tokio::fs::File,
    progress: Option<LogProgress>,
) -> Result<()> {
    let mut buffer = [0; 8192];
    let mut saved = 0usize;
    const LIMIT: usize = 16 * 1024 * 1024;
    loop {
        let n = stream.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        if saved < LIMIT {
            let kept = n.min(LIMIT - saved);
            file.write_all(&buffer[..kept]).await?;
            saved += kept;
            if let Some(p) = &progress {
                p(String::from_utf8_lossy(&buffer[..kept]).into_owned());
            }
            if saved == LIMIT {
                file.write_all(b"\n[log truncated after 16 MiB; remaining output drained]\n")
                    .await?;
            }
        }
    }
    file.flush().await?;
    Ok(())
}

pub async fn run_build(spec: &CommandSpec, directory: &Path, progress: LogProgress) -> Result<()> {
    progress(format!("{} {:?}\n", spec.program, spec.args));
    let mut child = spec
        .command()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("ビルドを起動できませんでした")?;
    let mut group = ProcessGroup(child.id());
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let out = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("build.stdout.log"))
        .await?;
    let err = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("build.stderr.log"))
        .await?;
    let wait = async {
        let status = child.wait().await;
        group.stop();
        status
    };
    let (out_result, err_result, status) = tokio::join!(
        copy_log(stdout, out, Some(progress.clone())),
        copy_log(stderr, err, Some(progress)),
        wait
    );
    out_result?;
    err_result?;
    let status = status?;
    ensure!(status.success(), "ビルドが失敗しました: {status}");
    Ok(())
}

pub struct SolverProcess {
    child: Child,
    stdin: ChildStdin,
    rx: mpsc::Receiver<Result<String, String>>,
    group: ProcessGroup,
    stdout_task: JoinHandle<()>,
    stderr_task: JoinHandle<Result<()>>,
}
impl SolverProcess {
    pub async fn spawn(spec: &CommandSpec, directory: &Path) -> Result<Self> {
        Self::spawn_named(spec, directory, "stderr.log").await
    }
    pub async fn spawn_named(spec: &CommandSpec, directory: &Path, log_name: &str) -> Result<Self> {
        let file = tokio::fs::File::create(directory.join(log_name)).await?;
        let mut child = spec
            .command()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("ソルバを起動できませんでした")?;
        let group = ProcessGroup(child.id());
        let stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (tx, rx) = mpsc::channel(16);
        let stdout_task = tokio::spawn(async move {
            const MAX_LINE: usize = 1024 * 1024;
            let mut line = Vec::new();
            let mut discard = false;
            let mut buffer = [0; 4096];
            loop {
                let n = match stdout.read(&mut buffer).await {
                    Ok(0) => {
                        if !line.is_empty() {
                            let _ = tx.send(Err("回答末尾に改行がありません".into())).await;
                        }
                        break;
                    }
                    Ok(n) => n,
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string())).await;
                        break;
                    }
                };
                for &byte in &buffer[..n] {
                    if byte == b'\n' {
                        if !discard {
                            let text = String::from_utf8(std::mem::take(&mut line))
                                .map_err(|_| "回答がUTF-8ではありません".to_owned());
                            if tx.send(text).await.is_err() {
                                return;
                            }
                        }
                        discard = false;
                    } else if !discard {
                        line.push(byte);
                        if line.len() > MAX_LINE {
                            line.clear();
                            discard = true;
                            if tx
                                .send(Err("回答が1 MiBを超えました".into()))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                }
            }
        });
        let stderr_task = tokio::spawn(copy_log(stderr, file, None));
        Ok(Self {
            child,
            stdin,
            rx,
            group,
            stdout_task,
            stderr_task,
        })
    }
    pub async fn exchange(&mut self, request: &Request) -> (Option<Value>, Option<Failure>, u64) {
        let start = Instant::now();
        let deadline = start + Duration::from_millis(request.time_limit_ms);
        let result = timeout_at(deadline, async {
            let mut data =
                serde_json::to_vec(request).map_err(|e| Failure::new("protocol", e.to_string()))?;
            data.push(b'\n');
            self.stdin
                .write_all(&data)
                .await
                .map_err(|e| Failure::new("process", format!("要求を送信できません: {e}")))?;
            self.stdin
                .flush()
                .await
                .map_err(|e| Failure::new("process", e.to_string()))?;
            loop {
                let line = self
                    .rx
                    .recv()
                    .await
                    .ok_or_else(|| Failure::new("process", "ソルバの標準出力が終了しました"))?
                    .map_err(|e| Failure::new("protocol", e))?;
                let response: Response = serde_json::from_str(&line).map_err(|e| {
                    Failure::new(
                        "protocol",
                        format!(
                            "回答JSONが不正です: {e}; {}",
                            line.chars().take(160).collect::<String>()
                        ),
                    )
                })?;
                if response.request_id < request.request_id {
                    continue;
                }
                if response.request_id != request.request_id {
                    return Err(Failure::new("protocol", "要求IDが一致しません"));
                }
                return Ok(response.answer);
            }
        })
        .await;
        let elapsed = start.elapsed().as_millis() as u64;
        match result {
            Ok(Ok(v)) => (Some(v), None, elapsed),
            Ok(Err(e)) => (None, Some(e), elapsed),
            Err(_) => (
                None,
                Some(Failure::new(
                    "timeout",
                    format!("{} msの制限時間を超えました", request.time_limit_ms),
                )),
                elapsed,
            ),
        }
    }
    pub async fn finish(&mut self) {
        self.group.stop();
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
        let _ = tokio::time::timeout(Duration::from_secs(2), &mut self.stderr_task).await;
        self.stdout_task.abort();
        self.stderr_task.abort();
    }
}
impl Drop for SolverProcess {
    fn drop(&mut self) {
        self.group.stop();
        let _ = self.child.start_kill();
        self.stdout_task.abort();
        self.stderr_task.abort();
    }
}
