use crate::{
    http::{CONFIG_UA, Net, decode, is_http_url},
    model::{array, flag, merge, number, s, text},
};
use anyhow::{Context, Result, bail};
use base64::Engine;
use regex::Regex;
use serde_json::{Value, json};
use std::{collections::HashSet, sync::LazyLock};

static BARE_DOMAIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^[\w.-]+\.[a-z]{2,}(?::\d+)?(?:[/?#]|$)").expect("constant regex")
});
static LOCAL_FILE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^[^/]+\.(php|json|js|py)(?:[/?#]|$)").expect("constant regex")
});
static API_FILE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\.(php|json|js|py)(?:[/?]|$)").expect("constant regex"));
static PYTHON_FILE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\.py(?:\?|$)").expect("constant regex"));
static JS_FILE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\.js(?:\?|$)").expect("constant regex"));
static CIPHER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^2423[0-9a-f]{16,}").expect("constant regex"));
use tokio_util::sync::CancellationToken;

pub fn normalize_url(input: &str) -> String {
    let input = input.trim();
    if is_windows_path(input) {
        return input.trim_end_matches(['/', '\\']).to_owned();
    }
    match url::Url::parse(input) {
        Ok(mut url) => {
            url.set_fragment(None);
            if url.path().len() > 1 {
                let path = url.path().trim_end_matches('/').to_owned();
                url.set_path(&path);
            }
            url.to_string()
        }
        Err(_) => input.trim_end_matches('/').to_lowercase(),
    }
}

pub fn is_windows_path(input: &str) -> bool {
    input.starts_with("\\\\")
        || (input.len() >= 3
            && input.as_bytes()[0].is_ascii_alphabetic()
            && input.as_bytes()[1] == b':'
            && matches!(input.as_bytes()[2], b'/' | b'\\'))
}

pub fn resolve(input: &str, base: &str) -> String {
    if input.is_empty() || !is_http_url(base) || url::Url::parse(input).is_ok() {
        return input.to_owned();
    }
    url::Url::parse(base)
        .and_then(|base| base.join(input))
        .map(|u| {
            u.to_string()
                .replace("%7B", "{")
                .replace("%7D", "}")
                .replace("%7b", "{")
                .replace("%7d", "}")
        })
        .unwrap_or_else(|_| input.to_owned())
}

pub fn short_hash(input: &str) -> String {
    let mut hash: u32 = 0x811c9dc5;
    for unit in input.encode_utf16() {
        hash = (hash ^ u32::from(unit)).wrapping_mul(0x01000193);
    }
    let mut out = String::new();
    loop {
        out.push(char::from_digit(hash % 36, 36).expect("base36"));
        hash /= 36;
        if hash == 0 {
            break;
        }
    }
    out.chars().rev().collect()
}

pub fn host_name(input: &str) -> String {
    if is_windows_path(input) {
        return input
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("本地配置")
            .trim_end_matches(".json")
            .to_owned();
    }
    url::Url::parse(input)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_else(|| "手动影视源".to_owned())
}

pub fn detect(bytes: &[u8], content_type: &str) -> Value {
    if content_type.to_lowercase().starts_with("image/")
        || bytes.starts_with(&[0xff, 0xd8])
        || bytes.starts_with(b"\x89PNG")
    {
        return json!({"kind":"image"});
    }
    let body = decode(bytes, content_type);
    let body = body.trim();
    let lower = body.to_lowercase();
    if body.starts_with("#EXTM3U") {
        return json!({"kind":"m3u"});
    }
    if lower.contains("<rss") && (lower.contains("<list") || lower.contains("<class")) {
        return json!({"kind":"cms"});
    }
    if CIPHER.is_match(body) {
        return json!({"kind":"encrypted"});
    }
    match serde_json::from_str::<Value>(body) {
        Ok(raw) => {
            if raw["sites"].is_array() {
                return json!({"kind":"tvbox","name":raw["name"],"siteCount":array(&raw["sites"]).len()});
            }
            let repos = if raw.is_array() {
                Some(&raw)
            } else {
                raw.get("urls").or_else(|| raw.get("repos"))
            };
            if let Some(repos) = repos.filter(|v| v.is_array())
                && (!raw.is_array()
                    || (!array(repos).is_empty()
                        && array(repos).iter().all(|v| v["url"].is_string())))
            {
                return json!({"kind":"multirepo","name":raw.get("name"),"repoCount":array(repos).len()});
            }
            if raw["class"].is_array() || raw["list"].is_array() {
                return json!({"kind":"cms"});
            }
        }
        Err(_)
            if ["<html", "<body", "<!doctype"]
                .iter()
                .any(|v| lower.contains(v)) =>
        {
            return json!({"kind":"html"});
        }
        Err(_) => {}
    }
    json!({"kind":"unknown"})
}

