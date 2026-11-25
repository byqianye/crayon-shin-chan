use crate::{model::s, sources::Sources};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

pub fn list() -> Vec<Value> {
    vec![
        json!({"id":"public-cms","name":"公开可用源（苹果CMS 采集站）","desc":"5 个公开采集接口 + 1 个公开直播源。这类源随时可能失效，失效后在「影视源」「电视直播」页一看便知。","siteCount":5,"liveCount":1}),
        json!({"id":"fm-config","name":"肥猫接口","desc":"启动后自动同步和解密。普通站点按配置启用，第三方 Spider 可在「影视源」页确认后批量启用。","remote":true,"siteCount":0,"liveCount":0}),
    ]
}
pub fn add(sources: &Sources, id: &str) -> Result<Value> {
    match id {
        "fm-config" => sources.add(&json!({"name":"肥猫接口","url":"http://肥猫.net"})),
        "public-cms" => {
            let dir = sources.store.root.join("presets");
            std::fs::create_dir_all(&dir)?;
            let path = dir.join("public-cms.json");
            let address = path.to_string_lossy().into_owned();
            if sources.list()?.iter().any(|v| s(v, "url") == address) {
                bail!("这个内置订阅已经添加过了");
            }
            if !path.exists() {
                std::fs::write(&path, include_bytes!("../../samples/public-sources.json"))
                    .context("无法写入内置订阅")?;
            }
            sources.add(&json!({"name":"公开可用源（苹果CMS 采集站）","url":address}))
        }
        _ => bail!("没有这个内置订阅"),
    }
}
pub async fn ensure(sources: &Sources) -> Result<()> {
    if std::env::var("CRAYON_DISABLE_BUNDLED_SOURCES").as_deref() == Ok("1") {
        return Ok(());
    }
    let store = &sources.store;
    if store.read("bundled-sources", json!(false))? != true {
        if sources.list()?.is_empty() {
            let created = add(sources, "public-cms")?;
            store.write("bundled-sources", &json!(true))?;
            sources.sync(&s(&created, "id")).await?;
        } else {
            store.write("bundled-sources", &json!(true))?;
        }
    }
    if store.read("bundled-remote-sources", json!(false))? != true {
        if !sources.list()?.iter().any(|v| {
            crate::config::normalize_url(&s(v, "url"))
                == crate::config::normalize_url("http://肥猫.net")
        }) {
            add(sources, "fm-config")?;
        }
        store.write("bundled-remote-sources", &json!(true))?;
    }
    Ok(())
}
pub async fn sync_remote(sources: &Sources) -> Result<Vec<Value>> {
    if std::env::var("CRAYON_DISABLE_BUNDLED_SOURCES").as_deref() == Ok("1") {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    for source in sources.list()?.iter().filter(|v| {
        v["enabled"] == true
            && crate::config::normalize_url(&s(v, "url"))
                == crate::config::normalize_url("http://肥猫.net")
    }) {
        out.push(sources.sync(&s(source, "id")).await?);
    }
    Ok(out)
}
