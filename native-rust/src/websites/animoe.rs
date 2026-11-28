use crate::{
    html_rules::{absolutize, extract, select},
    http::{Request, header_map},
    media_probe::{SAMPLE_LIMIT, identify},
    model::{array, merge, number, s, text},
    sources::Sources,
};
use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use reqwest::{Client, Method, header::LOCATION, redirect::Policy};
use scraper::{ElementRef, Html};
use serde_json::{Value, json};
use std::{collections::HashSet, sync::OnceLock, time::Duration};
use tokio_util::sync::CancellationToken;
use url::Url;

// Public rule contracts: AniBakaBaka/AniBakaRule/animoe.json and
// violetACER/kazumirules/animoe.json. Homepage routes also appear in
// Smith-Rosco/WebPages/pages/动漫/82.html (snapshot dated 2025-10-19).
fn base(site: &Value) -> Result<Url> {
    let mut url = Url::parse(&s(site, "api")).context("Animoe 站点地址无效")?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("Animoe 站点必须使用 HTTP 或 HTTPS");
    }
    url.set_path("/");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn timeout(sources: &Sources, site: &Value) -> Result<f64> {
    Ok(number(
        &site["timeout"],
        number(&sources.store.settings()?["timeout"], 20.0),
    ))
}

fn media_client() -> Result<Client> {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }
    let client = Client::builder()
        .user_agent(crate::http::DEFAULT_UA)
        .pool_idle_timeout(Duration::from_secs(90))
        .connect_timeout(Duration::from_secs(15))
        .redirect(Policy::none())
        .build()?;
    let _ = CLIENT.set(client);
    Ok(CLIENT.get().expect("media client initialized").clone())
}

fn headers(sources: &Sources, site: &Value, address: &str) -> Result<Value> {
    let mut out = json!({"Referer":base(site)?.as_str()});
    merge(&mut out, &sources.headers(site, address)?);
    Ok(out)
}

fn check_response(status: u16, body: &str) -> Result<()> {
    let lower = body.to_ascii_lowercase();
    if lower.contains("cf_chl_opt")
        || lower.contains("/cdn-cgi/challenge-platform/")
        || (lower.contains("just a moment") && lower.contains("cloudflare"))
    {
        bail!("Animoe 被 Cloudflare 安全验证拦截，当前原生 HTTP 无法读取影视内容");
    }
    if !(200..300).contains(&status) {
        bail!("Animoe 返回 HTTP {status}");
    }
    Ok(())
}

async fn fetch(
    sources: &Sources,
    site: &Value,
    address: &str,
    ajax: bool,
    cancel: &CancellationToken,
) -> Result<String> {
    let mut header = headers(sources, site, address)?;
    if ajax {
        header["X-Requested-With"] = json!("XMLHttpRequest");
    }
    let response = sources
        .net
        .request(
            Request {
                url: address,
                method: Method::GET,
                headers: &header,
                body: None,
                timeout: timeout(sources, site)?,
                allow_http_error: true,
            },
            cancel,
        )
        .await?;
    let body = response.text();
    check_response(response.status, &body)?;
    Ok(body)
}

fn route_id(address: &str, prefix: &str) -> Option<String> {
    let path = Url::parse(address).ok()?.path().to_owned();
    let id = path.strip_prefix(prefix)?.strip_suffix(".html")?;
    (!id.is_empty() && id.bytes().all(|v| v.is_ascii_digit())).then(|| id.to_owned())
}

fn same_origin(base: &str, target: &str) -> bool {
    Url::parse(base)
        .ok()
        .zip(Url::parse(target).ok())
        .is_some_and(|(base, target)| base.origin() == target.origin())
}

fn detail_address(site: &Value, id: &str) -> Result<String> {
    let root = base(site)?;
    let address = if !id.is_empty() && id.bytes().all(|v| v.is_ascii_digit()) {
        root.join(&format!("info/{id}.html"))?
    } else {
        root.join(id)?
    };
    if address.origin() != root.origin() || route_id(address.as_str(), "/info/").is_none() {
        bail!("Animoe 详情地址无效");
    }
    Ok(address.to_string())
}

