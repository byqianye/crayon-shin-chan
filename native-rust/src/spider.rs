use crate::{
    config::is_windows_path,
    http::Net,
    model::{merge, now, s},
    storage::Store,
};
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{Mutex as AsyncMutex, oneshot},
};
use tokio_util::sync::CancellationToken;

static SECRET_ENV: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)TOKEN|SECRET|PASSWORD|PASSWD|API[_-]?KEY|ACCESS[_-]?KEY|PRIVATE[_-]?KEY|CREDENTIAL|COOKIE|SESSION|AUTH|DEEPSEEK|OPENAI|ANTHROPIC|GITHUB|GH_|AWS_|AZURE_|ALIYUN|TENCENT").expect("constant regex")
});
type Reply = oneshot::Sender<std::result::Result<Value, String>>;

struct PendingRequest {
    session: Arc<Session>,
    id: u64,
    armed: bool,
}
impl Drop for PendingRequest {
    fn drop(&mut self) {
        if !self.armed
            || !self
                .session
                .pending
                .lock()
                .expect("pending lock")
                .remove(&self.id)
                .is_some()
        {
            return;
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let session = self.session.clone();
            let id = self.id;
            runtime.spawn(async move {
                let _ = session
                    .write(&json!({"id":0,"method":"cancel","params":{"id":id}}))
                    .await;
            });
        }
    }
}

pub struct Spider {
    pub root: PathBuf,
    pub store: Arc<Store>,
    net: Net,
    session: AsyncMutex<Option<Arc<Session>>>,
    preparing: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
}

struct Session {
    child: AsyncMutex<Child>,
    input: AsyncMutex<ChildStdin>,
    pending: Mutex<HashMap<u64, Reply>>,
    opening: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    opened: Mutex<HashSet<String>>,
    next: AtomicU64,
    last: AtomicU64,
    alive: AtomicBool,
    timed_out: AtomicUsize,
}

fn process(executable: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(executable);
    command.env_clear().envs(
        std::env::vars_os().filter(|(name, _)| !SECRET_ENV.is_match(&name.to_string_lossy())),
    );
    command.kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command
}

impl Spider {
    pub fn new(store: Arc<Store>, net: Net) -> Self {
        let root = std::env::var_os("SPIDER_HOST_ROOT")
            .map(PathBuf::from)
            .or_else(|| {
                let executable = std::env::current_exe().ok()?;
                let folder = executable.parent()?;
                [
                    folder.join("spider-host"),
                    folder.join("resources/spider-host"),
                ]
                .into_iter()
                .find(|p| p.is_dir())
            })
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../spider-host"));
        Self {
            root,
            store,
            net,
            session: AsyncMutex::new(None),
            preparing: Mutex::new(HashMap::new()),
        }
    }

    pub fn java(&self) -> PathBuf {
        if let Some(path) = std::env::var_os("SPIDER_JAVA") {
            return path.into();
        }
        let bundled = self.root.join("runtime/bin/java.exe");
        if bundled.is_file() {
            return bundled;
        }
        if let Some(home) = std::env::var_os("JAVA_HOME") {
            let path = PathBuf::from(home).join("bin/java.exe");
            if path.is_file() {
                return path;
            }
        }
        PathBuf::from("java")
    }

    fn classpath(&self) -> Result<std::ffi::OsString> {
        let paths = [
            self.root.join("out"),
            self.root.join("lib/android-4.1.1.4.jar"),
            self.root.join("lib/json-20090211.jar"),
            self.root.join("lib/*"),
        ];
        std::env::join_paths(paths).context("Spider 类路径无效")
    }

