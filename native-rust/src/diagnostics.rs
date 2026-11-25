use crate::{
    catalogue::Catalogue,
    cms,
    model::{array, s},
    spider,
};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{sync::Arc, time::Instant};
use tokio_util::sync::CancellationToken;

pub async fn test(catalogue: &Arc<Catalogue>, key: &str) -> Result<Value> {
    let site = catalogue.sources.site(key)?;
    let started = Instant::now();
    let cancel = CancellationToken::new();
    let outcome = async {
        if !s(&site, "unsupported").is_empty() {
            bail!("{}", s(&site, "unsupported"));
        }
        if !s(&site, "spiderClass").is_empty() {
            let data = catalogue
                .spider
                .call(&site, "home", json!({"filter":true}), 60.0, &cancel)
                .await?;
            let categories = array(&data["class"]).len();
            let videos = array(&data["list"]).len();
            if videos + categories == 0 {
                for probe in ["电影", "庆余年"] {
                    let result = catalogue.search_site(&site, probe, 3, &cancel).await;
                    if !array(&result["list"]).is_empty() {
                        return Ok(format!(
                            "Spider 正常，首页为空，搜索命中 {} 条",
                            array(&result["list"]).len()
                        ));
                    }
                }
                bail!("Spider 连通，但首页和搜索都没有返回内容");
            }
            return Ok(format!(
                "Spider 正常，{categories} 个分类 / 首页 {videos} 条"
            ));
        }
        if [3, 4].contains(&site["type"].as_i64().unwrap_or(2)) {
            let payload = catalogue
                .request_cms(&site, json!({"ac":"detail","pg":1}), None)
                .await?;
            if !cms::is_ok(&payload) {
                bail!("接口返回异常：{}", s(&payload, "msg"));
            }
            let count = array(&payload["list"]).len();
            if count == 0 {
                bail!("接口连通但未返回数据");
            }
            return Ok(format!("正常，返回 {count} 条"));
        }
        let result = catalogue.search_site(&site, "测试", 3, &cancel).await;
        if result["ok"] != true {
            bail!("{}", s(&result, "message"));
        }
        Ok("网页规则正常".into())
    }
    .await;
    let ms = started.elapsed().as_millis() as u64;
    if outcome.is_ok() {
        catalogue
            .sources
            .override_site(&[key.into()], &json!({"health":ms}))?;
    }
    Ok(match outcome {
        Ok::<String, anyhow::Error>(message) => json!({"ok":true,"ms":ms,"message":message}),
        Err(error) => {
            let message = format!("{error:#}");
            json!({"ok":false,"ms":ms,"category":spider::classify_failure(&message),"message":message})
        }
    })
}
pub async fn audit(catalogue: &Arc<Catalogue>, key: &str, probe: &str) -> Result<Value> {
    let started = Instant::now();
    let site = catalogue.sources.site(key)?;
    let classes = catalogue.classes(&site).await;
    let categories = classes.as_ref().map(Vec::len).unwrap_or(0);
    let mut result = json!({"siteKey":site["key"],"siteName":site["name"],"categories":categories,"searchHits":0,"detailLines":0,"episodes":0});
    let search = catalogue
        .search_site(
            &site,
            if probe.is_empty() { "电影" } else { probe },
            5,
            &CancellationToken::new(),
        )
        .await;
    if search["ok"] != true {
        result["ok"] = json!(false);
        result["failStage"] = json!("search");
        result["reason"] = search["message"].clone();
    } else {
        let list = array(&search["list"]);
        result["searchHits"] = json!(list.len());
        if let Some(sample) = list.first() {
            match catalogue.detail(key, &s(sample, "vod_id")).await {
                Ok(detail) => {
                    let lines = array(&detail["lines"]);
                    let episodes = lines
                        .iter()
                        .map(|v| array(&v["episodes"]).len())
                        .sum::<usize>();
                    result["detailLines"] = json!(lines.len());
                    result["episodes"] = json!(episodes);
                    result["ok"] = json!(!lines.is_empty());
                    if lines.is_empty() {
                        result["failStage"] = json!("detail");
                        result["reason"] = json!("详情没有解析出播放线路");
                    }
                }
                Err(error) => {
                    result["ok"] = json!(false);
                    result["failStage"] = json!("detail");
                    result["reason"] = json!(format!("{error:#}"));
                }
            }
        } else {
            result["ok"] = json!(true);
            result["reason"] = json!(
                classes
                    .err()
                    .map(|e| format!("搜索无结果；{e:#}"))
                    .unwrap_or("搜索无结果".into())
            );
        }
    }
    result["ms"] = json!(started.elapsed().as_millis() as u64);
    Ok(result)
}