fn node_text(node: ElementRef<'_>) -> String {
    node.text()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn page_classes(input: &str, address: &str) -> Vec<Value> {
    let html = Html::parse_document(input);
    let mut seen = HashSet::new();
    select(html.root_element(), "a[href]")
        .into_iter()
        .filter_map(|node| {
            let link = absolutize(node.value().attr("href")?, address);
            if !same_origin(address, &link) {
                return None;
            }
            let id = route_id(&link, "/type/")?;
            if !seen.insert(id.clone()) {
                return None;
            }
            let label = node
                .value()
                .attr("title")
                .map(str::to_owned)
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| node_text(node));
            (!label.is_empty()).then(|| json!({"type_id":id,"type_name":label,"type_pid":"anime"}))
        })
        .collect()
}

fn page_list(input: &str, address: &str, limit: usize) -> Vec<Value> {
    let html = Html::parse_document(input);
    let mut seen = HashSet::new();
    select(html.root_element(), "a[href]")
        .into_iter()
        .filter_map(|node| {
            let link = absolutize(node.value().attr("href")?, address);
            if !same_origin(address,&link) { return None; }
            route_id(&link, "/info/")?;
            let name = node.value().attr("title").map(str::to_owned)
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| extract(node, "img&&alt", address, &["alt"]));
            let name = if name.is_empty() { node_text(node) } else { name };
            if name.is_empty() || !seen.insert(link.clone()) { return None; }
            Some(json!({"vod_id":link,"vod_name":name,"vod_pic":absolutize(&extract(node,"img",address,&["data-original","data-src","src"]),address)}))
        }).take(limit).collect()
}

struct Pagination {
    count: usize,
    target: Option<String>,
    next: Option<String>,
}

fn pagination(input: &str, address: &str, current: usize, wanted: usize) -> Pagination {
    let html = Html::parse_document(input);
    let origin = Url::parse(address).ok().map(|v| v.origin());
    let mut max = current;
    let mut target = None;
    let mut next = None;
    for node in select(html.root_element(), "a[href]") {
        let label = node_text(node);
        let Some(raw) = node.value().attr("href") else {
            continue;
        };
        let Ok(url) = Url::parse(&absolutize(raw, address)) else {
            continue;
        };
        if Some(url.origin()) != origin
            || !["/type/", "/show/"]
                .iter()
                .any(|v| url.path().starts_with(v))
        {
            continue;
        }
        if let Ok(page) = label.parse::<usize>() {
            max = max.max(page);
            if page == wanted {
                target = Some(url.to_string());
            }
            if page == current.saturating_add(1) {
                next = Some(url.to_string());
            }
        } else if node
            .value()
            .attr("rel")
            .is_some_and(|v| v.split_whitespace().any(|v| v.eq_ignore_ascii_case("next")))
            || [label.as_str(), node.value().attr("title").unwrap_or("")]
                .iter()
                .any(|v| v.contains("下一页") || v.contains("下页"))
        {
            next = Some(url.to_string());
        }
    }
    if next.is_some() {
        max = max.max(current.saturating_add(1));
    }
    Pagination {
        count: max,
        target,
        next,
    }
}