fn rules(input: &Value) -> Value {
    let aliases: &[(&str, &[&str])] = &[
        ("list", &["list", "vodList", "listRule"]),
        ("name", &["name", "vodName", "title"]),
        ("link", &["link", "vodLink", "href"]),
        ("pic", &["pic", "vodPic", "image"]),
        ("remarks", &["remarks", "vodRemarks", "note"]),
        ("search", &["search", "searchUrl"]),
        ("detailName", &["detailName"]),
        ("detailContent", &["detailContent", "content"]),
        ("playList", &["playList", "playFrom"]),
        ("playItem", &["playItem", "playUrl"]),
    ];
    let mut out = json!({});
    for (key, names) in aliases {
        if let Some(value) = names
            .iter()
            .map(|name| s(input, name))
            .find(|v| !v.is_empty())
        {
            out[*key] = json!(value);
        }
    }
    out
}

pub fn normalize_site(
    raw: &Value,
    id: &str,
    source_name: &str,
    base: &str,
    spider: &str,
    defaults: &Value,
) -> Option<Value> {
    if !raw.is_object() {
        return None;
    }
    let name = raw
        .get("name")
        .or_else(|| raw.get("title"))
        .map(text)
        .unwrap_or_default();
    let api = raw
        .get("api")
        .or_else(|| raw.get("url"))
        .or_else(|| raw["ext"].get("api"))
        .map(text)
        .unwrap_or_default();
    let api = if url::Url::parse(&api).is_ok() {
        api
    } else if BARE_DOMAIN.is_match(&api) && !LOCAL_FILE.is_match(&api) {
        format!("https://{api}")
    } else if api.starts_with('.') || api.starts_with('/') || API_FILE.is_match(&api) {
        resolve(&api, base)
    } else {
        api
    };
    if name.is_empty() && api.is_empty() {
        return None;
    }
    let rule = rules(raw.get("rules").unwrap_or(&raw["ext"]["rules"]));
    let has_rules = rule.as_object().is_some_and(|m| !m.is_empty());
    let raw_type = text(&raw["type"]).to_lowercase();
    let mut kind = match raw_type.as_str() {
        "cms" | "maccms" | "xml" | "apple" | "apples" | "苹果cms" => 3,
        "api" | "json" => 4,
        "html" | "web" | "rule" | "rules" | "page" => 1,
        "spider" | "jar" | "aggregate" => 0,
        "0" => 0,
        "1" => 1,
        "2" | "other" => 2,
        "3" => 3,
        "4" => 4,
        "" if has_rules => 1,
        "" if api.contains("provide/vod") || api.contains("api.php") || api.contains("/vod/") => 3,
        "" if !api.is_empty() => 4,
        _ => 2,
    };
    if matches!(raw_type.as_str(), "0" | "1")
        && is_http_url(&api)
        && (kind == 0 || (kind == 1 && !has_rules))
    {
        kind = 3;
    }
    let key = raw
        .get("key")
        .or_else(|| raw.get("id"))
        .map(text)
        .unwrap_or_else(|| format!("csp_{}", short_hash(&format!("{name}|{api}"))));
    let mut header = defaults.get("header").cloned().unwrap_or_else(|| json!({}));
    merge(&mut header, &raw["header"]);
    let ext = if let Some(ext) = raw["ext"].as_str() {
        if ext.starts_with('.') || ext.starts_with('/') {
            resolve(ext, base)
        } else {
            ext.to_owned()
        }
    } else if !raw["ext"].is_null() {
        raw["ext"].to_string()
    } else {
        String::new()
    };
    let jar = raw["jar"]
        .as_str()
        .map(|v| resolve(v, base))
        .unwrap_or_else(|| spider.to_owned());
    let searchable = flag(&raw["searchable"], true);
    let mut site = json!({"key":key,"name":if name.is_empty(){&key}else{&name},"type":kind,"api":api,"ext":ext,"jar":jar,"searchable":searchable,"quickSearch":flag(&raw["quickSearch"],searchable),"filterable":flag(&raw["filterable"],kind==3||kind==4),"playerType":number(&raw["playerType"],1.0),"header":header,"sourceId":id,"sourceName":source_name,"enabled":flag(&raw["enabled"],true),"rules":rule});
    let fallback_timeout = number(&defaults["timeout"], 0.0);
    let timeout = number(&raw["timeout"], fallback_timeout);
    let timeout = if timeout == 0.0 {
        fallback_timeout
    } else {
        timeout
    };
    if timeout != 0.0 {
        site["timeout"] = json!(timeout);
    }
    for key in ["categories", "style"] {
        if !raw[key].is_null() {
            site[key] = raw[key].clone();
        }
    }
    let lower = api.to_lowercase();
    let unsupported = if api.is_empty()
        || ["proxy:", "assets:", "file:", "clan:", "to:"]
            .iter()
            .any(|s| lower.starts_with(s))
    {
        Some("该站点依赖内置 Spider 或本地资源，桌面端无法直接请求")
    } else if lower.starts_with("csp_") {
        site["spiderClass"] = json!(format!("com.github.catvod.spider.{}", &api[4..]));
        if raw.get("enabled").is_none() {
            site["enabled"] = json!(false);
        }
        if jar.is_empty() {
            Some("该站点依赖 Spider，但配置没有提供 spider 文件")
        } else {
            None
        }
    } else if PYTHON_FILE.is_match(&api) {
        Some("该站点依赖 Python Spider（.py），当前版本未内置 Python 运行时")
    } else if lower.contains("drpy") || JS_FILE.is_match(&api) || api.starts_with('.') {
        Some("该站点依赖 JS Spider（drpy），当前版本未内置 JS 运行时")
    } else if kind == 0 {
        Some("该站点需要配套 Spider（jar/dex），当前配置未提供可运行文件")
    } else if kind == 2 {
        Some("未知站点类型，暂不支持")
    } else {
        None
    };
    if let Some(reason) = unsupported {
        site["unsupported"] = json!(reason);
        site["searchable"] = json!(false);
        site["quickSearch"] = json!(false);
    }
    Some(site)
}

