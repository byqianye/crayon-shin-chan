use crate::{
    catalogue::Catalogue,
    http::{DEFAULT_UA, header_map, match_headers},
    media_probe::{SAMPLE_LIMIT, identify, normalized_mime},
    model::{array, merge, now, number, s},
    spider,
};
use anyhow::{Context, Result, bail};
use axum::{
    Router,
    body::Body,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use futures_util::{StreamExt, stream};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

pub const SAMPLE_TARGET: &str = "https://v.qq.com/x/cover/mzc00200mp8vo9b.html";
static MEDIA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\.(m3u8|mp4|flv|mkv|ts|m2ts|mts|mov|avi|webm|m4v|mpd|mp3|m4a|aac|flac|ogg|opus|wav)([?#]|$)")
        .expect("constant regex")
});
static NON_MEDIA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\.(js|mjs|cjs|css|json|xml|html?|png|jpe?g|gif|webp|svg|ico|woff2?|ttf|eot|map|txt|apk|zip)([?#]|$)").expect("constant regex")
});
static DIRECT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(https?|rtmp|rtsp|file)://").expect("constant regex"));
pub fn is_direct(input: &str) -> bool {
    let lower = input.trim().to_lowercase();
    ["rtmp:", "rtsp:", "file:"]
        .iter()
        .any(|v| lower.starts_with(v))
        || MEDIA.is_match(&lower)
}
pub fn is_hls(input: &str) -> bool {
    let lower = input.to_lowercase();
    lower.contains("/hls/")
        || lower
            .split(['?', '#'])
            .next()
            .is_some_and(|v| v.ends_with(".m3u8"))
}
fn unescape(input: &str) -> String {
    input
        .replace(r"\/", "/")
        .replace(r"\u002f", "/")
        .replace(r"\u002F", "/")
        .replace(r"\u0026", "&")
        .replace("&amp;", "&")
        .trim()
        .to_owned()
}
fn playable(value: &str, strict: bool) -> Option<String> {
    let url = unescape(value);
    if !DIRECT.is_match(&url) || url.chars().any(char::is_whitespace) {
        return None;
    }
    if if strict {
        is_direct(&url)
    } else {
        !NON_MEDIA.is_match(&url)
    } {
        Some(url)
    } else {
        None
    }
}
fn deep_url(value: &Value, depth: usize) -> Option<String> {
    if depth > 4 {
        return None;
    }
    if let Some(value) = value.as_str() {
        return playable(value, false);
    }
    if let Some(items) = value.as_array() {
        return items.iter().find_map(|v| deep_url(v, depth + 1));
    }
    let object = value.as_object()?;
    for (key, value) in object {
        if [
            "url", "playurl", "play_url", "m3u8", "src", "video", "link", "address",
        ]
        .contains(&key.to_lowercase().as_str())
            && let Some(url) = deep_url(value, depth + 1)
        {
            return Some(url);
        }
    }
    ["data", "result", "info", "content"]
        .iter()
        .find_map(|key| object.get(*key).and_then(|v| deep_url(v, depth + 1)))
}
pub fn extract_url(input: &str, strict: bool) -> Option<String> {
    let input = input.trim();
    if let Some(url) = playable(input, strict) {
        return Some(url);
    }
    if let Ok(payload) = crate::config::parse_json_text(input)
        && let Some(url) = deep_url(&payload, 0).and_then(|v| playable(&v, strict))
    {
        return Some(url);
    }
    static SCAN: LazyLock<Vec<Regex>> = LazyLock::new(|| {
        [
            r#"(?i)https?://[^\s'"<>()]+\.m3u8(?:\?[^\s'"<>()]*)?"#,
            r#"(?i)https?://[^\s'"<>()]+\.(?:mp4|flv|mkv|mov|webm|m4v|mpd)(?:\?[^\s'"<>()]*)?"#,
        ]
        .map(|v| Regex::new(v).expect("constant regex"))
        .into()
    });
    let flat = unescape(input);
    for pattern in SCAN.iter() {
        if let Some(found) = pattern.find(&flat) {
            return Some(found.as_str().to_owned());
        }
    }
    static PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
        [
            r#"(?i)\bvar\s+main\s*=\s*['"]([^'"]+)['"]"#,
            r#"(?i)\bvar\s+url\s*=\s*['"]([^'"]+)['"]"#,
            r#""url"\s*:\s*"([^"]+)""#,
            r#"'url'\s*:\s*'([^']+)'"#,
            r#"(?i)"playUrl"\s*:\s*"([^"]+)""#,
            r#"(?i)<(?:video|source)[^>]+src=["']([^"']+)["']"#,
            r#"(?i)(?:src|url)\s*=\s*["'](https?:\\?/\\?/[^"']+)["']"#,
        ]
        .map(|v| Regex::new(v).expect("constant regex"))
        .into()
    });
    for pattern in PATTERNS.iter() {
        for capture in pattern.captures_iter(input) {
            if let Some(url) = playable(&capture[1], strict) {
                return Some(url);
            }
        }
    }
    None
}
pub fn redirect(input: &str, base: &str) -> Option<String> {
    static PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
        [
            r#"(?i)<meta[^>]+http-equiv=["']?refresh["']?[^>]*content=["'][^"']*?url=([^"';\s>]+)"#,
            r#"(?i)content=["'][^"']*?url=([^"';\s>]+)["'][^>]*http-equiv=["']?refresh"#,
            r#"(?i)(?:window\.)?location\.href\s*=\s*['"]([^'"]+)['"]"#,
            r#"(?i)(?:window\.)?location\.replace\(\s*['"]([^'"]+)['"]"#,
            r#"(?i)(?:window\.)?location\s*=\s*['"]([^'"]+)['"]"#,
        ]
        .map(|v| Regex::new(v).expect("constant regex"))
        .into()
    });
    for pattern in PATTERNS.iter() {
        if let Some(capture) = pattern.captures(input) {
            let target = unescape(&capture[1]);
            if crate::http::is_http_url(&target) && target != base {
                return Some(target);
            }
        }
    }
    None
}
pub fn parse_address(template: &str, target: &str) -> Result<String> {
    if template.trim().is_empty() {
        bail!("解析接口地址为空");
    }
    let encoded = spider::encode(target);
    static SLOT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\{url\}").expect("constant regex"));
    static END: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"[?&][^=&]*=$").expect("constant regex"));
    let template = template.trim();
    Ok(if SLOT.is_match(template) {
        SLOT.replace_all(template, encoded).to_string()
    } else if END.is_match(template) {
        format!("{template}{encoded}")
    } else {
        format!(
            "{template}{}url={encoded}",
            if template.contains('?') { "&" } else { "?" }
        )
    })
}

