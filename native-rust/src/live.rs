use crate::{
    catalogue::Catalogue,
    http::header_map,
    media::Media,
    model::{array, compare_cn, now, number, s},
};
use anyhow::{Context, Result, bail};
use futures_util::{StreamExt, stream};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

pub const BUILTIN: &str = "游戏赛事,#genre#\ndanking直播,huya://10188\n";
static ATTR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)([\w-]+)="([^"]*)""#).expect("constant regex"));
static ADDRESS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(https?|rtmp|rtsp|rtp|udp|file|huya|douyin)://").expect("constant regex")
});

pub fn normalize_group(input: &str) -> String {
    let name = input
        .trim_start_matches(|c: char| c.is_whitespace() || "•·・*-—".contains(c))
        .replace(
            [
                '「', '」', '【', '】', '《', '》', '[', ']', '（', '）', '(', ')',
            ],
            "",
        );
    let name = name.trim();
    if name.is_empty()
        || [
            "未分组",
            "其他",
            "其它",
            "默认",
            "default",
            "undefined",
            "null",
        ]
        .contains(&name.to_lowercase().as_str())
    {
        return "其它".into();
    }
    for (prefix, normalized) in [
        ("游戏", "游戏赛事"),
        ("影视", "影视轮播"),
        ("咪咕", "咪咕移动"),
    ] {
        if name.starts_with(prefix) {
            return normalized.into();
        }
    }
    name.to_owned()
}

pub fn parse(input: &str) -> Vec<Value> {
    let mut out = Vec::new();
    if input.to_uppercase().contains("#EXTINF") || input.to_uppercase().contains("#EXTM3U") {
        let mut pending = None;
        for line in input.lines().map(str::trim).filter(|v| !v.is_empty()) {
            if line.to_uppercase().starts_with("#EXTINF") {
                let body = line.split_once(':').map(|v| v.1).unwrap_or(line);
                let (attrs, label) = body.rsplit_once(',').unwrap_or((body, ""));
                let attributes: HashMap<_, _> = ATTR
                    .captures_iter(attrs)
                    .map(|c| (c[1].to_lowercase(), c[2].trim().to_owned()))
                    .collect();
                let name = attributes
                    .get("tvg-name")
                    .filter(|v| !v.is_empty())
                    .cloned()
                    .unwrap_or_else(|| label.trim().into());
                pending = Some(
                    json!({"name":name,"group":normalize_group(attributes.get("group-title").map(String::as_str).unwrap_or("")),"logo":attributes.get("tvg-logo")}),
                );
            } else if !line.starts_with('#')
                && let Some(mut channel) = pending.take()
            {
                if s(&channel, "name").is_empty() {
                    channel["name"] = json!(line);
                }
                channel["url"] = json!(line);
                out.push(channel);
            }
        }
    } else {
        let mut group = "其它".to_owned();
        for line in input
            .lines()
            .map(str::trim)
            .filter(|v| !v.is_empty() && !v.starts_with("//"))
        {
            if line.to_lowercase().contains("#genre") {
                group = normalize_group(line.split(',').next().unwrap_or(""));
                continue;
            }
            if line.starts_with('#') {
                continue;
            }
            if let Some((name, url)) = line.split_once(',') {
                let (name, url) = (name.trim(), url.trim());
                if name.is_empty() || url.is_empty() {
                    continue;
                }
                let urls = if ADDRESS.is_match(url) {
                    vec![url]
                } else {
                    url.split('#')
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                        .collect()
                };
                for url in urls {
                    out.push(json!({"name":name,"url":url,"group":group}));
                }
            }
        }
    }
    out
}