pub fn parse_config(raw: &Value, id: &str, name: &str, base: &str, timeout: f64) -> Value {
    let desktop = &raw["tvboxDesktop"];
    let mut defaults = json!({"timeout":number(&desktop["timeout"],number(&raw["settings"]["timeout"],timeout)),"header":{}});
    merge(&mut defaults["header"], &raw["header"]);
    let spider = resolve(&s(raw, "spider"), base);
    let mut seen = HashSet::new();
    let sites: Vec<Value> = array(&raw["sites"])
        .iter()
        .filter_map(|site| normalize_site(site, id, name, base, &spider, &defaults))
        .filter(|site| seen.insert(s(site, "key")))
        .collect();
    let parses: Vec<Value> = array(&raw["parses"])
        .into_iter()
        .filter(|p| !s(p, "url").is_empty())
        .map(|mut p| {
            p["url"] = json!(resolve(&s(&p, "url"), base));
            let kind = number(&p["type"], 1.0);
            p["type"] = json!(if kind == 0.0 { 1.0 } else { kind });
            if s(&p, "name").is_empty() {
                p["name"] = json!("未命名解析");
            }
            p
        })
        .collect();
    let lives: Vec<Value> = array(&raw["lives"])
        .into_iter()
        .map(|p| {
            if p.is_string() {
                json!({"name":"直播源","type":0,"url":p})
            } else {
                p
            }
        })
        .filter(|p| {
            !s(p, "url").is_empty()
                && !["proxy:", "assets:", "clan:", "file:", "to:"]
                    .iter()
                    .any(|x| s(p, "url").to_ascii_lowercase().starts_with(x))
        })
        .map(|mut p| {
            p["url"] = json!(resolve(&s(&p, "url"), base));
            if s(&p, "name").is_empty() {
                p["name"] = json!("直播源");
            }
            if p["type"].is_null() {
                p["type"] = json!(0);
            }
            if !p["epg"].is_null() {
                p["epg"] = json!(text(&p["epg"]));
            }
            p
        })
        .collect();
    let mut out = json!({"name":raw.get("name").or_else(||desktop.get("name")).cloned().unwrap_or_else(||json!(name)),"notice":raw["notice"],"sites":sites,"parses":parses,"lives":lives,"flags":array(&raw["flags"]),"headers":raw.get("headers").or_else(||raw["settings"].get("headers")).cloned().unwrap_or_else(||json!([])),"raw":raw});
    for key in ["theme", "externalPlayer", "home"] {
        out[key] = raw
            .get(key)
            .or_else(|| desktop.get(key))
            .cloned()
            .unwrap_or(Value::Null);
    }
    out
}