pub struct Media {
    pub catalogue: Arc<Catalogue>,
    grants: Mutex<HashMap<String, (u64, Value)>>,
    client: reqwest::Client,
}
impl Media {
    pub fn new(catalogue: Arc<Catalogue>) -> Result<Self> {
        Ok(Self {
            catalogue,
            grants: Mutex::new(HashMap::new()),
            client: reqwest::Client::builder()
                .user_agent(DEFAULT_UA)
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(15))
                .build()?,
        })
    }
    pub fn register(&self, address: &str, input: &Value) {
        let Some(origin) = origin(address) else {
            return;
        };
        let mut grants = self.grants.lock().expect("grants lock");
        grants.remove(&origin);
        let mut headers = json!({});
        if let Some(input) = input.as_object() {
            for (key, value) in input {
                if ["user-agent", "referer", "origin", "cookie", "authorization"]
                    .contains(&key.to_lowercase().as_str())
                    && value.as_str().is_some_and(|v| !v.contains(['\r', '\n']))
                {
                    headers[key] = value.clone();
                }
            }
        }
        if headers.as_object().is_some_and(|v| !v.is_empty()) {
            grants.insert(origin, (now(), headers));
        }
        while grants.len() > 64 {
            let key = grants
                .iter()
                .min_by_key(|(_, v)| v.0)
                .map(|(k, _)| k.clone())
                .expect("grants");
            grants.remove(&key);
        }
    }
    pub fn headers(&self, address: &str) -> Result<Value> {
        let sources = &self.catalogue.sources;
        let rules: Vec<_> = sources
            .configs()?
            .iter()
            .flat_map(|v| array(&v["headers"]))
            .collect();
        let mut headers = json!({"User-Agent":sources.store.settings()?["userAgent"]});
        merge(&mut headers, &match_headers(address, &rules));
        if let Some(origin) = origin(address) {
            let mut grants = self.grants.lock().expect("grants lock");
            grants.retain(|_, v| now().saturating_sub(v.0) < 1_800_000);
            if let Some((_, grant)) = grants.get(&origin) {
                merge(&mut headers, grant);
            }
        }
        merge(&mut headers, &platform_headers(address));
        Ok(headers)
    }
    pub async fn inspect(&self, address: &str, cancel: &CancellationToken) -> Result<Value> {
        let timeout =
            number(&self.catalogue.sources.store.settings()?["timeout"], 20.0).clamp(1.0, 8.0);
        self.inspect_timeout(address, Duration::from_secs_f64(timeout), cancel)
            .await
    }
    pub(crate) async fn inspect_timeout(
        &self,
        address: &str,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        let started = Instant::now();
        let mut report = identify(address, "", &[]);
        merge(
            &mut report,
            &json!({
                "ok":false, "http":{"status":null,"contentType":"","redirected":false},
                "sample":{"bytes":0,"limit":SAMPLE_LIMIT,"truncated":false},
                "errorCategory":null,"error":null
            }),
        );
        let invalid = match url::Url::parse(address) {
            Err(_) => Some(("invalid_address", "媒体地址无效")),
            Ok(url) if !matches!(url.scheme(), "http" | "https") => {
                Some(("unsupported_protocol", "浅探测仅支持 HTTP/HTTPS"))
            }
            _ => None,
        };
        let failure = if let Some(invalid) = invalid {
            Some(invalid)
        } else {
            let operation = async {
                let range = format!("bytes=0-{}", SAMPLE_LIMIT - 1);
                let response = self
                    .upstream_timeout(address, Some(&range), timeout)
                    .await?;
                let status = response.status();
                let final_url = response.url().to_string();
                let content_type = response
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_owned();
                let length = response.content_length();
                report["http"] = json!({
                    "status":status.as_u16(),"contentType":normalized_mime(&content_type),
                    "redirected":url::Url::parse(address).is_ok_and(|v| v.as_str() != final_url)
                });
                if !status.is_success() {
                    merge(&mut report, &identify(&final_url, &content_type, &[]));
                    report["errorCategory"] = json!("http_status");
                    report["error"] = json!(format!("HTTP {}", status.as_u16()));
                    return Ok(());
                }
                let mut bytes = Vec::with_capacity(SAMPLE_LIMIT);
                let mut stopped_early = false;
                let mut stream = response.bytes_stream();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk?;
                    let count = chunk.len().min(SAMPLE_LIMIT - bytes.len());
                    bytes.extend_from_slice(&chunk[..count]);
                    report["sample"]["bytes"] = json!(bytes.len());
                    let found = identify(&final_url, &content_type, &bytes);
                    if bytes.len() == SAMPLE_LIMIT
                        || (found["confidence"] == "signature"
                            && found["kind"] != "hls"
                            && found["kind"] != "dash")
                    {
                        stopped_early = true;
                        break;
                    }
                }
                report["sample"]["truncated"] =
                    json!(stopped_early && length.is_none_or(|v| v > bytes.len() as u64));
                merge(&mut report, &identify(&final_url, &content_type, &bytes));
                let (category, error) = if report["kind"] == "html" {
                    (
                        Some("non_media"),
                        Some("返回的是 HTML 页面，可能是登录页或访问拦截"),
                    )
                } else if bytes.is_empty()
                    || report["kind"] == "unknown"
                    || report["confidence"] == "url"
                {
                    (Some("unknown_media"), Some("未取得足够的媒体内容证据"))
                } else {
                    (None, None)
                };
                report["ok"] = json!(category.is_none());
                report["errorCategory"] = json!(category);
                report["error"] = json!(error);
                Ok::<_, anyhow::Error>(())
            };
            tokio::select! { biased;
                _ = cancel.cancelled() => Some(("cancelled", "媒体诊断已取消")),
                outcome = tokio::time::timeout(timeout, operation) => match outcome {
                    Err(_) => Some(("timeout", "媒体诊断超时")),
                    Ok(Err(error)) => {
                        if error.chain().any(|v| v.downcast_ref::<reqwest::Error>().is_some_and(reqwest::Error::is_timeout)) {
                            Some(("timeout", "媒体诊断超时"))
                        } else {
                            Some(("network", "媒体网络请求或读取失败"))
                        }
                    }
                    Ok(Ok(())) => None,
                }
            }
        };
        if let Some((category, error)) = failure {
            report["ok"] = json!(false);
            report["errorCategory"] = json!(category);
            report["error"] = json!(error);
        }
        report["ms"] = json!(started.elapsed().as_millis() as u64);
        Ok(report)
    }
    pub fn parses(&self) -> Result<Vec<Value>> {
        let store = &self.catalogue.sources.store;
        let stats = store.read("parse-stats", json!({}))?;
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        let configs: Vec<_> = self
            .catalogue
            .sources
            .configs()?
            .iter()
            .flat_map(|v| array(&v["parses"]))
            .collect();
        for (list, kind) in [
            (configs, "config"),
            (array(&store.read("parses", json!([]))?), "custom"),
        ] {
            for mut item in list {
                let address = s(&item, "url");
                if address.is_empty() || !seen.insert(address.clone()) {
                    continue;
                }
                item["url"] = json!(address);
                if s(&item, "name").is_empty() {
                    item["name"] = item["url"].clone();
                }
                item["type"] = json!(number(&item["type"], 1.0));
                item["origin"] = json!(kind);
                let flags = if let Some(value) = item["ext"]["flag"].as_str() {
                    value
                        .split([',', '|'])
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                        .map(|v| json!(v))
                        .collect::<Vec<_>>()
                } else {
                    array(&item["ext"]["flag"])
                };
                item["flags"] = json!(flags);
                item["stat"] = stats
                    .get(&address)
                    .cloned()
                    .unwrap_or_else(|| json!({"ok":0,"fail":0,"lastOk":0,"lastMs":0}));
                out.push(item);
            }
        }
        Ok(out)
    }
    pub fn add_parse(&self, input: &Value) -> Result<Vec<Value>> {
        let address = s(input, "url");
        if origin(&address).is_none() {
            bail!("解析接口地址要以 http(s):// 开头");
        }
        if self.parses()?.iter().any(|v| s(v, "url") == address) {
            bail!("这条解析接口已经有了");
        }
        self.catalogue.sources.store.update("parses",json!([]),|all|{all.as_array_mut().context("解析接口数据格式无效")?.push(json!({"name":if s(input,"name").is_empty(){address.clone()}else{s(input,"name")},"type":number(&input["type"],1.0),"url":address,"ext":{"flag":array(&input["flags"])}}));Ok(())})?;
        self.parses()
    }
    pub fn remove_parse(&self, address: &str) -> Result<Vec<Value>> {
        self.catalogue
            .sources
            .store
            .update("parses", json!([]), |all| {
                all.as_array_mut()
                    .context("解析接口数据格式无效")?
                    .retain(|v| s(v, "url") != address.trim());
                Ok(())
            })?;
        self.parses()
    }
    fn record_parse(&self, address: &str, attempt: &Value) -> Result<()> {
        self.catalogue
            .sources
            .store
            .update("parse-stats", json!({}), |stats| {
                if !stats[address].is_object() {
                    stats[address] = json!({"ok":0,"fail":0,"lastOk":0,"lastMs":0});
                }
                let item = &mut stats[address];
                let key = if attempt["ok"] == true { "ok" } else { "fail" };
                item[key] = json!(number(&item[key], 0.0) + 1.0);
                if key == "ok" {
                    item["lastOk"] = json!(now());
                    item["lastMs"] = attempt["ms"].clone();
                } else {
                    item["lastError"] = attempt["error"].clone();
                }
                Ok(())
            })
    }
    async fn walk(
        &self,
        start: &str,
        headers: Value,
        label: &str,
        strict: bool,
        cancel: &CancellationToken,
    ) -> Value {
        let started = Instant::now();
        let mut address = start.to_owned();
        let result = async {
            for _ in 0..3 {
                let response = self
                    .catalogue
                    .sources
                    .net
                    .get(&address, &headers, 8.0, cancel)
                    .await?;
                if response.url != address && is_direct(&response.url) {
                    return Ok(response.url);
                }
                let body = response.text();
                if let Some(found) = extract_url(&body, strict)
                    && found != address
                {
                    return Ok(found);
                }
                if let Some(next) = redirect(&body, &response.url) {
                    address = next;
                } else {
                    break;
                }
            }
            bail!("没有拿到可识别的播放地址")
        }
        .await;
        match result {
            Ok::<String, anyhow::Error>(url) => {
                json!({"name":label,"ok":true,"ms":started.elapsed().as_millis() as u64,"url":url})
            }
            Err(error) => {
                json!({"name":label,"ok":false,"ms":started.elapsed().as_millis() as u64,"error":format!("{error:#}")})
            }
        }
    }
    async fn try_parse(
        &self,
        item: &Value,
        target: &str,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        let address = parse_address(&s(item, "url"), target)?;
        let mut header = item["ext"]["header"].clone();
        if !header.is_object() {
            header = json!({});
        }
        if s(&header, "Referer").is_empty() {
            header["Referer"] = item["url"].clone();
        }
        Ok(self
            .walk(&address, header, &s(item, "name"), false, cancel)
            .await)
    }
    pub async fn test_parse(&self, address: &str, target: &str) -> Result<Value> {
        let item = self
            .parses()?
            .into_iter()
            .find(|v| s(v, "url") == address)
            .context("没有这条解析接口")?;
        let target = if target.trim().is_empty() {
            SAMPLE_TARGET
        } else {
            target
        };
        let mut result = self
            .try_parse(&item, target, &CancellationToken::new())
            .await?;
        self.record_parse(address, &result)?;
        result["target"] = json!(target);
        Ok(result)
    }
    fn resolve_headers(&self, address: &str, options: &Value) -> Result<Value> {
        let mut headers = self.headers(address)?;
        let key = s(options, "siteKey");
        if !key.is_empty() {
            let site = self.catalogue.sources.site(&key)?;
            merge(&mut headers, &site["header"]);
        }
        Ok(headers)
    }
    pub async fn resolve(
        &self,
        raw: &str,
        options: &Value,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        let result = self.resolve_inner(raw, options, cancel).await?;
        if result["ok"] == true {
            let address = s(&result, "url");
            let key = s(options, "siteKey");
            // Website adapters already scope credentials to the media destination.
            let mut headers = if !key.is_empty()
                && crate::websites::supported(&self.catalogue.sources.site(&key)?)
            {
                json!({})
            } else {
                self.resolve_headers(&address, options)?
            };
            merge(&mut headers, &result["header"]);
            self.register(&address, &headers);
        }
        Ok(result)
    }
    async fn resolve_inner(
        &self,
        raw: &str,
        options: &Value,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        let mut address = raw.trim().to_owned();
        if address.is_empty() {
            bail!("没有播放地址");
        }
        if cancel.is_cancelled() {
            bail!("请求已取消");
        }
        let started = Instant::now();
        if address.starts_with("spider://") {
            let (key, flag, id) = spider::parse_url(&address).context("Spider 播放地址格式不对")?;
            let site = self.catalogue.sources.site(&key)?;
            let payload = self
                .catalogue
                .spider
                .call(
                    &site,
                    "player",
                    json!({"flag":flag,"id":id,"vipFlags":[]}),
                    60.0,
                    cancel,
                )
                .await?;
            address = s(&payload, "url");
            if address.is_empty() {
                bail!("Spider 没有返回播放地址");
            }
            if number(&payload["parse"], 0.0) != 1.0 {
                self.register(&address, &payload["header"]);
                return Ok(
                    json!({"ok":true,"url":address,"direct":false,"parseName":"Spider","header":payload["header"],"ms":started.elapsed().as_millis() as u64,"attempts":[]}),
                );
            }
        }
        let key = s(options, "siteKey");
        if !key.is_empty() {
            let site = self.catalogue.sources.site(&key)?;
            if crate::websites::supported(&site) {
                let resolved =
                    crate::websites::resolve(&self.catalogue.sources, &site, &address, cancel)
                        .await?;
                let address = s(&resolved, "url");
                if !crate::http::is_http_url(&address) {
                    bail!("网站没有返回可用的媒体地址");
                }
                return Ok(
                    json!({"ok":true,"url":address,"direct":false,"parseName":site["name"],"header":resolved["header"],"ms":started.elapsed().as_millis() as u64,"attempts":[]}),
                );
            }
        }
        if is_direct(&address) {
            return Ok(json!({"ok":true,"url":address,"direct":true,"ms":0,"attempts":[]}));
        }
        let mut headers = self.resolve_headers(&address, options)?;
        if !header_map(&headers)?.contains_key(header::REFERER) {
            headers["Referer"] = json!(
                origin(&address)
                    .map(|v| format!("{v}/"))
                    .unwrap_or_default()
            );
        }
        self.register(&address, &headers);
        let probe = self.inspect(&address, cancel).await?;
        if cancel.is_cancelled() {
            bail!("请求已取消");
        }
        if probe["ok"] == true {
            return Ok(
                json!({"ok":true,"url":address,"direct":true,"ms":started.elapsed().as_millis() as u64,"attempts":[]}),
            );
        }
        let first = self
            .walk(&address, headers, "直接从播放页里取", true, cancel)
            .await;
        if cancel.is_cancelled() {
            bail!("请求已取消");
        }
        let mut attempts = vec![first.clone()];
        if first["ok"] == true {
            return Ok(
                json!({"ok":true,"url":first["url"],"direct":false,"parseName":first["name"],"ms":started.elapsed().as_millis() as u64,"attempts":attempts}),
            );
        }
        let mut parsers = self.parses()?;
        let prefer = s(options, "prefer");
        let line = s(options, "flag").to_lowercase();
        let tier = |v: &Value| -> u8 {
            if !prefer.is_empty() && (s(v, "name") == prefer || s(v, "url") == prefer) {
                0
            } else if !line.is_empty()
                && array(&v["flags"])
                    .iter()
                    .any(|v| crate::model::text(v).to_lowercase() == line)
            {
                1
            } else if array(&v["flags"]).is_empty() {
                2
            } else {
                3
            }
        };
        let score = |v: &Value| {
            let stat = &v["stat"];
            let ok = number(&stat["ok"], 0.0);
            let fail = number(&stat["fail"], 0.0);
            let ms = number(&stat["lastMs"], 0.0);
            (ok + 1.0) / (ok + fail + 2.0) * 2.0
                + if ms > 0.0 {
                    (1.0 - ms.min(8000.0) / 8000.0).max(0.0)
                } else {
                    0.5
                }
        };
        parsers.sort_by(|a, b| {
            tier(a)
                .cmp(&tier(b))
                .then_with(|| {
                    (number(&a["type"], 1.0) == 0.0).cmp(&(number(&b["type"], 1.0) == 0.0))
                })
                .then_with(|| score(b).total_cmp(&score(a)))
        });
        for item in parsers {
            if cancel.is_cancelled() {
                bail!("请求已取消");
            }
            if started.elapsed() > Duration::from_secs(24) {
                attempts.push(
                    json!({"name":item["name"],"ok":false,"ms":0,"error":"整体超时，没轮到这条"}),
                );
                continue;
            }
            let attempt = self.try_parse(&item, &address, cancel).await?;
            if cancel.is_cancelled() {
                bail!("请求已取消");
            }
            self.record_parse(&s(&item, "url"), &attempt)?;
            attempts.push(attempt.clone());
            if attempt["ok"] == true {
                return Ok(
                    json!({"ok":true,"url":attempt["url"],"direct":false,"parseName":attempt["name"],"ms":started.elapsed().as_millis() as u64,"attempts":attempts}),
                );
            }
        }
        let reason = attempts
            .iter()
            .find_map(|v| v["error"].as_str())
            .unwrap_or("未知原因");
        Ok(
            json!({"ok":false,"url":"","direct":false,"ms":started.elapsed().as_millis() as u64,"error":format!("{} 条路子都没能换出地址（{reason}）",attempts.len()),"attempts":attempts}),
        )
    }
    pub async fn start_proxy(self: &Arc<Self>) -> Result<MediaProxy> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!(
            "http://{}/{}/media",
            listener.local_addr()?,
            uuid::Uuid::new_v4()
        );
        let path = url::Url::parse(&base)?.path().to_owned();
        let state = Arc::new(ProxyState {
            media: self.clone(),
            base: base.clone(),
        });
        let app = Router::new()
            .route(&path, get(proxy_request).head(proxy_request))
            .with_state(state);
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        let task = tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, app)
                .with_graceful_shutdown(stop.cancelled_owned())
                .await
            {
                eprintln!("[media] 代理退出失败：{error}");
            }
        });
        Ok(MediaProxy { base, cancel, task })
    }
    async fn upstream(&self, address: &str, range: Option<&str>) -> Result<reqwest::Response> {
        self.upstream_timeout(address, range, Duration::from_secs(120))
            .await
    }
    async fn upstream_timeout(
        &self,
        address: &str,
        range: Option<&str>,
        timeout: Duration,
    ) -> Result<reqwest::Response> {
        let mut address = url::Url::parse(address).context("媒体地址无效")?;
        for _ in 0..10 {
            if !matches!(address.scheme(), "http" | "https") {
                bail!("媒体代理仅支持 HTTP/HTTPS");
            }
            let mut headers = header_map(&self.headers(address.as_str())?)?;
            if let Some(range) = range {
                headers.insert(header::RANGE, range.parse()?);
            }
            let response = self
                .client
                .get(address.clone())
                .headers(headers)
                .timeout(timeout)
                .send()
                .await?;
            if response.status().is_redirection() {
                let target = response
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .context("媒体跳转缺少目标地址")?;
                address = address.join(target)?;
                continue;
            }
            return Ok(response);
        }
        bail!("媒体重定向次数过多")
    }
}

