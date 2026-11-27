use crate::{
    model::{now, s},
    playback::Playback,
    player,
    storage::Store,
};
use anyhow::{Context, Result, anyhow, bail};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    process::Stdio,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    process::Child,
    sync::{oneshot, watch},
    task::JoinHandle,
    time::{Duration, Instant, sleep, timeout},
};
use tokio_util::sync::CancellationToken;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
const FINAL_SNAPSHOT_TIMEOUT: Duration = Duration::from_millis(800);
const MAX_FRAME: usize = 64 * 1024;

/// One IPC connection owns one history session, independently of the WebView.
pub struct MpvConnection {
    playback: Playback,
    session: String,
    state: Value,
    status: watch::Sender<Value>,
    position: Option<f64>,
    duration: Option<f64>,
    pending: BTreeMap<u64, Pending>,
    next_request: u64,
    resume: Option<f64>,
    resume_started: Option<Instant>,
    seek_acked: bool,
    restarted: bool,
    resume_query: bool,
    resume_retry_at: Option<Instant>,
    stopping: bool,
    ended: bool,
    ready: usize,
}

enum Command {
    Observe,
    Seek,
    ResumePosition,
    Snapshot(&'static str),
}
struct Pending {
    command: Command,
    sent: Instant,
}

impl MpvConnection {
    pub fn new(store: Arc<Store>, session: &str, entry: Value, live: bool) -> Result<Self> {
        let playback = Playback::new(store);
        playback.attach(session, if live { Value::Null } else { entry.clone() })?;
        let state = json!({
            "session":session,"event":"starting","state":"starting","progressLinked":true,
            "connected":false,"position":0.0,"duration":0.0,"paused":false,
            "savedAt":null,"entry":public_entry(&entry),"live":live,"title":s(&entry,"name")
        });
        let (status, _) = watch::channel(state.clone());
        Ok(Self {
            playback,
            session: session.into(),
            state,
            status,
            position: None,
            duration: None,
            pending: BTreeMap::new(),
            next_request: 0,
            resume: None,
            resume_started: None,
            seek_acked: false,
            restarted: false,
            resume_query: false,
            resume_retry_at: None,
            stopping: false,
            ended: false,
            ready: 0,
        })
    }

    pub fn subscribe(&self) -> watch::Receiver<Value> {
        self.status.subscribe()
    }

    pub async fn run<T>(mut self, transport: T, cancel: CancellationToken) -> Result<()>
    where
        T: AsyncRead + AsyncWrite + Unpin,
    {
        let (reader, mut writer) = tokio::io::split(transport);
        let mut reader = Frames {
            reader,
            buffer: Vec::new(),
        };
        let result = self.drive(&mut reader, &mut writer, &cancel).await;
        let saved = self.playback.finish();
        let result = match (result, saved) {
            (Err(error), Err(saved)) => Err(anyhow!("{error:#}；保存进度失败：{saved:#}")),
            (Err(error), _) => Err(error),
            (_, result) => result,
        };
        match &result {
            Ok(()) => {
                if !self.ended {
                    self.state["state"] = json!("stopped");
                }
                self.state["event"] = json!("stopped");
            }
            Err(error) => {
                self.state["state"] = json!("error");
                self.state["event"] = json!("error");
                self.state["error"] = json!(safe_diagnostic(&format!("{error:#}")));
            }
        }
        self.publish();
        result
    }