pub fn merge_channels(raw: &[Value]) -> Vec<Value> {
    let mut channels: Vec<Value> = Vec::new();
    let mut by_name = HashMap::new();
    for channel in raw {
        let name = s(channel, "name");
        let url = json!({"url":channel["url"],"source":channel["source"]});
        if let Some(&index) = by_name.get(&name) {
            let existing: &mut Value = &mut channels[index];
            if !array(&existing["urls"])
                .iter()
                .any(|v| v["url"] == url["url"])
            {
                existing["urls"].as_array_mut().expect("urls").push(url);
            }
            if s(existing, "logo").is_empty() && !s(channel, "logo").is_empty() {
                existing["logo"] = channel["logo"].clone();
            }
        } else {
            by_name.insert(name.clone(), channels.len());
            channels.push(
                json!({"name":name,"group":channel["group"],"logo":channel["logo"],"urls":[url]}),
            );
        }
    }
    let mut groups: Vec<Value> = Vec::new();
    for channel in channels {
        let name = s(&channel, "group");
        if let Some(group) = groups.iter_mut().find(|v| s(v, "name") == name) {
            group["channels"]
                .as_array_mut()
                .expect("channels")
                .push(channel);
        } else {
            groups.push(json!({"name":name,"channels":[channel]}));
        }
    }
    for group in &mut groups {
        group["channels"]
            .as_array_mut()
            .expect("channels")
            .sort_by(|a, b| compare_cn(&s(a, "name"), &s(b, "name")));
    }
    groups.sort_by(|a, b| {
        (s(a, "name") == "其它")
            .cmp(&(s(b, "name") == "其它"))
            .then_with(|| {
                array(&b["channels"])
                    .len()
                    .cmp(&array(&a["channels"]).len())
            })
            .then_with(|| compare_cn(&s(a, "name"), &s(b, "name")))
    });
    groups
}

