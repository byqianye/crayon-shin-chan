use crate::{
    html_rules::{absolutize, extract, select},
    http::{RawResponse, header_map},
    media_probe::{SAMPLE_LIMIT, identify},
    model::{merge, number, s, text},
    sources::Sources,
};
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use regex::Regex;
use reqwest::Method;
use scraper::{ElementRef, Html};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use url::Url;

const ALPHABET: &str = "PXhw7UT1B0a9kQDKZsjIASmOezxYG4CHo5Jyfg2b8FLpEvRr3WtVnlqMidu6cN";
const PAGE_LIMIT: usize = 2 * 1024 * 1024;
const CACHE_LIMIT: usize = 8 * 1024 * 1024;
const CACHE_TTL: Duration = Duration::from_secs(20);

#[derive(Default)]
struct Session {
    cookies: BTreeMap<String, String>,
    pages: VecDeque<(String, Instant, String)>,
}
type SharedSession = Arc<tokio::sync::Mutex<Session>>;
static SESSIONS: LazyLock<Mutex<HashMap<String, SharedSession>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn base(site: &Value) -> Result<Url> {
    let mut root = Url::parse(&s(site, "api")).context("看看鸭站点地址无效")?;
    if !matches!(root.scheme(), "http" | "https")
        || !root.username().is_empty()
        || root.password().is_some()
    {
        bail!("看看鸭站点必须使用无凭据的 HTTP 或 HTTPS 地址");
    }
    root.set_path("/");
    root.set_query(None);
    root.set_fragment(None);
    Ok(root)
}
fn timeout(sources: &Sources, site: &Value) -> Result<Duration> {
    Ok(Duration::from_secs_f64(
        number(
            &site["timeout"],
            number(&sources.store.settings()?["timeout"], 20.0),
        )
        .clamp(1.0, 300.0),
    ))
}
fn headers(sources: &Sources, site: &Value, address: &str) -> Result<Value> {
    let mut out = json!({"Referer":base(site)?.as_str()});
    merge(&mut out, &sources.headers(site, address)?);
    Ok(out)
}
fn remove_header(header: &mut Value, name: &str) {
    if let Some(map) = header.as_object_mut() {
        map.retain(|key, _| !key.eq_ignore_ascii_case(name));
    }
}
fn session(root: &Url, header: &Value) -> Result<SharedSession> {
    let key = format!("{}|{header}", root.origin().ascii_serialization());
    let mut sessions = SESSIONS
        .lock()
        .map_err(|_| anyhow::anyhow!("看看鸭会话锁不可用"))?;
    if let Some(session) = sessions.get(&key) {
        return Ok(session.clone());
    }
    if sessions.len() >= 16 {
        let stale = sessions
            .iter()
            .find(|(_, value)| Arc::strong_count(value) == 1)
            .map(|(key, _)| key.clone());
        if let Some(stale) = stale {
            sessions.remove(&stale);
        } else {
            bail!("看看鸭并发会话超过上限");
        }
    }
    let next = Arc::new(tokio::sync::Mutex::new(Session::default()));
    sessions.insert(key, next.clone());
    Ok(next)
}
fn cookie_headers(header: &Value, session: &Session) -> Value {
    let mut header = header.clone();
    let configured = header
        .as_object()
        .and_then(|values| {
            values
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("cookie"))
        })
        .map(|(_, value)| text(value))
        .unwrap_or_default();
    let mut cookies = session
        .cookies
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>();
    if !configured.is_empty() {
        cookies.insert(0, configured);
    }
    remove_header(&mut header, "cookie");
    if !cookies.is_empty() {
        header["Cookie"] = json!(cookies.join("; "));
    }
    header
}
fn save_cookies(session: &mut Session, response: &RawResponse) {
    for value in response.headers.get_all(reqwest::header::SET_COOKIE) {
        let Ok(value) = value.to_str() else { continue };
        let Some(pair) = value.split(';').next() else {
            continue;
        };
        let Some((name, value)) = pair.split_once('=') else {
            continue;
        };
        if name.is_empty() || pair.len() > 4096 || session.cookies.len() >= 32 {
            continue;
        }
        if value.is_empty() {
            session.cookies.remove(name.trim());
        } else {
            session
                .cookies
                .insert(name.trim().to_owned(), value.trim().to_owned());
        }
    }
}
async fn fetch_raw(
    sources: &Sources,
    site: &Value,
    address: &str,
    method: Method,
    header: &Value,
    body: Option<String>,
    cancel: &CancellationToken,
) -> Result<RawResponse> {
    let budget = timeout(sources, site)?;
    let operation = async {
        let mut request = sources
            .net
            .client
            .request(method, address)
            .headers(header_map(header)?)
            .timeout(budget);
        if let Some(body) = body {
            request = request.body(body);
        }
        let response = request.send().await.context("看看鸭网络请求失败")?;
        let status = response.status().as_u16();
        let address = response.url().clone();
        if address.origin() != base(site)?.origin() {
            bail!("看看鸭页面重定向到其他站点");
        }
        if !(200..300).contains(&status) {
            bail!("看看鸭返回 HTTP {status}");
        }
        let headers = response.headers().clone();
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("读取看看鸭响应失败")?;
            if bytes.len().saturating_add(chunk.len()) > PAGE_LIMIT {
                bail!("看看鸭页面超过 2MB 上限");
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(RawResponse {
            status,
            url: address.to_string(),
            headers,
            bytes,
        })
    };
    tokio::select! { biased; _=cancel.cancelled()=>bail!("请求已取消"),
    result=tokio::time::timeout(budget,operation)=>result.context("看看鸭请求超时")? }
}
fn encrypt_gate(input: &str) -> String {
    let alphabet: Vec<char> = ALPHABET.chars().collect();
    let mut out = String::new();
    for c in input.chars() {
        out.push('P');
        out.push(
            alphabet
                .iter()
                .position(|v| *v == c)
                .map(|i| alphabet[(i + 3) % 62])
                .unwrap_or(c),
        );
        out.push('P');
    }
    STANDARD.encode(out.as_bytes())
}
fn gate_token(input: &str) -> Result<Option<String>> {
    if !input.contains("robot.php") {
        return Ok(None);
    }
    static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"var\s+token\s*=\s*encrypt\(\s*["']([^"']+)["']\s*\)"#)
            .expect("constant regex")
    });
    let token = TOKEN
        .captures(input)
        .and_then(|v| v.get(1))
        .context("看看鸭公开检查页面结构已变化")?;
    Ok(Some(token.as_str().to_owned()))
}
async fn page(
    sources: &Sources,
    site: &Value,
    address: &str,
    cancel: &CancellationToken,
) -> Result<String> {
    let root = base(site)?;
    if Url::parse(address)?.origin() != root.origin() {
        bail!("看看鸭请求地址必须属于当前站点");
    }
    let header = headers(sources, site, address)?;
    let shared = session(&root, &header)?;
    let mut session = tokio::select! { biased; _=cancel.cancelled()=>bail!("请求已取消"), guard=shared.lock()=>guard };
    session
        .pages
        .retain(|(_, time, _)| time.elapsed() < CACHE_TTL);
    if let Some((_, _, body)) = session.pages.iter().find(|(key, _, _)| key == address) {
        return Ok(body.clone());
    }
    let response = fetch_raw(
        sources,
        site,
        address,
        Method::GET,
        &cookie_headers(&header, &session),
        None,
        cancel,
    )
    .await?;
    save_cookies(&mut session, &response);
    let mut input = response.text();
    if let Some(token) = gate_token(&input)? {
        let endpoint = root.join("robot.php")?;
        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("value", &encrypt_gate(&response.url))
            .append_pair("token", &encrypt_gate(&token))
            .finish();
        let mut check_header = cookie_headers(&header, &session);
        check_header["Content-Type"] = json!("application/x-www-form-urlencoded");
        check_header["Referer"] = json!(response.url);
        let verified = fetch_raw(
            sources,
            site,
            endpoint.as_str(),
            Method::POST,
            &check_header,
            Some(body),
            cancel,
        )
        .await?;
        let result: Value =
            serde_json::from_str(&verified.text()).context("看看鸭公开检查未返回有效 JSON")?;
        if result["msg"] != "ok" {
            bail!("看看鸭公开检查失败");
        }
        save_cookies(&mut session, &verified);
        let response = fetch_raw(
            sources,
            site,
            address,
            Method::GET,
            &cookie_headers(&header, &session),
            None,
            cancel,
        )
        .await?;
        save_cookies(&mut session, &response);
        input = response.text();
        if gate_token(&input)?.is_some() {
            bail!("看看鸭公开检查后仍未返回影视页面");
        }
    }
    while session.pages.len() >= 24
        || session
            .pages
            .iter()
            .map(|(_, _, body)| body.len())
            .sum::<usize>()
            + input.len()
            > CACHE_LIMIT
    {
        if session.pages.pop_front().is_none() {
            break;
        }
    }
    session
        .pages
        .push_back((address.to_owned(), Instant::now(), input.clone()));
    Ok(input)
}
fn route_id(address: &str, prefix: &str) -> Option<String> {
    let url = Url::parse(address).ok()?;
    let id = url.path().strip_prefix(prefix)?.strip_suffix(".html")?;
    (!id.is_empty() && id.bytes().all(|v| v.is_ascii_digit())).then(|| id.to_owned())
}
fn node_text(node: ElementRef<'_>) -> String {
    node.text()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn classes(input: &str, address: &str) -> Vec<Value> {
    let html = Html::parse_document(input);
    let origin = Url::parse(address).ok().map(|v| v.origin());
    let mut seen = HashSet::new();
    select(html.root_element(), ".navbar-item a[href]")
        .into_iter()
        .filter_map(|node| {
            let link = absolutize(node.value().attr("href")?, address);
            if Url::parse(&link).ok().map(|v| v.origin()) != origin {
                return None;
            }
            let id = route_id(&link, "/type/")?;
            if !seen.insert(id.clone()) {
                return None;
            }
            let name = node_text(node);
            (!name.is_empty()).then(|| json!({"type_id":id,"type_name":name,"type_pid":"0"}))
        })
        .collect()
}
fn items(input: &str, address: &str) -> Vec<Value> {
    let html = Html::parse_document(input);
    let origin = Url::parse(address).ok().map(|v| v.origin());
    let mut seen = HashSet::new();
    select(html.root_element(), ".module-poster-item, .module-card-item").into_iter().filter_map(|node| {
        let link = node.value().attr("href").map(str::to_owned).or_else(|| select(node, "a[href]").into_iter().find_map(|node| {
            let raw = node.value().attr("href")?; route_id(&absolutize(raw, address), "/detail/").map(|_| raw.to_owned())
        }))?;
        let link = absolutize(&link, address); route_id(&link, "/detail/")?;
        if Url::parse(&link).ok().map(|v| v.origin()) != origin || !seen.insert(link.clone()) { return None; }
        let name = node.value().attr("title").filter(|v| !v.trim().is_empty()).map(str::to_owned)
            .unwrap_or_else(|| extract(node, "img&&alt", address, &["alt"]));
        let name = if name.is_empty() { extract(node, ".module-poster-item-title", address, &["Text"]) } else { name };
        if name.is_empty() { return None; }
        Some(json!({"vod_id":link,"vod_name":name,"vod_pic":absolutize(&extract(node,"img",address,&["data-original","data-src","src"]),address),
            "vod_remarks":extract(node,".module-item-note",address,&["Text"])}))
    }).take(120).collect()
}
fn pagecount(input: &str, current: usize) -> usize {
    static PAGE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"/show/\d+--------(\d+)---\.html").expect("constant regex"));
    let html = Html::parse_document(input);
    select(html.root_element(), ".page-link[href], .page-number[href]")
        .into_iter()
        .filter_map(|node| node.value().attr("href"))
        .filter_map(|href| PAGE.captures(href).and_then(|v| v[1].parse::<usize>().ok()))
        .max()
        .unwrap_or(current)
        .max(current)
}
fn detail(input: &str, address: &str) -> Result<Value> {
    let html = Html::parse_document(input);
    let root = html.root_element();
    let origin = Url::parse(address)?.origin();
    let name = extract(root, ".module-info-heading h1", address, &["Text"]);
    if name.is_empty() {
        bail!("看看鸭详情页缺少名称，页面结构可能已变化");
    }
    let tabs = select(root, ".module-tab-item[data-dropdown-value]")
        .into_iter()
        .map(|node| {
            node.value()
                .attr("data-dropdown-value")
                .unwrap_or("")
                .to_owned()
        })
        .collect::<Vec<_>>();
    let (mut from, mut groups) = (Vec::new(), Vec::new());
    for (index, group) in select(root, ".module-play-list-content")
        .into_iter()
        .enumerate()
    {
        let mut seen = HashSet::new();
        let episodes: Vec<_> = select(group, "a.module-play-list-link[href]")
            .into_iter()
            .filter_map(|node| {
                let link = absolutize(node.value().attr("href")?, address);
                let target = Url::parse(&link).ok()?;
                if target.origin() != origin
                    || !target.path().starts_with("/play/")
                    || !seen.insert(link.clone())
                {
                    return None;
                }
                let name = node_text(node).replace(['$', '#'], " ");
                Some(format!(
                    "{}{}{}",
                    if name.is_empty() {
                        format!("第{}集", seen.len())
                    } else {
                        name
                    },
                    "$",
                    link
                ))
            })
            .collect();
        if episodes.is_empty() {
            continue;
        }
        from.push(
            tabs.get(index)
                .filter(|v| !v.is_empty())
                .cloned()
                .unwrap_or_else(|| format!("线路{}", index + 1))
                .replace(['$', '#'], " "),
        );
        groups.push(episodes.join("#"));
    }
    if groups.is_empty() {
        bail!("看看鸭详情页没有可播放分集，页面结构可能已变化");
    }
    let tags = select(root, ".module-info-tag-link a")
        .into_iter()
        .map(node_text)
        .collect::<Vec<_>>();
    let mut out = json!({"vod_id":address,"vod_name":name,"vod_pic":absolutize(&extract(root,".module-info-poster img",address,&["data-original","data-src","src"]),address),
        "vod_content":extract(root,".module-info-introduction-content",address,&["Text"]),"vod_year":tags.first().cloned().unwrap_or_default(),
        "vod_area":tags.get(1).cloned().unwrap_or_default(),"vod_class":tags.iter().skip(2).cloned().collect::<Vec<_>>().join(","),
        "vod_play_from":from.join("$$$"),"vod_play_url":groups.join("$$$")});
    for node in select(root, ".module-info-item") {
        let title = extract(node, ".module-info-item-title", address, &["Text"]);
        let value = extract(node, ".module-info-item-content", address, &["Text"]);
        if title.starts_with("导演") {
            out["vod_director"] = json!(value);
        } else if title.starts_with("主演") {
            out["vod_actor"] = json!(value);
        } else if title.starts_with("集数") {
            out["vod_remarks"] = json!(value);
        }
    }
    Ok(out)
}
pub async fn request(
    sources: &Sources,
    site: &Value,
    query: &Value,
    cancel: &CancellationToken,
) -> Result<Value> {
    if cancel.is_cancelled() {
        bail!("请求已取消");
    }
    let root = base(site)?;
    if let Some(keyword) = query.get("wd") {
        let keyword = text(keyword).trim().to_owned();
        if keyword.is_empty() {
            return Ok(json!({"code":1,"list":[]}));
        }
        let mut address = root.join("search/-------------.html")?;
        address.query_pairs_mut().append_pair("wd", &keyword);
        let input = page(sources, site, address.as_str(), cancel).await?;
        let list = items(&input, address.as_str());
        if list.is_empty() {
            let html = Html::parse_document(&input);
            let root = html.root_element();
            let empty = select(root, ".nofound")
                .into_iter()
                .any(|node| node_text(node).contains("没有找到与"));
            if !empty || !select(root, ".module-card-item").is_empty() {
                bail!("看看鸭搜索未返回有效结果页面，页面结构可能已变化");
            }
        }
        return Ok(json!({"code":1,"page":1,"pagecount":1,"list":list}));
    }
    let id = s(query, "ids");
    if !id.is_empty() {
        let address = if id.bytes().all(|v| v.is_ascii_digit()) {
            root.join(&format!("detail/{id}.html"))?
        } else {
            root.join(&id)?
        };
        if address.origin() != root.origin() || route_id(address.as_str(), "/detail/").is_none() {
            bail!("看看鸭详情地址无效");
        }
        let input = page(sources, site, address.as_str(), cancel).await?;
        return Ok(json!({"code":1,"list":[detail(&input,address.as_str())?]}));
    }
    let class = s(query, "t");
    if !class.is_empty() && !class.bytes().all(|v| v.is_ascii_digit()) {
        bail!("看看鸭分类编号无效");
    }
    let current = number(&query["pg"], 1.0).clamp(1.0, 100000.0) as usize;
    let address = if class.is_empty() {
        root.clone()
    } else {
        root.join(&format!("show/{class}--------{current}---.html"))?
    };
    let input = page(sources, site, address.as_str(), cancel).await?;
    if s(query, "ac") == "list" {
        let classes = classes(&input, address.as_str());
        if classes.is_empty() {
            bail!("看看鸭页面没有分类，页面结构可能已变化");
        }
        return Ok(json!({"code":1,"class":classes}));
    }
    let list = items(&input, address.as_str());
    if list.is_empty() && current == 1 {
        bail!("看看鸭页面没有作品，页面结构可能已变化");
    }
    Ok(json!({"code":1,"page":current,"pagecount":pagecount(&input,current),"list":list}))
}
fn b64(input: &str) -> Result<Vec<u8>> {
    STANDARD
        .decode(input.trim())
        .context("看看鸭播放器 Base64 数据无效")
}
fn decode_media(encoded: &str, mode: u64) -> Result<String> {
    if mode == 0 {
        return Ok(encoded.to_owned());
    }
    let raw = b64(encoded)?;
    match mode {
        2 => {
            let value = String::from_utf8(raw).context("看看鸭模式2数据不是 UTF-8")?;
            let chars: Vec<_> = value.chars().collect();
            if chars.len() % 3 != 0 {
                bail!("看看鸭模式2数据长度无效");
            }
            let alphabet: Vec<_> = ALPHABET.chars().collect();
            Ok(chars
                .iter()
                .skip(1)
                .step_by(3)
                .map(|c| {
                    alphabet
                        .iter()
                        .position(|v| v == c)
                        .map(|i| alphabet[(i + 59) % 62])
                        .unwrap_or(*c)
                })
                .collect())
        }
        1 => {
            let key = format!("{:x}", md5::compute(b"test"));
            let raw: Vec<_> = raw
                .iter()
                .enumerate()
                .map(|(i, byte)| byte ^ key.as_bytes()[i % key.len()])
                .collect();
            let decoded = String::from_utf8(b64(
                std::str::from_utf8(&raw).context("看看鸭模式1数据无效")?
            )?)?;
            let mut parts = decoded.splitn(3, '/');
            let target: Vec<String> = serde_json::from_slice(&b64(parts
                .next()
                .context("看看鸭模式1缺少目标字母表")?)?)?;
            let lookup: Vec<String> = serde_json::from_slice(&b64(parts
                .next()
                .context("看看鸭模式1缺少查找字母表")?)?)?;
            if target.len() != 26
                || lookup.len() != 26
                || target
                    .iter()
                    .chain(&lookup)
                    .any(|v| v.len() != 1 || !v.as_bytes()[0].is_ascii_lowercase())
                || target.iter().collect::<HashSet<_>>().len() != 26
                || lookup.iter().collect::<HashSet<_>>().len() != 26
            {
                bail!("看看鸭模式1字母表无效");
            }
            let value =
                String::from_utf8(b64(parts.next().context("看看鸭模式1缺少媒体地址")?)?)?;
            Ok(value
                .chars()
                .map(|c| {
                    lookup
                        .iter()
                        .position(|v| v.starts_with(c))
                        .map(|i| target[i].chars().next().expect("validated alphabet"))
                        .unwrap_or(c)
                })
                .collect())
        }
        _ => bail!("看看鸭播放器 URL 编码模式 {mode} 尚不支持"),
    }
}
fn decode_player_url(player: &Value) -> Result<String> {
    let raw = s(player, "url");
    let raw = match player["encrypt"].as_u64().unwrap_or(0) {
        0 => return Ok(raw),
        1 => raw,
        2 => String::from_utf8(b64(&raw)?)?,
        mode => bail!("看看鸭播放页编码模式 {mode} 尚不支持"),
    };
    let bytes = raw.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let code = bytes
                .get(i + 1..i + 3)
                .context("看看鸭播放页 URL 转义不完整")?;
            out.push(
                u8::from_str_radix(std::str::from_utf8(code)?, 16)
                    .context("看看鸭播放页 URL 转义无效")?,
            );
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Ok(String::from_utf8(out)?)
}
pub async fn resolve(
    sources: &Sources,
    site: &Value,
    address: &str,
    cancel: &CancellationToken,
) -> Result<Value> {
    let root = base(site)?;
    let address = root.join(address)?;
    if address.origin() != root.origin() || !address.path().starts_with("/play/") {
        bail!("看看鸭播放页地址无效");
    }
    let input = page(sources, site, address.as_str(), cancel).await?;
    let player = {
        let html = Html::parse_document(&input);
        select(html.root_element(), "script")
            .into_iter()
            .find_map(|node| {
                let script = node.text().collect::<String>();
                let (_, tail) = script.split_once("player_aaaa")?;
                let (_, tail) = tail.split_once('=')?;
                serde_json::Deserializer::from_str(tail.trim())
                    .into_iter::<Value>()
                    .next()?
                    .ok()
            })
            .context("看看鸭播放页缺少有效 player_aaaa 配置")?
    };
    let vid = decode_player_url(&player)?;
    if vid.is_empty() {
        bail!("看看鸭播放页缺少视频标识");
    }
    let endpoint = root.join("yaplay/api.php")?;
    let mut header = headers(sources, site, address.as_str())?;
    let shared = session(&root, &header)?;
    let mut session = tokio::select! { biased; _=cancel.cancelled()=>bail!("请求已取消"),guard=shared.lock()=>guard };
    header = cookie_headers(&header, &session);
    header["Content-Type"] = json!("application/x-www-form-urlencoded");
    header["Referer"] = json!(address.as_str());
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("vid", &vid)
        .finish();
    let response = fetch_raw(
        sources,
        site,
        endpoint.as_str(),
        Method::POST,
        &header,
        Some(body),
        cancel,
    )
    .await?;
    save_cookies(&mut session, &response);
    let media_session = Session {
        cookies: session.cookies.clone(),
        ..Session::default()
    };
    drop(session);
    let payload: Value =
        serde_json::from_str(&response.text()).context("看看鸭播放器 API 未返回有效 JSON")?;
    if payload["code"] != 200 {
        bail!("看看鸭播放器 API 返回失败");
    }
    let mode = payload["data"]["urlmode"]
        .as_u64()
        .context("看看鸭播放器缺少 URL 编码模式")?;
    let media = Url::parse(&decode_media(&s(&payload["data"], "url"), mode)?)
        .context("看看鸭播放器没有返回有效媒体地址")?;
    if !matches!(media.scheme(), "http" | "https")
        || !media.username().is_empty()
        || media.password().is_some()
    {
        bail!("看看鸭媒体必须使用无凭据的 HTTP 或 HTTPS 地址");
    }
    let mut header = super::media_headers(sources, site, media.as_str())?;
    if media.origin() == root.origin() {
        header = cookie_headers(&header, &media_session);
    }
    remove_header(&mut header, "proxy-authorization");
    remove_header(&mut header, "range");
    header["Range"] = json!(format!("bytes=0-{}", SAMPLE_LIMIT - 1));
    let budget = timeout(sources, site)?;
    let operation = async {
        let response = sources
            .net
            .client
            .get(media)
            .headers(header_map(&header)?)
            .timeout(budget)
            .send()
            .await
            .context("看看鸭媒体探测失败")?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            bail!("看看鸭媒体返回 HTTP {status}");
        }
        let address = response.url().to_string();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while bytes.len() < SAMPLE_LIMIT {
            let Some(chunk) = stream.next().await else {
                break;
            };
            let chunk = chunk.context("读取看看鸭媒体探测样本失败")?;
            bytes.extend_from_slice(&chunk[..chunk.len().min(SAMPLE_LIMIT - bytes.len())]);
        }
        let report = identify(&address, &content_type, &bytes);
        if report["confidence"] != "signature" || report["kind"] == "html" {
            bail!("看看鸭播放地址未返回可识别媒体内容");
        }
        Ok::<_, anyhow::Error>((address, report))
    };
    let (address, report) = tokio::select! { biased; _=cancel.cancelled()=>bail!("请求已取消"),result=tokio::time::timeout(budget,operation)=>result.context("看看鸭媒体探测超时")?? };
    let final_media = Url::parse(&address)?;
    if !final_media.username().is_empty() || final_media.password().is_some() {
        bail!("看看鸭媒体重定向包含凭据");
    }
    let mut header = super::media_headers(sources, site, &address)?;
    if final_media.origin() == root.origin() {
        header = cookie_headers(&header, &media_session);
    }
    remove_header(&mut header, "range");
    remove_header(&mut header, "proxy-authorization");
    Ok(json!({"url":address,"header":header,"probe":report}))
}