    async fn ready(&self) -> Result<Arc<Session>> {
        let mut slot = self.session.lock().await;
        if let Some(session) = slot.as_ref().filter(|s| s.alive.load(Ordering::Acquire)) {
            return Ok(session.clone());
        }
        let work = self.store.root.join("spider-work");
        tokio::fs::create_dir_all(work.join("tmp")).await?;
        let mut child = process(self.java())
            .args([
                "-Xms16m",
                "-Xmx192m",
                "-XX:+UseSerialGC",
                "-XX:TieredStopAtLevel=1",
                "-XX:MaxMetaspaceSize=96m",
                "-XX:ReservedCodeCacheSize=48m",
                "-noverify",
                "-Dfile.encoding=UTF-8",
            ])
            .arg(format!("-Duser.home={}", work.display()))
            .arg(format!("-Djava.io.tmpdir={}", work.join("tmp").display()))
            .arg("-cp")
            .arg(self.classpath()?)
            .arg("host.SpiderHost")
            .current_dir(&work)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("无法启动 Spider Java 宿主")?;
        let stdout = child.stdout.take().context("Spider 输出管道未建立")?;
        let stderr = child.stderr.take().context("Spider 错误管道未建立")?;
        let input = child.stdin.take().context("Spider 输入管道未建立")?;
        let session = Arc::new(Session {
            child: AsyncMutex::new(child),
            input: AsyncMutex::new(input),
            pending: Mutex::new(HashMap::new()),
            opening: Mutex::new(HashMap::new()),
            opened: Mutex::new(HashSet::new()),
            next: AtomicU64::new(1),
            last: AtomicU64::new(now()),
            alive: AtomicBool::new(true),
            timed_out: AtomicUsize::new(0),
        });
        let (ready, receiver) = oneshot::channel();
        let weak = Arc::downgrade(&session);
        tokio::spawn(async move {
            let mut ready = Some(ready);
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if message["event"] == "ready" {
                    if let Some(sender) = ready.take() {
                        let _ = sender.send(());
                    }
                    continue;
                }
                let Some(session) = weak.upgrade() else {
                    return;
                };
                if let Some(id) = message["id"].as_u64()
                    && let Some(reply) = session.pending.lock().expect("pending lock").remove(&id)
                {
                    session.timed_out.store(0, Ordering::Release);
                    let value = if message["ok"] == true {
                        Ok(message["data"].clone())
                    } else {
                        Err(s(&message, "error"))
                    };
                    let _ = reply.send(value);
                }
            }
            if let Some(session) = weak.upgrade() {
                session.fail_all("Spider 宿主已退出");
            }
        });
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if std::env::var_os("SPIDER_DEBUG").is_some() {
                    eprintln!("[spider] {line}");
                }
            }
        });
        match tokio::time::timeout(Duration::from_secs(15), receiver).await {
            Ok(Ok(())) => {}
            _ => {
                session.stop().await?;
                bail!("Spider 宿主未就绪，请检查 runtime、out 和 lib 是否齐全");
            }
        }
        let weak = Arc::downgrade(&session);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let Some(session) = weak.upgrade() else {
                    return;
                };
                if !session.alive.load(Ordering::Acquire) {
                    return;
                }
                if now().saturating_sub(session.last.load(Ordering::Acquire)) >= 300_000
                    && session.pending.lock().expect("pending lock").is_empty()
                {
                    if let Err(error) = session.stop().await {
                        eprintln!("[spider] 空闲退出失败：{error:#}");
                    }
                    return;
                }
            }
        });
        *slot = Some(session.clone());
        Ok(session)
    }

    pub async fn ping(&self) -> Result<Value> {
        self.ready()
            .await?
            .send("ping", json!({}), 10.0, &CancellationToken::new())
            .await
    }
    pub async fn stop(&self) -> Result<()> {
        if let Some(session) = self.session.lock().await.take() {
            session.stop().await?;
        }
        Ok(())
    }

    pub async fn call(
        &self,
        site: &Value,
        method: &str,
        params: Value,
        timeout: f64,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        if cancel.is_cancelled() {
            bail!("请求已取消");
        }
        if s(site, "spiderClass").is_empty() {
            bail!("该站点没有 Spider 实现类");
        }
        let session = tokio::select! { _ = cancel.cancelled() => bail!("请求已取消"), value = self.ready() => value? };
        let identity = serde_json::to_string(&json!([
            site["sourceId"],
            site["jar"],
            site["spiderClass"],
            site["ext"]
        ]))?;
        let runtime = format!("{}:{:x}", s(site, "key"), md5::compute(identity));
        let opening = session
            .opening
            .lock()
            .expect("opening lock")
            .entry(runtime.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();
        {
            let _guard = tokio::select! { _ = cancel.cancelled() => bail!("请求已取消"), guard = opening.lock() => guard };
            let opened = session
                .opened
                .lock()
                .expect("opened lock")
                .contains(&runtime);
            if !opened {
                let (jar, raw) = self.prepare(site, cancel).await?;
                let ext = self.prepare_ext(site, &raw, cancel).await?;
                session.send("open", json!({"site":runtime,"jar":jar,"raw":raw,"class":site["spiderClass"],"ext":ext}), 90.0, cancel).await?;
                session
                    .opened
                    .lock()
                    .expect("opened lock")
                    .insert(runtime.clone());
            }
        }
        let mut params = params;
        params["site"] = json!(runtime);
        session.send(method, params, timeout, cancel).await
    }

    async fn prepare(
        &self,
        site: &Value,
        cancel: &CancellationToken,
    ) -> Result<(PathBuf, PathBuf)> {
        let address = s(site, "jar");
        if address.is_empty() {
            bail!("配置没有提供 spider 文件");
        }
        let key = cache_key(site);
        let gate = self
            .preparing
            .lock()
            .expect("preparing lock")
            .entry(key.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();
        let _guard = tokio::select! { _ = cancel.cancelled() => bail!("请求已取消"), guard = gate.lock() => guard };
        let dir = self.store.root.join("spider-cache");
        tokio::fs::create_dir_all(&dir).await?;
        let output = dir.join(format!("{key}.jar"));
        let raw = dir.join(format!("{key}.raw.zip"));
        if output.metadata().is_ok_and(|v| v.len() > 0) && raw.is_file() {
            return Ok((output, raw));
        }
        if raw.is_file() {
            let bytes = tokio::fs::read(&raw).await?;
            if looks_converted(&bytes) {
                return Ok((raw.clone(), raw));
            }
        }
        let bytes = if is_windows_path(&address) || address.starts_with('/') {
            tokio::fs::read(&address)
                .await
                .context("无法读取 Spider 文件")?
        } else {
            self.net
                .get(
                    address.split(";md5;").next().unwrap_or(&address),
                    &json!({"User-Agent":"Mozilla/5.0"}),
                    60.0,
                    cancel,
                )
                .await?
                .bytes
        };
        let start = bytes
            .windows(4)
            .position(|v| v == b"PK\x03\x04")
            .context("spider 文件不是有效的 zip/dex")?;
        tokio::fs::write(&raw, &bytes[start..]).await?;
        if looks_converted(&bytes[start..]) {
            return Ok((raw.clone(), raw));
        }
        let configured = std::env::var_os("SPIDER_DEX_TOOLS").map(PathBuf::from);
        let lib = configured
            .into_iter()
            .chain([
                self.root.join("dex-tools"),
                self.root.join("dex-tools/dex-tools-v2.4"),
            ])
            .map(|v| v.join("lib"))
            .find(|v| v.is_dir())
            .context("缺少 dex2jar，请运行 spider-host/fetch-tools.ps1")?;
        let command = process(self.java())
            .arg("-cp")
            .arg(lib.join("*"))
            .arg("com.googlecode.dex2jar.tools.Dex2jarCmd")
            .arg("-f")
            .arg(&raw)
            .arg("-o")
            .arg(&output)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output();
        let result = tokio::select! { _ = cancel.cancelled() => bail!("请求已取消"), result = tokio::time::timeout(Duration::from_secs(90), command) => result.context("dex2jar 转换超时")?? };
        if !result.status.success() || !output.is_file() {
            if output.is_file() {
                tokio::fs::remove_file(&output).await?;
            }
            bail!(
                "dex2jar 转换失败：{}",
                tail(&String::from_utf8_lossy(&result.stderr), 400)
            );
        }
        Ok((output, raw))
    }

    async fn prepare_ext(
        &self,
        site: &Value,
        raw: &Path,
        cancel: &CancellationToken,
    ) -> Result<String> {
        let ext = s(site, "ext");
        if ext.is_empty() || ext.starts_with(['{', '[']) || crate::http::is_http_url(&ext) {
            return Ok(ext);
        }
        let bytes = tokio::fs::read(raw).await?;
        if ![b"FishGuard-v8.so".as_slice(), b"FishGuard-v7.so".as_slice()]
            .iter()
            .any(|needle| bytes.windows(needle.len()).any(|w| w == *needle))
        {
            return Ok(ext);
        }
        let key = format!("{:x}", md5::compute(format!("{}|{ext}", raw.display())));
        let dir = self.store.root.join("spider-cache");
        let cached = dir.join(format!("{key}.ext.txt"));
        if cached.is_file() {
            return Ok(tokio::fs::read_to_string(cached).await?.trim().to_owned());
        }
        let input = dir.join(format!("{key}.{}.ext.input", uuid::Uuid::new_v4()));
        tokio::fs::write(&input, &ext).await?;
        let classpath = std::env::join_paths([
            self.root.join("unidbg-out"),
            self.root.join("unidbg-libs/*"),
        ])?;
        let result = async {
            let command = process(self.java()).args(["-Xmx256m", "-cp"]).arg(classpath).arg("host.FishGuardBridge").arg("--jar").arg(raw).args(["--method", "extDe", "--input-file"]).arg(&input).output();
            let output = tokio::select! { _ = cancel.cancelled() => bail!("请求已取消"), result = tokio::time::timeout(Duration::from_secs(90), command) => result.context("FishGuard 解密超时")?? };
            let text = String::from_utf8_lossy(&output.stdout);
            let value: Value = text.lines().rev().find(|v| v.trim().starts_with("{\"ok\"")).and_then(|v| serde_json::from_str(v).ok()).context("FishGuard 没有返回解密结果")?;
            if value["ok"] != true || !value["text"].is_string() { bail!("{}", s(&value, "error")); }
            let value = s(&value, "text");
            tokio::fs::write(cached, &value).await?;
            Ok::<_, anyhow::Error>(value)
        }.await;
        tokio::fs::remove_file(input)
            .await
            .context("清理 FishGuard 临时输入失败")?;
        match result {
            Ok(value) => Ok(value),
            Err(error) if !cancel.is_cancelled() => {
                eprintln!("[spider] ext 解密失败，继续使用原 ext：{error:#}");
                Ok(ext)
            }
            Err(error) => Err(error),
        }
    }

    pub fn runtime_status(&self) -> Value {
        let root = &self.root;
        let java = self.java();
        let bundled = root.join("runtime/bin/java.exe").is_file();
        let host = root.join("out/host/SpiderHost.class").is_file();
        let android = root.join("lib/android-4.1.1.4.jar").is_file();
        let json = root.join("lib/json-20090211.jar").is_file();
        let dex = root.join("dex-tools/lib").is_dir()
            || root.join("dex-tools/dex-tools-v2.4/lib").is_dir();
        let mut missing = Vec::new();
        if !bundled {
            missing.push("内置 JRE");
        }
        if !host {
            missing.push("宿主 class");
        }
        if !android || !json {
            missing.push("Android/JSON 依赖");
        }
        if !dex {
            missing.push("dex2jar");
        }
        let ready = missing.is_empty();
        let java_source = if bundled {
            "bundled"
        } else if std::env::var_os("JAVA_HOME")
            .is_some_and(|home| PathBuf::from(home).join("bin/java.exe").is_file())
        {
            "java_home"
        } else {
            "system"
        };
        json!({"root":root,"hostRoot":root,"java":java,"javaPath":java,"javaSource":java_source,"runtime":bundled,"host":host,"hostClasses":host,"android":android,"json":json,"libraries":android&&json,"dexTools":dex,"ready":ready,"available":ready,"reason":if ready {Value::Null} else {json!(format!("缺少 {}",missing.join("、")))}})
    }

    pub async fn cache_info(&self, sites: &[Value]) -> Result<Value> {
        let dir = self.store.root.join("spider-cache");
        let (mut files, mut bytes) = (0, 0);
        if dir.is_dir() {
            let mut entries = tokio::fs::read_dir(&dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                let metadata = entry.metadata().await?;
                if metadata.is_file() {
                    files += 1;
                    bytes += metadata.len();
                }
            }
        }
        let mut out = Vec::new();
        let mut digests = HashMap::new();
        for site in sites.iter().filter(|v| !s(v, "spiderClass").is_empty()) {
            let key = cache_key(site);
            if !digests.contains_key(&key) {
                let raw = dir.join(format!("{key}.raw.zip"));
                let jar = dir.join(format!("{key}.jar"));
                let path = if raw.is_file() { &raw } else { &jar };
                let info = if path.is_file() {
                    let data = tokio::fs::read(path).await?;
                    let size = [raw, jar]
                        .iter()
                        .filter_map(|v| v.metadata().ok())
                        .map(|v| v.len())
                        .sum::<u64>();
                    json!({"cached":true,"bytes":size,"sha256":format!("{:x}", Sha256::digest(data))})
                } else {
                    json!({"cached":false,"bytes":0})
                };
                digests.insert(key.clone(), info);
            }
            let mut value = json!({"siteKey":site["key"],"siteName":site["name"]});
            merge(&mut value, &digests[&key]);
            out.push(value);
        }
        Ok(json!({"dir":dir,"files":files,"bytes":bytes,"sites":out}))
    }

    pub async fn clear_cache(&self, sites: &[Value], site_key: Option<&str>) -> Result<Value> {
        self.stop().await?;
        let dir = self.store.root.join("spider-cache");
        let keys: HashSet<String> = sites
            .iter()
            .filter(|v| site_key.is_none_or(|k| s(v, "key") == k))
            .map(cache_key)
            .collect();
        if dir.is_dir() {
            let mut entries = tokio::fs::read_dir(&dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                let name = entry.file_name().to_string_lossy().to_string();
                if entry.file_type().await?.is_file()
                    && (site_key.is_none() || keys.iter().any(|k| name.starts_with(k)))
                {
                    tokio::fs::remove_file(entry.path()).await?;
                }
            }
        }
        self.cache_info(sites).await
    }
}

impl Session {
    async fn write(&self, message: &Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(message)?;
        bytes.push(b'\n');
        self.input
            .lock()
            .await
            .write_all(&bytes)
            .await
            .context("Spider 管道写入失败")
    }
    fn fail_all(&self, error: &str) {
        self.alive.store(false, Ordering::Release);
        for (_, reply) in self.pending.lock().expect("pending lock").drain() {
            let _ = reply.send(Err(error.to_owned()));
        }
    }
    async fn send(
        self: &Arc<Self>,
        method: &str,
        params: Value,
        timeout: f64,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        if !self.alive.load(Ordering::Acquire) {
            bail!("Spider 宿主已退出");
        }
        if cancel.is_cancelled() {
            bail!("请求已取消");
        }
        self.last.store(now(), Ordering::Release);
        let id = self.next.fetch_add(1, Ordering::AcqRel);
        let (reply, receive) = oneshot::channel();
        self.pending.lock().expect("pending lock").insert(id, reply);
        let mut pending_request = PendingRequest {
            session: self.clone(),
            id,
            armed: true,
        };
        if let Err(error) = self
            .write(&json!({"id":id,"method":method,"params":params}))
            .await
        {
            self.pending.lock().expect("pending lock").remove(&id);
            pending_request.armed = false;
            return Err(error);
        }
        let response = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(anyhow::anyhow!("请求已取消")),
            result = tokio::time::timeout(Duration::from_secs_f64(timeout.clamp(1.0, 300.0)), receive) => match result {
                Ok(Ok(result)) => result.map_err(anyhow::Error::msg),
                Ok(Err(_)) => Err(anyhow::anyhow!("Spider 宿主已退出")),
                Err(_) => { self.timed_out.fetch_add(1, Ordering::AcqRel); Err(anyhow::anyhow!("Spider {method} 超时（{timeout}s）")) },
            }
        };
        let pending = self
            .pending
            .lock()
            .expect("pending lock")
            .remove(&id)
            .is_some();
        pending_request.armed = false;
        if pending {
            let _ = self
                .write(&json!({"id":0,"method":"cancel","params":{"id":id}}))
                .await;
        }
        if self.timed_out.load(Ordering::Acquire) >= 3 {
            self.stop().await?;
        }
        self.last.store(now(), Ordering::Release);
        response
    }
    async fn stop(&self) -> Result<()> {
        self.fail_all("Spider 宿主已退出");
        let _ = tokio::time::timeout(
            Duration::from_millis(500),
            self.write(&json!({"id":0,"method":"shutdown","params":{}})),
        )
        .await;
        let mut child = self.child.lock().await;
        match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
            Ok(result) => {
                result.context("等待 Spider 宿主退出失败")?;
            }
            Err(_) => child.kill().await.context("终止 Spider 宿主失败")?,
        }
        Ok(())
    }
}