fn page_detail(input: &str, address: &str) -> Result<Value> {
    let html = Html::parse_document(input);
    let root = html.root_element();
    let name = extract(root, "h1", address, &["Text"]);
    let tabs: Vec<String> = select(root, ".module-tab-item[data-dropdown-value]")
        .into_iter()
        .map(|node| {
            node.value()
                .attr("data-dropdown-value")
                .unwrap_or("")
                .trim()
                .to_owned()
        })
        .collect();
    let mut from = Vec::new();
    let mut groups = Vec::new();
    for (index, group) in select(root, ".module-play-list").into_iter().enumerate() {
        let mut seen = HashSet::new();
        let episodes: Vec<String> = select(group, "a[href]")
            .into_iter()
            .filter_map(|node| {
                let link = absolutize(node.value().attr("href")?, address);
                let url = Url::parse(&link).ok()?;
                if !same_origin(address, &link)
                    || !url.path().starts_with("/play/")
                    || !seen.insert(link.clone())
                {
                    return None;
                }
                let label = node_text(node).replace(['$', '#'], " ");
                Some(format!(
                    "{}${link}",
                    if label.is_empty() {
                        format!("第{}集", seen.len())
                    } else {
                        label
                    }
                ))
            })
            .collect();
        if episodes.is_empty() {
            continue;
        }
        let title = tabs
            .get(index)
            .filter(|v| !v.is_empty())
            .cloned()
            .unwrap_or_else(|| format!("线路{}", index + 1));
        from.push(title.replace(['$', '#'], " "));
        groups.push(episodes.join("#"));
    }
    if groups.is_empty() {
        bail!("Animoe 详情页没有可播放分集，页面结构可能已变化");
    }
    let picture = extract(
        root,
        "meta[property='og:image']&&content",
        address,
        &["content"],
    );
    let content = extract(
        root,
        "meta[name='description']&&content",
        address,
        &["content"],
    );
    Ok(
        json!({"vod_id":address,"vod_name":if name.is_empty(){extract(root,"title",address,&["Text"])}else{name},"vod_pic":absolutize(&picture,address),"vod_content":content,"vod_play_from":from.join("$$$"),"vod_play_url":groups.join("$$$")}),
    )
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
        let mut address = root.join("index.php/ajax/suggest")?;
        address
            .query_pairs_mut()
            .append_pair("mid", "1")
            .append_pair("wd", &keyword)
            .append_pair("limit", "20");
        let payload: Value =
            serde_json::from_str(&fetch(sources, site, address.as_str(), true, cancel).await?)
                .context("Animoe 搜索没有返回有效 JSON")?;
        if !payload["list"].is_array() {
            bail!("Animoe 搜索响应缺少 list 数组");
        }
        let list: Vec<_> = array(&payload["list"]).into_iter().filter_map(|item| {
            let id = text(&item["id"]);
            let address = detail_address(site,&id).ok()?;
            let name = s(&item,"name");
            (!name.trim().is_empty()).then(||json!({"vod_id":address,"vod_name":name,"vod_pic":absolutize(&s(&item,"pic"),root.as_str())}))
        }).collect();
        return Ok(json!({"code":1,"page":1,"pagecount":1,"list":list}));
    }
    if !s(query, "ids").is_empty() {
        let address = detail_address(site, &s(query, "ids"))?;
        return Ok(
            json!({"code":1,"list":[page_detail(&fetch(sources,site,&address,false,cancel).await?,&address)?]}),
        );
    }
    let class = s(query, "t");
    if !class.is_empty() && !class.bytes().all(|v| v.is_ascii_digit()) {
        bail!("Animoe 分类编号无效");
    }
    let address = if class.is_empty() {
        root.to_string()
    } else {
        root.join(&format!("type/{class}.html"))?.to_string()
    };
    let mut input = fetch(sources, site, &address, false, cancel).await?;
    if s(query, "ac") == "list" {
        let mut classes = page_classes(&input, &address);
        if classes.is_empty() {
            bail!("Animoe 页面没有分类，页面结构可能已变化");
        }
        classes.insert(
            0,
            json!({"type_id":"anime","type_name":"动漫","type_pid":"0"}),
        );
        return Ok(json!({"code":1,"class":classes}));
    }
    let page = number(&query["pg"], 1.0).max(1.0) as usize;
    let mut pagecount = 1;
    let mut actual_address = address;
    let mut current = 1;
    let mut visited = HashSet::from([actual_address.clone()]);
    if !class.is_empty() && page > 1 {
        // Discover windowed pagination through actual links without inventing a URL template.
        for _ in 0..16 {
            let navigation = pagination(&input, &actual_address, current, page);
            pagecount = pagecount.max(navigation.count);
            let (next_page, next) = if let Some(target) = navigation.target {
                (page, target)
            } else if let Some(next) = navigation.next {
                (current + 1, next)
            } else if pagecount <= current {
                return Ok(json!({"code":1,"page":page,"pagecount":pagecount,"list":[]}));
            } else {
                bail!("Animoe 分页链接不完整，无法定位请求页");
            };
            if !visited.insert(next.clone()) {
                bail!("Animoe 分页链接出现循环");
            }
            input = fetch(sources, site, &next, false, cancel).await?;
            actual_address = next;
            current = next_page;
            if current == page {
                break;
            }
        }
        if current != page {
            bail!("Animoe 分页超过单次 16 次跳转上限");
        }
    }
    pagecount = pagecount.max(pagination(&input, &actual_address, current, page).count);
    let list = page_list(&input, &actual_address, 60);
    if list.is_empty() && page == 1 {
        bail!("Animoe 页面没有作品，页面结构可能已变化");
    }
    Ok(json!({"code":1,"page":page,"pagecount":pagecount,"list":list}))
}