pub fn parse_json_text(input: &str) -> Result<Value> {
    let input = input.trim_start_matches('\u{feff}').trim();
    let start = input.find(['{', '[']).context("配置内容不是合法 JSON")?;
    let end = input.rfind(['}', ']']).context("配置内容不是合法 JSON")?;
    if end <= start {
        bail!("配置内容不是合法 JSON");
    }
    json5::from_str(&input[start..=end]).context("无法解析配置 JSON")
}

pub fn image_tail(bytes: &[u8]) -> Result<String> {
    let end = bytes
        .windows(2)
        .rposition(|v| v == [0xff, 0xd9])
        .map(|i| i + 2)
        .or_else(|| {
            bytes
                .windows(4)
                .rposition(|v| v == b"IEND")
                .map(|i| (i + 8).min(bytes.len()))
        })
        .context("图片配置缺少尾部标记")?;
    let encoded: Vec<u8> = bytes[end..]
        .iter()
        .copied()
        .filter(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
        .collect();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .context("图片配置 Base64 无效")?;
    String::from_utf8(decoded).context("图片配置不是 UTF-8")
}

pub async fn load_payload(
    net: &Net,
    address: &str,
    timeout: f64,
    cancel: &CancellationToken,
) -> Result<Value> {
    if !is_http_url(address) {
        return parse_json_text(
            &tokio::fs::read_to_string(address)
                .await
                .context("无法读取本地配置")?,
        );
    }
    let response = net
        .get(address, &json!({"User-Agent":CONFIG_UA}), timeout, cancel)
        .await?;
    let content_type = response
        .headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if content_type.starts_with("image/")
        || response.bytes.starts_with(&[0xff, 0xd8])
        || response.bytes.starts_with(b"\x89PNG")
    {
        return parse_json_text(&image_tail(&response.bytes)?);
    }
    let content = decode(&response.bytes, content_type);
    let content = content.trim();
    if CIPHER.is_match(content) {
        let response = net.request(crate::http::Request {url: "http://肥猫.net/api/parse",method: reqwest::Method::POST,headers: &json!({"User-Agent":CONFIG_UA,"Content-Type":"application/json","X-Requested-With":"fetch"}),body: Some(serde_json::to_vec(&json!({"url":"","content":content}))?),timeout: timeout.max(30.0),allow_http_error: false},cancel).await?;
        let payload: Value =
            serde_json::from_str(&response.text()).context("解密服务返回内容无效")?;
        if payload["ok"] == false {
            bail!("{}", s(&payload, "error"));
        }
        let output = payload
            .get("json")
            .or_else(|| payload.get("config"))
            .context("解密服务没有返回配置内容")?;
        return if let Some(output) = output.as_str() {
            parse_json_text(output)
        } else if output.is_object() || output.is_array() {
            Ok(output.clone())
        } else {
            bail!("解密服务没有返回配置内容")
        };
    }
    parse_json_text(content)
}

pub fn multi_refs(raw: &Value, base: &str) -> Vec<Value> {
    let entries = if raw.is_array() {
        raw
    } else {
        raw.get("urls")
            .or_else(|| raw.get("repos"))
            .unwrap_or(&Value::Null)
    };
    array(entries)
        .into_iter()
        .map(|v| {
            if v.is_string() {
                json!({"name":v,"url":v})
            } else {
                v
            }
        })
        .filter(|v| !s(v, "url").is_empty())
        .map(|mut v| {
            v["url"] = json!(resolve(&s(&v, "url"), base));
            v
        })
        .collect()
}
