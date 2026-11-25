use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use regex::Regex;
use reqwest::{
    Client, Method,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use serde_json::{Value, json};
use std::{sync::OnceLock, time::Duration};
use tokio_util::sync::CancellationToken;

pub const DEFAULT_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";
pub const CONFIG_UA: &str = "okhttp/3.12.0";

pub fn is_http_url(input: &str) -> bool {
    url::Url::parse(input).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
}

#[derive(Clone)]
pub struct Net {
    pub client: Client,
}
pub struct Request<'a> {
    pub url: &'a str,
    pub method: Method,
    pub headers: &'a Value,
    pub body: Option<Vec<u8>>,
    pub timeout: f64,
    pub allow_http_error: bool,
}

#[derive(Debug)]
pub struct RawResponse {
    pub status: u16,
    pub url: String,
    pub headers: HeaderMap,
    pub bytes: Vec<u8>,
}

impl Net {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .user_agent(DEFAULT_UA)
                .pool_idle_timeout(Duration::from_secs(90))
                .connect_timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::limited(10))
                .build()?,
        })
    }

    pub async fn request(
        &self,
        request: Request<'_>,
        cancel: &CancellationToken,
    ) -> Result<RawResponse> {
        let Request {
            url,
            method,
            headers,
            body,
            timeout,
            allow_http_error,
        } = request;
        let parsed = url::Url::parse(url).context("请求地址无效")?;
        if !matches!(parsed.scheme(), "http" | "https") {
            bail!("请求地址必须使用 HTTP 或 HTTPS");
        }
        let mut builder = self
            .client
            .request(method, parsed)
            .headers(header_map(headers)?)
            .timeout(Duration::from_secs_f64(timeout.clamp(1.0, 300.0)));
        if let Some(body) = body {
            builder = builder.body(body);
        }
        let operation = async {
            let response = builder.send().await.context("网络请求失败")?;
            let status = response.status().as_u16();
            if !allow_http_error && !(200..300).contains(&status) {
                bail!("HTTP {status}");
            }
            let url = response.url().to_string();
            let headers = response.headers().clone();
            let mut bytes = Vec::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.context("读取响应失败")?;
                if bytes.len().saturating_add(chunk.len()) > 200 * 1024 * 1024 {
                    bail!("响应超过 200MB 上限");
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(RawResponse {
                status,
                url,
                headers,
                bytes,
            })
        };
        tokio::select! { biased; _ = cancel.cancelled() => bail!("请求已取消"), result = operation => result }
    }

    pub async fn get(
        &self,
        url: &str,
        headers: &Value,
        timeout: f64,
        cancel: &CancellationToken,
    ) -> Result<RawResponse> {
        self.request(
            Request {
                url,
                method: Method::GET,
                headers,
                body: None,
                timeout,
                allow_http_error: false,
            },
            cancel,
        )
        .await
    }

    pub async fn text(
        &self,
        url: &str,
        headers: &Value,
        timeout: f64,
        cancel: &CancellationToken,
    ) -> Result<String> {
        Ok(self.get(url, headers, timeout, cancel).await?.text())
    }
}

impl RawResponse {
    pub fn text(&self) -> String {
        decode(
            &self.bytes,
            self.headers
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or(""),
        )
    }
}

pub fn header_map(value: &Value) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    if let Some(object) = value.as_object() {
        for (name, value) in object {
            if let Some(value) = value.as_str() {
                headers.insert(
                    HeaderName::from_bytes(name.as_bytes()).context("请求头名称无效")?,
                    HeaderValue::from_str(value).context("请求头内容无效")?,
                );
            }
        }
    }
    Ok(headers)
}

pub fn decode(bytes: &[u8], content_type: &str) -> String {
    static CHARSET: OnceLock<Regex> = OnceLock::new();
    static META: OnceLock<Regex> = OnceLock::new();
    let charset =
        CHARSET.get_or_init(|| Regex::new(r#"(?i)charset=["']?([\w-]+)"#).expect("constant regex"));
    let meta = META.get_or_init(|| {
        Regex::new(r#"(?i)(?:charset\s*=\s*["']?|encoding\s*=\s*["'])([\w-]+)"#)
            .expect("constant regex")
    });
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
    let label = charset
        .captures(content_type)
        .or_else(|| meta.captures(&head))
        .and_then(|m| m.get(1))
        .map(|m| m.as_str())
        .unwrap_or("utf-8");
    let encoding = encoding_rs::Encoding::for_label(label.as_bytes()).unwrap_or(encoding_rs::UTF_8);
    encoding
        .decode(bytes)
        .0
        .trim_start_matches('\u{feff}')
        .to_owned()
}

pub fn match_headers(url: &str, rules: &[Value]) -> Value {
    let mut out = json!({});
    let Ok(address) = url::Url::parse(url) else {
        return out;
    };
    let Some(host) = address.host_str() else {
        return out;
    };
    let host = match address.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    for rule in rules {
        let pattern = crate::model::s(rule, "host").to_lowercase();
        if !pattern.is_empty() && wildcard(&host.to_lowercase(), &pattern) {
            crate::model::merge(&mut out, &rule["header"]);
        }
    }
    out
}

fn wildcard(input: &str, pattern: &str) -> bool {
    let (input, pattern) = (input.as_bytes(), pattern.as_bytes());
    let (mut i, mut p, mut star, mut matched) = (0, 0, None, 0);
    while i < input.len() {
        if p < pattern.len() && pattern[p] == input[i] {
            i += 1;
            p += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            p += 1;
            matched = i;
        } else if let Some(previous) = star {
            matched += 1;
            i = matched;
            p = previous + 1;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}