pub struct Live {
    pub media: Arc<Media>,
    cache: Mutex<Option<(u64, u64, Value)>>,
    load_lock: tokio::sync::Mutex<()>,
}
impl Live {
    pub fn new(media: Arc<Media>) -> Self {
        Self {
            media,
            cache: Mutex::new(None),
            load_lock: tokio::sync::Mutex::new(()),
        }
    }
    pub fn catalogue(&self) -> &Arc<Catalogue> {
        &self.media.catalogue
    }
    pub async fn load(&self, force: bool) -> Result<Value> {
        let _guard = self.load_lock.lock().await;
        let sources = &self.catalogue().sources;
        let revision = sources.revision();
        if !force
            && let Some((at, generation, value)) = &*self.cache.lock().expect("live cache")
            && *generation == revision
            && now().saturating_sub(*at) < 600_000
        {
            return Ok(value.clone());
        }
        let mut seen = HashSet::new();
        let lists: Vec<_> = sources
            .configs()?
            .iter()
            .flat_map(|v| array(&v["lives"]))
            .filter(|v| !s(v, "url").is_empty() && seen.insert(s(v, "url")))
            .collect();
        let timeout = number(&sources.store.settings()?["timeout"], 20.0);
        let loaded: Vec<_> = stream::iter(lists.into_iter().map(|source| async move {
            let address = s(&source, "url");
            let name = if s(&source, "name").is_empty() {
                address.clone()
            } else {
                s(&source, "name")
            };
            let result = async {
                if crate::http::is_http_url(&address) {
                    sources
                        .net
                        .text(
                            &address,
                            &self.media.headers(&address)?,
                            timeout,
                            &CancellationToken::new(),
                        )
                        .await
                } else {
                    let path = if address.starts_with("file:") {
                        url::Url::parse(&address)?
                            .to_file_path()
                            .map_err(|_| anyhow::anyhow!("本地直播路径无效"))?
                    } else {
                        std::path::PathBuf::from(&address)
                    };
                    Ok(tokio::fs::read_to_string(path).await?)
                }
            }
            .await;
            let mut channels = Vec::new();
            let mut stat = json!({"name":name,"url":address,"channels":0});
            match result {
                Ok::<String, anyhow::Error>(text) => {
                    channels = parse(&text);
                    stat["channels"] = json!(channels.len());
                    if channels.is_empty() {
                        stat["error"] = json!("没有解析出任何频道，格式可能不兼容");
                    }
                    for channel in &mut channels {
                        channel["source"] = json!(name);
                    }
                }
                Err(error) => stat["error"] = json!(format!("{error:#}")),
            }
            (stat, channels)
        }))
        .buffered(8)
        .collect()
        .await;
        let mut raw = parse(BUILTIN);
        for item in &mut raw {
            item["source"] = json!("内置自定义频道");
        }
        let mut stats =
            vec![json!({"name":"内置自定义频道","url":"builtin://custom","channels":raw.len()})];
        for (stat, channels) in loaded {
            stats.push(stat);
            raw.extend(channels);
        }
        let result = json!({"groups":merge_channels(&raw),"total":raw.len(),"sources":stats});
        if sources.revision() == revision {
            *self.cache.lock().expect("live cache") = Some((now(), revision, result.clone()));
        }
        Ok(result)
    }
    pub fn health(&self) -> Result<Value> {
        self.catalogue()
            .sources
            .store
            .read("live-health", json!({}))
    }
    pub fn clear_health(&self) -> Result<()> {
        self.catalogue()
            .sources
            .store
            .write("live-health", &json!({}))
    }
    pub async fn probe(&self, urls: &[String], force: bool) -> Result<Value> {
        let store = &self.catalogue().sources.store;
        let health = self.health()?;
        let mut seen = HashSet::new();
        let targets: Vec<_> = urls
            .iter()
            .map(|v| v.trim().to_owned())
            .filter(|v| {
                crate::http::is_http_url(v)
                    && seen.insert(v.clone())
                    && (force || !fresh(&health[v]))
            })
            .take(60)
            .collect();
        let timeout = number(&store.settings()?["timeout"], 20.0).min(12.0);
        let outcomes: Vec<_> = stream::iter(targets.into_iter().map(|url| async move {
            let started = Instant::now();
            let result = self.playlist(&url, timeout, None).await;
            let mut result = match result {
                Ok(text) => {
                    let mut value = json!({"ok":true});
                    for (field, pattern) in [
                        ("height", r"(?i)RESOLUTION=\d+x(\d+)"),
                        ("bandwidth", r"(?i)BANDWIDTH=(\d+)"),
                    ] {
                        let max = Regex::new(pattern)
                            .expect("constant regex")
                            .captures_iter(&text)
                            .filter_map(|v| v[1].parse::<u64>().ok())
                            .max();
                        if let Some(max) = max {
                            value[field] = json!(max);
                        }
                    }
                    value
                }
                Err(error) => json!({"ok":false,"error":format!("{error:#}")}),
            };
            result["ms"] = json!(started.elapsed().as_millis() as u64);
            result["at"] = json!(now());
            (url, result)
        }))
        .buffered(8)
        .collect()
        .await;
        store.update("live-health", json!({}), |health| {
            for (url, result) in outcomes {
                health[url] = result;
            }
            Ok(health.clone())
        })
    }
    async fn playlist(&self, address: &str, timeout: f64, referer: Option<&str>) -> Result<String> {
        let mut headers = self.media.headers(address)?;
        if let Some(referer) = referer {
            headers["Referer"] = json!(referer);
        }
        let response = self
            .catalogue()
            .sources
            .net
            .client
            .get(address)
            .headers(header_map(&headers)?)
            .timeout(Duration::from_secs_f64(timeout))
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            bail!("HTTP {status}");
        }
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let mut chunks = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk?;
            let count = chunk.len().min(4096 - bytes.len());
            bytes.extend_from_slice(&chunk[..count]);
            if bytes.len() >= 4096 {
                break;
            }
        }
        let text = String::from_utf8_lossy(&bytes).into_owned();
        if !text.to_uppercase().contains("#EXTM3U") {
            bail!("返回的不是 m3u8（{content_type}）");
        }
        Ok(text)
    }
    pub async fn resolve_room(&self, raw: &str) -> Result<Value> {
        static ROOM: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(?i)^(huya|douyin)://([a-z0-9_-]+)$").expect("constant regex")
        });
        let capture = ROOM.captures(raw.trim()).context("不认识的房间地址")?;
        let room = &capture[2];
        let net = &self.catalogue().sources.net;
        let cancel = CancellationToken::new();
        let result = if capture[1].eq_ignore_ascii_case("huya") {
            let referer = "https://www.huya.com/";
            let text = net
                .text(
                    &format!("https://mp.huya.com/cache.php?m=Live&do=profileRoom&roomid={room}"),
                    &json!({"Referer":referer,"Accept":"application/json, text/plain, */*"}),
                    12.0,
                    &cancel,
                )
                .await?;
            let payload: Value =
                serde_json::from_str(&text).context("虎牙返回的不是 JSON，接口可能改了")?;
            let mut lines = huya_lines(&payload)?;
            let rank = |v: &Value| match s(v, "cdnType").as_str() {
                "HS" => 0,
                "AL" => 2,
                _ => 1,
            };
            lines.sort_by_key(rank);
            let count = lines.len();
            let mut fallback = None;
            for line in lines {
                let url = s(&line, "url");
                if let Ok(text) = self.playlist(&url, 5.0, Some(referer)).await {
                    let resolved = json!({"url":url,"referer":referer,"line":line["cdnType"]});
                    if text.contains("#EXTINF") {
                        fallback = Some(resolved);
                        break;
                    }
                    if fallback.is_none() {
                        fallback = Some(resolved);
                    }
                }
            }
            fallback.with_context(|| format!("虎牙给了 {count} 条线路，但没有一条能拉下来"))?
        } else {
            let referer = "https://live.douyin.com/";
            let home = net
                .get(referer, &json!({"Accept":"text/html"}), 12.0, &cancel)
                .await?;
            let cookie = home
                .headers
                .get_all("set-cookie")
                .iter()
                .filter_map(|v| v.to_str().ok())
                .map(|v| v.split(';').next().unwrap_or(""))
                .collect::<Vec<_>>()
                .join("; ");
            let query = "aid=6383&app_name=douyin_web&live_id=1&device_platform=web&language=zh-CN&enter_from=web_live&cookie_enabled=true&screen_width=1920&screen_height=1080&browser_language=zh-CN&browser_platform=Win32&browser_name=Chrome&browser_version=126.0.0.0";
            let text=net.text(&format!("{referer}webcast/room/web/enter/?{query}&web_rid={room}"),&json!({"Referer":format!("{referer}{room}"),"Cookie":cookie,"Accept":"application/json, text/plain, */*"}),12.0,&cancel).await?;
            douyin_result(
                &serde_json::from_str(&text).context("抖音返回的不是 JSON，可能需要签名参数了")?,
            )?
        };
        self.media
            .register(&s(&result, "url"), &json!({"Referer":result["referer"]}));
        Ok(result)
    }
}
pub fn fresh(health: &Value) -> bool {
    let at = number(&health["at"], 0.0) as u64;
    at > 0
        && now().saturating_sub(at)
            < if health["ok"] == true {
                1_800_000
            } else {
                300_000
            }
}
pub fn huya_lines(payload: &Value) -> Result<Vec<Value>> {
    let data = &payload["data"];
    if !data.is_object() {
        bail!("{}", s(payload, "message"));
    }
    match s(data, "liveStatus").to_uppercase().as_str() {
        "" | "OFF" => bail!("主播当前没有开播"),
        "REPLAY" => bail!("主播当前没有开播（房间在放录像）"),
        _ => {}
    }
    let lines: Vec<_> = array(&data["stream"]["hls"]["multiLine"])
        .into_iter()
        .filter(|v| s(v, "url").to_lowercase().contains(".m3u8"))
        .collect();
    if lines.is_empty() {
        bail!("虎牙没有给出 HLS 线路");
    }
    Ok(lines)
}
pub fn douyin_result(payload: &Value) -> Result<Value> {
    let room = &payload["data"]["data"][0];
    if !room.is_object() {
        bail!("抖音没有返回房间数据，房间号可能不对");
    }
    match number(&room["status"], 0.0) as i64 {
        4 => bail!("主播当前没有开播（本场已结束）"),
        2 => {}
        _ => bail!("主播当前没有开播"),
    };
    let map = room["stream_url"]["hls_pull_url_map"]
        .as_object()
        .context("抖音没有给出 HLS 线路")?;
    let address = ["FULL_HD1", "HD1", "SD1", "SD2"]
        .iter()
        .find_map(|k| {
            map.get(*k)
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
        })
        .or_else(|| map.values().find_map(Value::as_str))
        .context("抖音没有给出 HLS 线路")?;
    Ok(json!({"url":address,"referer":"https://live.douyin.com/","line":room["owner"]["nickname"]}))
}
