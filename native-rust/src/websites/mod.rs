pub mod animoe;
pub mod kankanya;

use crate::{
    model::{merge, number, s},
    sources::Sources,
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use url::Url;

#[derive(Clone, Copy)]
enum Website {
    Kankanya,
    Animoe,
}

fn website(site: &Value) -> Option<Website> {
    let address = Url::parse(&s(site, "api")).ok()?;
    if !matches!(address.scheme(), "http" | "https")
        || address.path() != "/"
        || address.query().is_some()
        || address.fragment().is_some()
        || !address.username().is_empty()
        || address.password().is_some()
    {
        return None;
    }
    match address.host_str()? {
        "www.kankanya.pro" | "kankanya.pro" => Some(Website::Kankanya),
        "animoe.org" | "www.animoe.org" => Some(Website::Animoe),
        _ => None,
    }
}

pub fn supported(site: &Value) -> bool {
    website(site).is_some()
}

pub fn handles_address(site: &Value, address: &str) -> bool {
    supported(site)
        && Url::parse(&s(site, "api"))
            .ok()
            .zip(Url::parse(address).ok())
            .is_some_and(|(base, target)| {
                base.origin() == target.origin()
                    && target.username().is_empty()
                    && target.password().is_none()
            })
}

pub(super) fn media_headers(sources: &Sources, site: &Value, address: &str) -> Result<Value> {
    let root = Url::parse(&s(site, "api"))?;
    let target = Url::parse(address)?;
    if !matches!(target.scheme(), "http" | "https")
        || !target.username().is_empty()
        || target.password().is_some()
    {
        bail!("媒体地址必须使用无凭据的 HTTP 或 HTTPS 地址");
    }
    let mut media_site = site.clone();
    if let Some(header) = media_site["header"].as_object_mut() {
        header.retain(|key, _| {
            !key.eq_ignore_ascii_case("proxy-authorization")
                && (root.origin() == target.origin()
                    || !["cookie", "authorization"]
                        .iter()
                        .any(|v| key.eq_ignore_ascii_case(v)))
        });
    }
    let mut header = json!({"Referer":root.as_str()});
    merge(&mut header, &sources.headers(&media_site, address)?);
    Ok(header)
}

fn budget(sources: &Sources, site: &Value) -> Result<Duration> {
    Ok(Duration::from_secs_f64(
        number(
            &site["timeout"],
            number(&sources.store.settings()?["timeout"], 20.0),
        )
        .clamp(1.0, 300.0),
    ))
}

pub async fn request(
    sources: &Sources,
    site: &Value,
    query: &Value,
    cancel: &CancellationToken,
) -> Result<Value> {
    let adapter = website(site).context("没有匹配的原生网站适配器")?;
    // 避免适配器的多阶段 HTTP future 在目录缓存的嵌套调用中放大栈占用。
    let operation = async {
        match adapter {
            Website::Kankanya => Box::pin(kankanya::request(sources, site, query, cancel)).await,
            Website::Animoe => Box::pin(animoe::request(sources, site, query, cancel)).await,
        }
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => bail!("请求已取消"),
        result = tokio::time::timeout(budget(sources, site)?, operation) => {
            result.context("网站请求整体超时")?
        }
    }
}

pub async fn resolve(
    sources: &Sources,
    site: &Value,
    address: &str,
    cancel: &CancellationToken,
) -> Result<Value> {
    if !handles_address(site, address) {
        bail!("网站播放页必须属于当前影视源");
    }
    let adapter = website(site).context("没有匹配的原生网站适配器")?;
    let operation = async {
        match adapter {
            Website::Kankanya => Box::pin(kankanya::resolve(sources, site, address, cancel)).await,
            Website::Animoe => Box::pin(animoe::resolve(sources, site, address, cancel)).await,
        }
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => bail!("请求已取消"),
        result = tokio::time::timeout(budget(sources, site)?, operation) => {
            result.context("网站播放解析整体超时")?
        }
    }
}