pub fn cache_key(site: &Value) -> String {
    format!(
        "{:x}",
        md5::compute(format!("{}|{}", s(site, "sourceId"), s(site, "jar")))
    )
}
fn looks_converted(bytes: &[u8]) -> bool {
    !bytes.windows(11).any(|v| v == b"classes.dex") && bytes.windows(6).any(|v| v == b".class")
}
fn tail(text: &str, count: usize) -> String {
    text.chars()
        .rev()
        .take(count)
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}
pub fn encode(input: &str) -> String {
    url::form_urlencoded::byte_serialize(input.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}
pub fn make_url(site: &str, flag: &str, id: &str) -> String {
    format!("spider://{}/{}/{}", encode(site), encode(flag), encode(id))
}
pub fn parse_url(input: &str) -> Option<(String, String, String)> {
    let mut parts = input.trim().strip_prefix("spider://")?.splitn(3, '/');
    let decode = |value: &str| -> Option<String> {
        url::form_urlencoded::parse(format!("v={value}").as_bytes())
            .next()
            .map(|(_, v)| v.into_owned())
    };
    Some((
        decode(parts.next()?)?,
        decode(parts.next()?)?,
        decode(parts.next()?)?,
    ))
}
pub fn classify_failure(message: &str) -> &'static str {
    static RULES: LazyLock<Vec<(Regex, &str)>> = LazyLock::new(|| {
        [
        (r"(?i)ClassNotFoundException|NoClassDefFoundError|Class not found|Could not find class", "missing_class"),
        (r"(?i)not a Spider subclass", "wrong_type"), (r"(?i)UnsatisfiedLinkError|loadLibrary|native-lib|\.so\b", "native"),
        (r"(?i)NoSuchMethodError|Stub!|AbstractMethodError|NoSuchFieldError", "android_api"), (r"(?i)SSLHandshakeException|SSLException|PKIX|certificate|TLS", "tls"),
        (r"(?i)timeout|timed out|超时", "timeout"), (r"解析页|需要继续走解析", "parse_required"), (r"(?i)HTTP \d|请求失败|网络|fetch failed", "http"), (r"(?i)没有返回|为空|empty", "empty_search"),
    ].into_iter().map(|(pattern, kind)| (Regex::new(pattern).expect("constant regex"), kind)).collect()
    });
    RULES
        .iter()
        .find(|(pattern, _)| pattern.is_match(message))
        .map(|(_, kind)| *kind)
        .unwrap_or("unknown")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dropping_request_releases_host_pending_slot() {
        let folder = tempfile::tempdir().unwrap();
        let spider = Spider::new(
            Arc::new(Store::open(folder.path()).unwrap()),
            Net::new().unwrap(),
        );
        let session = spider.ready().await.unwrap();
        let cancel = CancellationToken::new();
        let mut request = Box::pin(session.send("ping", json!({}), 10.0, &cancel));
        assert!(futures_util::poll!(&mut request).is_pending());
        assert_eq!(session.pending.lock().unwrap().len(), 1);
        drop(request);
        let released = session.pending.lock().unwrap().is_empty();
        spider.stop().await.unwrap();
        assert!(released, "切页丢弃请求时须释放宿主待响应槽");
    }
}