pub struct MediaProxy {
    pub base: String,
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}
impl MediaProxy {
    pub fn url(&self, address: &str) -> String {
        proxy_url(&self.base, address)
    }
}
impl Drop for MediaProxy {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}
struct ProxyState {
    media: Arc<Media>,
    base: String,
}
async fn proxy_request(
    State(state): State<Arc<ProxyState>>,
    Query(query): Query<HashMap<String, String>>,
    method: axum::http::Method,
    headers: HeaderMap,
) -> Response {
    let result = async {
        let address = query.get("url").context("缺少媒体地址")?;
        let response = state
            .media
            .upstream(
                address,
                headers.get(header::RANGE).and_then(|v| v.to_str().ok()),
            )
            .await?;
        let status = response.status();
        let final_url = response.url().to_string();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_owned();
        let detected = identify(&final_url, &content_type, &[]);
        let mut playlist = detected["kind"] == "hls";
        let length = response.content_length();
        let mut out = Response::builder()
            .status(status)
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(
                header::ACCESS_CONTROL_EXPOSE_HEADERS,
                "Content-Length,Content-Range,Accept-Ranges,X-Crayon-Media-Url",
            )
            .header("X-Crayon-Media-Url", spider::encode(&final_url))
            .header(header::CACHE_CONTROL, "no-store");
        for key in [header::ACCEPT_RANGES, header::CONTENT_RANGE] {
            if let Some(value) = response.headers().get(&key) {
                out = out.header(key, value);
            }
        }
        if method == axum::http::Method::HEAD {
            if !playlist && let Some(length) = length {
                out = out.header(header::CONTENT_LENGTH, length);
            }
            return Ok(out
                .header(header::CONTENT_TYPE, content_type)
                .body(Body::empty())?);
        }
        let mut chunks = response.bytes_stream();
        let mut prefix = Vec::new();
        if detected["kind"] == "unknown" && status.is_success() {
            let mut sample = Vec::with_capacity(SAMPLE_LIMIT);
            while let Some(chunk) = chunks.next().await {
                let chunk = chunk?;
                let count = chunk.len().min(SAMPLE_LIMIT - sample.len());
                sample.extend_from_slice(&chunk[..count]);
                prefix.push(chunk);
                let found = identify(&final_url, &content_type, &sample);
                if found["kind"] == "hls" && found["confidence"] == "signature" {
                    playlist = true;
                    break;
                }
                let candidate = sample.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&sample);
                let candidate = candidate.trim_ascii_start();
                if sample.len() == SAMPLE_LIMIT
                    || (!b"#EXTM3U".starts_with(candidate)
                        && ![0xef, 0xbb, 0xbf].starts_with(&sample))
                {
                    break;
                }
            }
        }
        let mut chunks =
            stream::iter(prefix.into_iter().map(Ok::<_, reqwest::Error>)).chain(chunks);
        if playlist && status.is_success() {
            let mut bytes = Vec::new();
            while let Some(chunk) = chunks.next().await {
                let chunk = chunk?;
                if bytes.len() + chunk.len() > 5 * 1024 * 1024 {
                    bail!("播放列表超过 5MB 上限");
                }
                bytes.extend_from_slice(&chunk);
            }
            let body = rewrite_playlist(&String::from_utf8_lossy(&bytes), &final_url, &state.base);
            return Ok(out
                .header(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")
                .body(Body::from(body))?);
        }
        if let Some(length) = length {
            out = out.header(header::CONTENT_LENGTH, length);
        }
        Ok::<_, anyhow::Error>(
            out.header(header::CONTENT_TYPE, content_type)
                .body(Body::from_stream(chunks))?,
        )
    }
    .await;
    match result {
        Ok(response) => response,
        Err(error) => (StatusCode::BAD_GATEWAY, format!("媒体请求失败：{error:#}")).into_response(),
    }
}
pub fn proxy_url(base: &str, address: &str) -> String {
    format!("{base}?url={}", spider::encode(address))
}
pub fn rewrite_playlist(input: &str, address: &str, base: &str) -> String {
    static URI: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"URI="([^"]+)""#).expect("constant regex"));
    input
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                line.to_owned()
            } else if trimmed.starts_with('#') {
                URI.replace_all(line, |capture: &regex::Captures<'_>| {
                    let target = crate::html_rules::absolutize(&capture[1], address);
                    if crate::http::is_http_url(&target) {
                        format!("URI=\"{}\"", proxy_url(base, &target))
                    } else {
                        capture[0].to_owned()
                    }
                })
                .to_string()
            } else {
                let target = crate::html_rules::absolutize(trimmed, address);
                if crate::http::is_http_url(&target) {
                    proxy_url(base, &target)
                } else {
                    line.to_owned()
                }
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn origin(address: &str) -> Option<String> {
    let url = url::Url::parse(address).ok()?;
    if matches!(url.scheme(), "http" | "https") {
        Some(url.origin().ascii_serialization())
    } else {
        None
    }
}
pub fn platform_headers(address: &str) -> Value {
    static RULES: LazyLock<Vec<(Regex, &str, &str)>> = LazyLock::new(|| {
        [(r"(?i)huya\.com","https://www.huya.com/","https://www.huya.com"),(r"(?i)(douyin|douyincdn|bytefcdn|volccdn|pstatp|ixigua|bytedance|byteimg|snssdk)\.com","https://live.douyin.com/","https://live.douyin.com")].into_iter().map(|(pattern,referer,origin)|(Regex::new(pattern).expect("constant regex"),referer,origin)).collect()
    });
    if let Some((_, referer, origin)) = RULES
        .iter()
        .find(|(pattern, _, _)| pattern.is_match(address))
    {
        let mut out = json!({"Referer":referer});
        if url::Url::parse(address).is_ok_and(|url| url.scheme() == "https") {
            out["Origin"] = json!(origin);
        }
        out
    } else {
        json!({})
    }
}