pub async fn resolve(
    sources: &Sources,
    site: &Value,
    address: &str,
    cancel: &CancellationToken,
) -> Result<Value> {
    let root = base(site)?;
    let page = root.join(address)?;
    if page.origin() != root.origin() || !page.path().starts_with("/play/") {
        bail!("Animoe 播放页地址无效");
    }
    let input = fetch(sources, site, page.as_str(), false, cancel).await?;
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
            .context("Animoe 播放页缺少有效 player_aaaa 配置")?
    };
    let raw = s(&player, "url");
    let address = raw.split('*').next().unwrap_or("").trim();
    let media = Url::parse(address).context("Animoe 播放器没有返回 HTTP 媒体地址")?;
    if !matches!(media.scheme(), "http" | "https")
        || !media.username().is_empty()
        || media.password().is_some()
    {
        bail!("Animoe 媒体地址必须使用 HTTP 或 HTTPS");
    }
    let budget = Duration::from_secs_f64(timeout(sources, site)?.clamp(1.0, 300.0));
    let client = media_client()?;
    let operation = async {
        let mut target = media.clone();
        let mut visited = HashSet::from([target.to_string()]);
        let mut redirects = 0;
        let response = loop {
            let mut header = super::media_headers(sources, site, target.as_str())?;
            header["Range"] = json!(format!("bytes=0-{}", SAMPLE_LIMIT - 1));
            let response = client
                .get(target.clone())
                .headers(header_map(&header)?)
                .timeout(budget)
                .send()
                .await
                .context("Animoe 媒体探测失败")?;
            if !response.status().is_redirection() {
                break response;
            }
            if redirects >= 10 {
                bail!("Animoe 媒体重定向次数过多");
            }
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .context("Animoe 媒体重定向缺少有效目标")?;
            let next = target.join(location).context("Animoe 媒体重定向目标无效")?;
            if !next.username().is_empty() || next.password().is_some() {
                bail!("Animoe 媒体重定向目标不能包含凭据");
            }
            if next.origin() != root.origin() {
                bail!("Animoe 媒体重定向目标必须属于 Animoe 源");
            }
            if !visited.insert(next.to_string()) {
                bail!("Animoe 媒体重定向出现循环");
            }
            target = next;
            redirects += 1;
        };
        let status = response.status().as_u16();
        let final_address = response.url().to_string();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while bytes.len() < SAMPLE_LIMIT {
            let Some(chunk) = stream.next().await else {
                break;
            };
            let chunk = chunk.context("读取 Animoe 媒体探测样本失败")?;
            bytes.extend_from_slice(&chunk[..chunk.len().min(SAMPLE_LIMIT - bytes.len())]);
        }
        check_response(status, &String::from_utf8_lossy(&bytes))?;
        if bytes.starts_with(b"enc") {
            bail!("Animoe 返回非标准封装的播放清单，当前尚未支持该格式");
        }
        let report = identify(&final_address, &content_type, &bytes);
        if report["confidence"] != "signature" || report["kind"] == "html" {
            bail!("Animoe 播放地址未返回可识别媒体内容");
        }
        Ok::<_, anyhow::Error>((final_address, report))
    };
    let (address, report) = tokio::select! {biased; _=cancel.cancelled()=>bail!("请求已取消"),result=tokio::time::timeout(budget,operation)=>result.context("Animoe 媒体探测超时")??};
    let mut header = super::media_headers(sources, site, &address)?;
    header
        .as_object_mut()
        .context("Animoe 请求头格式无效")?
        .remove("Range");
    Ok(json!({"url":address,"header":header,"probe":report}))
}