    async fn drive<R, W>(
        &mut self,
        reader: &mut Frames<R>,
        writer: &mut W,
        cancel: &CancellationToken,
    ) -> Result<()>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        for (id, property) in [
            (1, "time-pos"),
            (2, "duration"),
            (3, "pause"),
            (4, "eof-reached"),
        ] {
            self.command(
                writer,
                json!(["observe_property", id, property]),
                Command::Observe,
            )
            .await?;
        }
        let mut check = tokio::time::interval(Duration::from_millis(100));
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    self.stopping = true;
                    self.state["state"] = json!("stopping");
                    self.publish();
                    self.final_snapshot(reader, writer).await?;
                    return Ok(());
                }
                message = reader.next() => {
                    let Some(message) = message? else {
                        if self.ended { return Ok(()); }
                        bail!("mpv IPC 连接意外关闭，已保存最近可靠进度");
                    };
                    if self.message(&message, writer).await? { return Ok(()); }
                }
                _ = check.tick() => {
                    if self.pending.values().any(|pending| pending.sent.elapsed() >= COMMAND_TIMEOUT) {
                        bail!("mpv IPC 命令应答超时");
                    }
                    if self.resume_started.is_some_and(|started| started.elapsed() >= Duration::from_secs(10)) {
                        bail!("mpv 续播跳转超时，保留原有进度");
                    }
                    self.confirm_resume(writer).await?;
                }
            }
        }
    }

    async fn command<W: AsyncWrite + Unpin>(
        &mut self,
        writer: &mut W,
        command: Value,
        kind: Command,
    ) -> Result<()> {
        self.next_request += 1;
        let mut bytes =
            serde_json::to_vec(&json!({"command":command,"request_id":self.next_request}))?;
        bytes.push(b'\n');
        timeout(Duration::from_millis(300), writer.write_all(&bytes))
            .await
            .context("mpv IPC 写入超时")?
            .context("mpv IPC 写入失败")?;
        self.pending.insert(
            self.next_request,
            Pending {
                command: kind,
                sent: Instant::now(),
            },
        );
        Ok(())
    }

    async fn message<W: AsyncWrite + Unpin>(
        &mut self,
        value: &Value,
        writer: &mut W,
    ) -> Result<bool> {
        if let Some(request) = value["request_id"].as_u64() {
            if let Some(pending) = self.pending.remove(&request) {
                let error = s(value, "error");
                if error != "success" {
                    if matches!(pending.command, Command::Snapshot(_)) && self.stopping {
                        self.state["warning"] = json!(format!(
                            "最终进度查询失败：{}，保留最近进度",
                            safe_diagnostic(&error)
                        ));
                    } else {
                        bail!("mpv IPC 命令失败：{}", safe_diagnostic(&error));
                    }
                } else {
                    match pending.command {
                        Command::Observe => {
                            self.ready += 1;
                            if self.ready == 4 {
                                self.state["connected"] = json!(true);
                            }
                        }
                        Command::Seek => {
                            self.seek_acked = true;
                        }
                        Command::ResumePosition => {
                            self.resume_query = false;
                            let position =
                                finite(&value["data"]).context("mpv 续播确认没有返回有效进度")?;
                            let target = self.resume.context("mpv 续播确认缺少目标")?;
                            if (position - target).abs() > 3.0 {
                                // Initial playback-restart can precede the queued seek.
                                self.resume_retry_at =
                                    Some(Instant::now() + Duration::from_millis(250));
                            } else {
                                self.position = Some(position);
                                self.resume = None;
                                self.resume_started = None;
                                self.resume_retry_at = None;
                                self.emit("resumed", writer).await?;
                            }
                        }
                        Command::Snapshot(property) => {
                            self.property(property, &value["data"], writer).await?;
                        }
                    }
                }
            }
            self.confirm_resume(writer).await?;
            self.publish();
            return Ok(false);
        }
        match s(value, "event").as_str() {
            "property-change" => {
                if s(value, "name") == "eof-reached" && value["data"] == true {
                    self.complete(writer).await?;
                } else {
                    self.property(&s(value, "name"), &value["data"], writer)
                        .await?;
                }
            }
            "playback-restart" => {
                if self.resume.is_some() {
                    self.restarted = true;
                    self.confirm_resume(writer).await?;
                }
            }
            "end-file" => {
                if s(value, "reason") == "error" {
                    bail!(
                        "mpv 媒体加载失败：{}",
                        safe_diagnostic(&s(value, "file_error"))
                    );
                }
                if s(value, "reason") == "eof" {
                    self.complete(writer).await?;
                }
                self.ended = true;
                if self.state["state"] != "ended" {
                    self.state["state"] = json!("stopped");
                }
                self.emit("destroy", writer).await?;
                return Ok(true);
            }
            "shutdown" => {
                self.ended = true;
                if self.state["state"] != "ended" {
                    self.state["state"] = json!("stopped");
                }
                self.emit("destroy", writer).await?;
                return Ok(true);
            }
            _ => {}
        }
        self.publish();
        Ok(false)
    }

    async fn confirm_resume<W: AsyncWrite + Unpin>(&mut self, writer: &mut W) -> Result<()> {
        if self.resume.is_some()
            && self.seek_acked
            && self.restarted
            && !self.resume_query
            && self.resume_retry_at.is_none_or(|at| Instant::now() >= at)
            && !self.stopping
        {
            self.resume_query = true;
            self.command(
                writer,
                json!(["get_property", "time-pos"]),
                Command::ResumePosition,
            )
            .await?;
        }
        Ok(())
    }

    async fn property<W: AsyncWrite + Unpin>(
        &mut self,
        name: &str,
        data: &Value,
        writer: &mut W,
    ) -> Result<()> {
        match name {
            "time-pos" => {
                if let Some(position) = finite(data) {
                    if self.resume.is_none() {
                        self.position = Some(position);
                    }
                    self.emit("timeupdate", writer).await?;
                }
            }
            "duration" => {
                if let Some(duration) = finite(data).filter(|duration| *duration > 0.0) {
                    self.duration = Some(duration);
                    self.emit("durationchange", writer).await?;
                }
            }
            "pause" if data.is_boolean() => {
                self.state["paused"] = data.clone();
                self.emit(if data == true { "pause" } else { "playing" }, writer)
                    .await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn emit<W: AsyncWrite + Unpin>(&mut self, kind: &str, writer: &mut W) -> Result<()> {
        self.state["event"] = json!(kind);
        if !self.stopping && !self.ended {
            self.state["state"] = json!(if self.state["paused"] == true {
                "paused"
            } else {
                "playing"
            });
        }
        if let Some(position) = self.position {
            self.state["position"] = json!(position);
        }
        if let Some(duration) = self.duration {
            self.state["duration"] = json!(duration);
        }
        if let (Some(position), Some(duration)) = (self.position, self.duration) {
            let outcome = self.playback.event(&json!({"session":self.session,"event":kind,"position":position,"duration":duration}), now())?;
            if outcome.saved {
                let previous = self.state["savedAt"].as_u64().unwrap_or(0);
                self.state["savedAt"] = json!(now().max(previous.saturating_add(1)));
            }
            if let Some(target) = outcome
                .action
                .as_ref()
                .and_then(|action| action["seek"].as_f64())
                && !self.stopping
            {
                self.resume = Some(target);
                self.resume_started = Some(Instant::now());
                self.seek_acked = false;
                self.restarted = false;
                self.resume_retry_at = None;
                self.command(
                    writer,
                    json!(["seek", target, "absolute+exact"]),
                    Command::Seek,
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn complete<W: AsyncWrite + Unpin>(&mut self, writer: &mut W) -> Result<()> {
        if self.resume.is_none()
            && let Some(duration) = self.duration
        {
            self.position = Some(duration);
        }
        self.emit("ended", writer).await?;
        self.state["state"] = json!("ended");
        Ok(())
    }

    async fn final_snapshot<R, W>(&mut self, reader: &mut Frames<R>, writer: &mut W) -> Result<()>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let snapshot = async {
            for property in ["duration", "time-pos", "pause"] {
                self.command(
                    writer,
                    json!(["get_property", property]),
                    Command::Snapshot(property),
                )
                .await?;
            }
            while self
                .pending
                .values()
                .any(|pending| matches!(pending.command, Command::Snapshot(_)))
            {
                let Some(message) = reader.next().await? else {
                    break;
                };
                if self.message(&message, writer).await? {
                    break;
                }
            }
            Ok::<_, anyhow::Error>(())
        };
        match timeout(FINAL_SNAPSHOT_TIMEOUT, snapshot).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                self.state["warning"] = json!(safe_diagnostic(&format!(
                    "最终进度查询失败，保留最近进度：{error:#}"
                )));
            }
            Err(_) => {
                self.state["warning"] = json!("最终进度查询超时，保留最近进度");
            }
        }
        self.emit("destroy", writer).await?;
        if !self.ended {
            // Quit is best effort after the reliable history snapshot has been saved.
            if let Err(error) = self
                .command(writer, json!(["quit"]), Command::Snapshot("quit"))
                .await
            {
                self.state["warning"] =
                    json!(safe_diagnostic(&format!("关闭 mpv IPC 失败：{error:#}")));
            }
        }
        Ok(())
    }

    fn publish(&mut self) {
        self.state["at"] = json!(now());
        self.status.send_replace(self.state.clone());
    }
}

struct Frames<R> {
    reader: R,
    buffer: Vec<u8>,
}
impl<R: AsyncRead + Unpin> Frames<R> {
    async fn next(&mut self) -> Result<Option<Value>> {
        loop {
            if let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
                if end >= MAX_FRAME {
                    bail!("mpv IPC 消息超出大小限制");
                }
                let frame: Vec<_> = self.buffer.drain(..=end).collect();
                if frame.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                return Ok(Some(
                    serde_json::from_slice(&frame).context("mpv IPC 返回无效 JSON")?,
                ));
            }
            if self.buffer.len() >= MAX_FRAME {
                bail!("mpv IPC 消息超出大小限制");
            }
            let mut bytes = [0; 4096];
            let count = self
                .reader
                .read(&mut bytes)
                .await
                .context("mpv IPC 读取失败")?;
            if count == 0 {
                if !self.buffer.is_empty() {
                    bail!("mpv IPC 在 JSON 消息中途断开");
                }
                return Ok(None);
            }
            self.buffer.extend_from_slice(&bytes[..count]);
        }
    }
}

fn finite(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .filter(|value| value.is_finite() && *value >= 0.0)
}
fn public_entry(entry: &Value) -> Value {
    let mut value = json!({});
    for key in [
        "siteKey",
        "vodId",
        "name",
        "lineIndex",
        "epIndex",
        "epName",
        "lineName",
    ] {
        if let Some(field) = entry.get(key) {
            value[key] = field.clone();
        }
    }
    value
}
fn safe_diagnostic(text: &str) -> String {
    static URL: OnceLock<Regex> = OnceLock::new();
    static SECRET: OnceLock<Regex> = OnceLock::new();
    let url =
        URL.get_or_init(|| Regex::new(r#"(?i)[a-z][a-z0-9+.-]*://[^\s"'<>]+"#).expect("URL regex"));
    let secret = SECRET.get_or_init(|| {
        Regex::new(r"(?i)authorization|cookie|password|token|api[-_]?key|referer|http-header")
            .expect("secret regex")
    });
    text.lines()
        .take(40)
        .map(|line| {
            if secret.is_match(line) {
                "[已隐藏可能含凭据的播放器诊断]".into()
            } else {
                url.replace_all(line, "[媒体地址]")
                    .chars()
                    .take(300)
                    .collect::<String>()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub struct ExternalPlayback {
    store: Arc<Store>,
    operation: tokio::sync::Mutex<()>,
    active: Mutex<Option<Active>>,
    sequence: AtomicU64,
    status: Arc<watch::Sender<Value>>,
}
struct Active {
    session: String,
    cancel: CancellationToken,
    task: Option<JoinHandle<Result<()>>>,
}

impl ExternalPlayback {
    pub fn new(store: Arc<Store>) -> Self {
        let (status, _) = watch::channel(json!({"state":"idle","progressLinked":false}));
        Self {
            store,
            operation: tokio::sync::Mutex::new(()),
            active: Mutex::new(None),
            sequence: AtomicU64::new(0),
            status: Arc::new(status),
        }
    }
    pub fn status(&self) -> Value {
        self.status.borrow().clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<Value> {
        self.status.subscribe()
    }

    pub async fn open(
        &self,
        address: &str,
        title: &str,
        entry: Value,
        live: bool,
    ) -> Result<Value> {
        self.open_cancellable(address, title, entry, live, &CancellationToken::new())
            .await
    }

    pub async fn open_cancellable(
        &self,
        address: &str,
        title: &str,
        entry: Value,
        live: bool,
        caller: &CancellationToken,
    ) -> Result<Value> {
        if caller.is_cancelled() {
            bail!("外部播放请求已取消");
        }
        let request = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        self.cancel(None)?;
        let _operation = tokio::select! {
            biased;
            _ = caller.cancelled() => bail!("外部播放请求已取消"),
            operation = self.operation.lock() => operation,
        };
        if caller.is_cancelled() || request != self.sequence.load(Ordering::SeqCst) {
            bail!("外部播放请求已取消");
        }
        self.stop_active(None).await?;
        if caller.is_cancelled() || request != self.sequence.load(Ordering::SeqCst) {
            bail!("外部播放请求已取消");
        }
        let session = uuid::Uuid::new_v4().to_string();
        let cancel = caller.child_token();
        *self
            .active
            .lock()
            .map_err(|_| anyhow!("外部播放状态锁异常"))? = Some(Active {
            session: session.clone(),
            cancel: cancel.clone(),
            task: None,
        });
        self.status.send_replace(json!({"session":session,"event":"starting","state":"starting","progressLinked":false,"entry":public_entry(&entry),"live":live,"title":title}));
        let launch = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(anyhow!("外部播放请求已取消")),
            result = player::resolve(&self.store, address, title) => result,
        };
        let launch = launch.and_then(|launch| {
            if cancel.is_cancelled() {
                bail!("外部播放请求已取消");
            }
            Ok(launch)
        });
        let result = match launch {
            Ok(launch) if launch.is_mpv() => self.start_mpv(launch, &session, title, entry, live, cancel.clone()).await,
            Ok(launch) => {
                player::launch(&launch).map(|_| {
                    let value = json!({"session":session,"event":"launched","state":"unlinked","player":launch.command,"progressLinked":false,"message":"已交给外部播放器；此播放器不支持进度和续播联动","entry":public_entry(&entry),"live":live,"title":title});
                    self.status.send_replace(value.clone());
                    value
                })
            }
            Err(error) => Err(error),
        };
        if let Err(error) = &result {
            cancel.cancel();
            let cleanup = self.stop_active(Some(&session)).await;
            let detail = cleanup
                .err()
                .map(|cleanup| format!("{error:#}；关闭失败：{cleanup:#}"))
                .unwrap_or_else(|| format!("{error:#}"));
            let mut value = self.status();
            value["state"] = json!("error");
            value["event"] = json!("error");
            value["error"] = json!(safe_diagnostic(&detail));
            self.status.send_replace(value);
        }
        result
    }

    pub async fn stop_session(&self, session: &str) -> Result<()> {
        self.cancel(Some(session))?;
        let _operation = self.operation.lock().await;
        self.stop_active(Some(session)).await
    }
    pub async fn finish(&self) -> Result<()> {
        self.sequence.fetch_add(1, Ordering::SeqCst);
        self.cancel(None)?;
        let _operation = self.operation.lock().await;
        self.stop_active(None).await
    }
    fn cancel(&self, session: Option<&str>) -> Result<()> {
        let active = self
            .active
            .lock()
            .map_err(|_| anyhow!("外部播放状态锁异常"))?;
        if let Some(active) = active
            .as_ref()
            .filter(|active| session.is_none_or(|id| id == active.session))
        {
            active.cancel.cancel();
        }
        Ok(())
    }
    async fn stop_active(&self, session: Option<&str>) -> Result<()> {
        let active = {
            let mut active = self
                .active
                .lock()
                .map_err(|_| anyhow!("外部播放状态锁异常"))?;
            if active
                .as_ref()
                .is_some_and(|active| session.is_none_or(|id| id == active.session))
            {
                active.take()
            } else {
                None
            }
        };
        if let Some(active) = active {
            active.cancel.cancel();
            if let Some(mut task) = active.task {
                match timeout(Duration::from_secs(5), &mut task).await {
                    Ok(result) => result.context("mpv 会话任务异常")??,
                    Err(_) => {
                        task.abort();
                        let _ = task.await;
                        bail!("mpv 会话关闭超时，已取消子进程");
                    }
                }
            }
        }
        Ok(())
    }

    async fn start_mpv(
        &self,
        launch: player::Launch,
        session: &str,
        title: &str,
        entry: Value,
        live: bool,
        cancel: CancellationToken,
    ) -> Result<Value> {
        #[cfg(not(windows))]
        {
            let _ = (launch, session, title, entry, live, cancel);
            bail!("mpv 进度联动目前需要 Windows 命名管道");
        }
        #[cfg(windows)]
        {
            if cancel.is_cancelled() {
                bail!("外部播放请求已取消");
            }
            let connection = MpvConnection::new(self.store.clone(), session, entry, live)?;
            let pipe = format!(r"\\.\pipe\crayon-mpv-{}", uuid::Uuid::new_v4());
            let mut args = launch.args;
            let insert = args
                .iter()
                .position(|arg| arg == "--")
                .unwrap_or(args.len());
            args.splice(
                insert..insert,
                [
                    format!("--input-ipc-server={pipe}"),
                    "--resume-playback=no".into(),
                    "--save-position-on-quit=no".into(),
                    "--input-terminal=no".into(),
                ],
            );
            let mut command = tokio::process::Command::new(&launch.command);
            command
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .creation_flags(0x08000000);
            if cancel.is_cancelled() {
                bail!("外部播放请求已取消");
            }
            let child = command.spawn().context("无法启动 mpv")?;
            let (ready, started) = oneshot::channel();
            let task = tokio::spawn(run_process(
                child,
                pipe,
                connection,
                self.status.clone(),
                title.into(),
                cancel.clone(),
                ready,
            ));
            if let Some(active) = self
                .active
                .lock()
                .map_err(|_| anyhow!("外部播放状态锁异常"))?
                .as_mut()
                .filter(|active| active.session == session)
            {
                active.task = Some(task);
            } else {
                task.abort();
                bail!("外部播放请求已取消");
            }
            tokio::select! {
                biased;
                _ = cancel.cancelled() => bail!("外部播放请求已取消"),
                result = timeout(Duration::from_secs(9), started) => { result.context("mpv IPC 初始化超时")?.context("mpv 初始化任务提前退出")?.map_err(|error| anyhow!(error))?; }
            }
            Ok(self.status())
        }
    }
}
impl Drop for ExternalPlayback {
    fn drop(&mut self) {
        if let Ok(active) = self.active.get_mut()
            && let Some(active) = active.as_ref()
        {
            active.cancel.cancel();
        }
    }
}

#[cfg(windows)]
async fn run_process(
    mut child: Child,
    pipe: String,
    connection: MpvConnection,
    status: Arc<watch::Sender<Value>>,
    title: String,
    cancel: CancellationToken,
    ready: oneshot::Sender<std::result::Result<(), String>>,
) -> Result<()> {
    let mut ready = Some(ready);
    let mut updates = connection.subscribe();
    let session = s(&updates.borrow(), "session");
    let mut stderr = child.stderr.take().context("无法读取 mpv 标准错误输出")?;
    let mut diagnostics = DiagnosticTask(tokio::spawn(async move {
        let mut tail = Vec::new();
        let mut bytes = [0; 4096];
        loop {
            let count = stderr.read(&mut bytes).await.context("读取 mpv 诊断失败")?;
            if count == 0 {
                break;
            }
            tail.extend_from_slice(&bytes[..count]);
            if tail.len() > 16 * 1024 {
                tail.drain(..tail.len() - 16 * 1024);
            }
        }
        Ok::<_, anyhow::Error>(safe_diagnostic(&String::from_utf8_lossy(&tail)))
    }));
    let started = Instant::now();
    let transport = loop {
        if cancel.is_cancelled() {
            break Err(anyhow!("mpv 初始化已取消"));
        }
        match child.try_wait() {
            Ok(Some(exit)) => break Err(anyhow!("mpv 在 IPC 连接前退出（{exit}）")),
            Ok(None) => {}
            Err(error) => break Err(error).context("无法读取 mpv 子进程状态"),
        }
        match tokio::net::windows::named_pipe::ClientOptions::new().open(&pipe) {
            Ok(transport) => break Ok(transport),
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    || error.raw_os_error() == Some(231) => {}
            Err(error) => break Err(error).context("无法连接 mpv 命名管道"),
        }
        if started.elapsed() >= Duration::from_secs(6) {
            break Err(anyhow!("mpv 命名管道连接超时"));
        }
        tokio::select! { _ = cancel.cancelled() => {}, _ = sleep(Duration::from_millis(40)) => {} }
    };
    let mut result = match transport {
        Ok(transport) => {
            let ipc = connection.run(transport, cancel.clone());
            tokio::pin!(ipc);
            loop {
                tokio::select! {
                    result = &mut ipc => break result,
                    changed = updates.changed() => {
                        if changed.is_err() { continue; }
                        let mut update = updates.borrow_and_update().clone(); update["title"] = json!(title);
                        if update["connected"] == true && let Some(ready) = ready.take() { let _ = ready.send(Ok(())); }
                        publish_session(&status, &session, update);
                    }
                }
            }
        }
        Err(_) if cancel.is_cancelled() => Ok(()),
        Err(error) => Err(error),
    };
    let cleanup = close_child(&mut child).await;
    let exit_error = match &cleanup {
        Ok(Some(error)) => Some(error.clone()),
        Err(error) => Some(format!("{error:#}")),
        Ok(None) => None,
    };
    if let Some(error) = exit_error {
        result = Err(match result {
            Ok(()) => anyhow!(error),
            Err(previous) => anyhow!("{previous:#}；{error:#}"),
        });
    }
    let diagnostic = match timeout(Duration::from_millis(500), &mut diagnostics.0).await {
        Ok(Ok(Ok(text))) => text,
        Ok(Ok(Err(error))) => {
            result = Err(error);
            String::new()
        }
        Ok(Err(error)) => {
            result = Err(error).context("mpv 诊断任务异常");
            String::new()
        }
        Err(_) => {
            diagnostics.0.abort();
            let _ = (&mut diagnostics.0).await;
            "mpv 诊断流关闭超时，已取消读取".into()
        }
    };
    let mut update = updates.borrow().clone();
    update["title"] = json!(title);
    if cancel.is_cancelled() && result.is_ok() {
        update["state"] = json!("stopped");
        update["event"] = json!("stopped");
    }
    if !diagnostic.is_empty() {
        update["diagnostics"] = json!(diagnostic);
    }
    if let Err(error) = &result {
        update["state"] = json!("error");
        update["event"] = json!("error");
        update["error"] = json!(safe_diagnostic(&format!("{error:#}")));
    }
    publish_session(&status, &session, update);
    if let Some(ready) = ready {
        let _ = ready.send(Err(result
            .as_ref()
            .err()
            .map(|error| safe_diagnostic(&format!("{error:#}")))
            .unwrap_or_else(|| "mpv 在 IPC 初始化完成前退出".into())));
    }
    // Playback errors remain in structured status; only cleanup failure blocks a new session.
    cleanup.map(|_| ())
}

#[cfg(windows)]
struct DiagnosticTask(JoinHandle<Result<String>>);
#[cfg(windows)]
impl Drop for DiagnosticTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(windows)]
fn publish_session(status: &watch::Sender<Value>, session: &str, value: Value) {
    status.send_if_modified(|current| {
        if s(current, "session") == session {
            *current = value;
            true
        } else {
            false
        }
    });
}

#[cfg(windows)]
async fn close_child(child: &mut Child) -> Result<Option<String>> {
    if let Some(exit) = child.try_wait().context("读取 mpv 退出状态失败")? {
        return Ok((!exit.success()).then(|| format!("mpv 异常退出（{exit}）")));
    }
    match timeout(Duration::from_millis(800), child.wait()).await {
        Ok(result) => {
            let exit = result.context("等待 mpv 退出失败")?;
            if !exit.success() {
                return Ok(Some(format!("mpv 异常退出（{exit}）")));
            }
        }
        Err(_) => {
            child.start_kill().context("无法终止 mpv 子进程")?;
            timeout(Duration::from_secs(1), child.wait())
                .await
                .context("终止 mpv 后等待退出超时")?
                .context("回收 mpv 子进程失败")?;
        }
    }
    Ok(None)
}
