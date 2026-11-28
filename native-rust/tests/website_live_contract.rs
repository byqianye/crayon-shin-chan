use anyhow::{Result, anyhow};
use crayon_core::{
    catalogue::Catalogue,
    cms,
    media::Media,
    model::{array, number, s},
    sources::Sources,
    storage::Store,
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Instant};
use tokio_util::sync::CancellationToken;

struct LiveFixture {
    catalogue: Arc<Catalogue>,
    _folder: tempfile::TempDir,
}

impl LiveFixture {
    fn new(home_site: &str) -> Self {
        let folder = tempfile::tempdir().expect("create isolated website test directory");
        let store = Arc::new(Store::open(folder.path()).expect("open isolated website test store"));
        store
            .write(
                "sources",
                &json!([{
                    "id":"website-live","name":"公网验收","enabled":true,
                    "url":"https://example.invalid/website-live.json",
                    "raw":{
                        "sites":[
                            {"key":"kankanya","name":"看看鸭","type":4,"api":"https://www.kankanya.pro/","timeout":15},
                            {"key":"animoe","name":"Animoe","type":4,"api":"https://animoe.org/","timeout":15}
                        ],
                        "home":{"sections":[{"site":home_site,"title":"公网验收"}]}
                    }
                }]),
            )
            .expect("write isolated website source configuration");
        let sources = Arc::new(Sources::new(store).expect("parse isolated website sources"));
        Self {
            catalogue: Arc::new(Catalogue::new(sources)),
            _folder: folder,
        }
    }
}

// Network errors may carry signed URLs; only fixed categories enter test output.
fn checked<T>(site: &str, stage: &str, result: Result<T>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            let message = format!("{error:#}").to_lowercase();
            if site == "animoe" && message.contains("cloudflare") {
                println!(
                    "site=animoe status=blocked stage={stage} reason=Cloudflare live_playback_verified=false"
                );
                return None;
            }
            let category = if error.chain().any(|cause| {
                cause
                    .downcast_ref::<reqwest::Error>()
                    .is_some_and(reqwest::Error::is_timeout)
            }) || message.contains("超时")
            {
                "timeout"
            } else if message.contains("enc") && message.contains("清单") {
                "unsupported_playlist"
            } else if message.contains("未返回可识别媒体") {
                "unrecognized_media"
            } else {
                "request_or_contract"
            };
            let redactor =
                regex::Regex::new(r#"https?://[^\s"<>)]*"#).expect("URL redaction regex");
            let reason = error
                .chain()
                .map(|cause| {
                    redactor
                        .replace_all(&cause.to_string(), "[URL]")
                        .into_owned()
                })
                .collect::<Vec<_>>()
                .join(": ");
            panic!("site={site} status=failed stage={stage} category={category} reason={reason}");
        }
    }
}

fn detail_cache_key(catalogue: &Catalogue, site: &Value, id: &str) -> Result<String> {
    let address = cms::build_url(site, &json!({"ac":"detail","ids":id}))?;
    let headers = catalogue.sources.headers(site, &address)?;
    let timeout = number(
        &site["timeout"],
        number(&catalogue.sources.store.settings()?["timeout"], 20.0),
    );
    let identity = json!([
        catalogue.sources.revision(),
        site["sourceId"],
        site["key"],
        site["api"],
        headers,
        timeout
    ])
    .to_string();
    Ok(format!("detail:{identity}:{id}"))
}

async fn verify_live_site(site_key: &str, keyword: &str) {
    let fixture = LiveFixture::new(site_key);
    let catalogue = &fixture.catalogue;
    let site = catalogue
        .sources
        .site(site_key)
        .expect("configured website");
    assert!(site["type"] == 4, "website must use type 4 dispatch");
    assert!(
        s(&site, "spiderClass").is_empty(),
        "website must use native dispatch"
    );
    let cancel = CancellationToken::new();

    let started = Instant::now();
    let Some(classes) = checked(site_key, "classes", catalogue.classes(&site).await) else {
        return;
    };
    let classes_ms = started.elapsed().as_millis();
    assert!(
        !classes.is_empty(),
        "site={site_key} stage=classes empty result"
    );

    let started = Instant::now();
    let Some(home) = checked(site_key, "home", catalogue.home(false).await) else {
        return;
    };
    let home_ms = started.elapsed().as_millis();
    let section = home
        .iter()
        .find(|section| section["siteKey"] == site_key)
        .expect("configured website home section");
    if let Some(error) = section.get("error") {
        let Some(()) = checked(site_key, "home", Err(anyhow!(error.to_string()))) else {
            return;
        };
    }
    let home_count = array(&section["items"]).len();
    assert!(home_count > 0, "site={site_key} stage=home empty result");

    let started = Instant::now();
    let search = if keyword.is_empty() {
        json!({"ok":true,"list":section["items"]})
    } else {
        catalogue.search_site(&site, keyword, 20, &cancel).await
    };
    let search_ms = started.elapsed().as_millis();
    if search["ok"] != true {
        let Some(()) = checked(site_key, "search", Err(anyhow!(s(&search, "message")))) else {
            return;
        };
    }
    let works = array(&search["list"]);
    assert!(
        !works.is_empty(),
        "site={site_key} stage=search empty result"
    );
    let id = s(&works[0], "vod_id");
    assert!(
        !id.is_empty(),
        "site={site_key} stage=search missing work id"
    );

    // Search and home may return details, so explicitly start with a cold cache.
    catalogue.cache.clear();
    let cache_key = detail_cache_key(catalogue, &site, &id).expect("construct detail cache key");
    assert!(
        catalogue.cache.read(&cache_key).is_none(),
        "detail cache must start empty"
    );
    let started = Instant::now();
    let Some(cold_detail) = checked(
        site_key,
        "detail_cold",
        catalogue.detail(site_key, &id).await,
    ) else {
        return;
    };
    let cold_detail_ms = started.elapsed().as_millis();
    let cached = catalogue
        .cache
        .read(&cache_key)
        .expect("detail response must populate its cache entry");
    assert!(
        cms::detail(&cached, &site) == cold_detail,
        "cached detail must match the returned work"
    );
    let started = Instant::now();
    let Some(hot_detail) = checked(
        site_key,
        "detail_hot",
        catalogue.detail(site_key, &id).await,
    ) else {
        return;
    };
    let hot_detail_ms = started.elapsed().as_millis();
    assert!(
        cold_detail == hot_detail,
        "repeated detail must return the cached work"
    );
    assert!(
        catalogue.cache.read(&cache_key).as_ref() == Some(&cached),
        "repeated detail must preserve the cache entry"
    );

    let lines = array(&cold_detail["lines"]);
    let episode_count: usize = lines
        .iter()
        .map(|line| array(&line["episodes"]).len())
        .sum();
    assert!(
        !lines.is_empty() && episode_count > 0,
        "site={site_key} stage=detail missing episodes"
    );
    println!(
        "site={site_key} status=catalogue_verified classes={} works={} home_works={home_count} lines={} episodes={episode_count} classes_ms={classes_ms} home_ms={home_ms} search_ms={search_ms} cold_detail_ms={cold_detail_ms} hot_detail_ms={hot_detail_ms} cache_entry_verified=true",
        classes.len(),
        works.len(),
        lines.len()
    );

    let media = Media::new(catalogue.clone()).expect("create media service");
    let started = Instant::now();
    let mut candidate = Err(anyhow!("前四条线路均没有可播放分集"));
    let redactor = regex::Regex::new(r#"https?://[^\s"<>)]*"#).unwrap();
    for (index, line) in lines.iter().take(4).enumerate() {
        let episode_url = s(&line["episodes"][0], "url");
        if episode_url.is_empty() {
            continue;
        }
        candidate = media
            .resolve(
                &episode_url,
                &json!({"siteKey":site_key,"flag":s(line,"name")}),
                &cancel,
            )
            .await;
        if candidate.is_ok() {
            println!("site={site_key} status=line_media_verified line_index={index}");
            break;
        }
        let reason = candidate.as_ref().unwrap_err().to_string();
        println!(
            "site={site_key} status=line_failed line_index={index} reason={}",
            redactor.replace_all(&reason, "[URL]")
        );
    }
    let Some(resolved) = checked(site_key, "resolve", candidate) else {
        return;
    };
    let resolve_ms = started.elapsed().as_millis();
    assert!(
        resolved["ok"] == true,
        "site={site_key} stage=resolve unsuccessful result"
    );
    let media_url = s(&resolved, "url");
    assert!(
        !media_url.is_empty(),
        "site={site_key} stage=resolve missing media URL"
    );
    let started = Instant::now();
    let Some(probe) = checked(
        site_key,
        "inspect",
        media.inspect(&media_url, &cancel).await,
    ) else {
        return;
    };
    let inspect_ms = started.elapsed().as_millis();
    assert!(
        probe["ok"] == true,
        "site={site_key} stage=inspect media not verified"
    );
    assert!(
        probe["confidence"] == "signature",
        "site={site_key} stage=inspect missing media signature"
    );
    let kind = s(&probe, "kind");
    assert!(
        matches!(
            kind.as_str(),
            "hls" | "dash" | "mp4" | "webm" | "flv" | "ts" | "mkv" | "audio"
        ),
        "site={site_key} stage=inspect unexpected media kind"
    );
    let decode_verified = probe["decodeVerified"]
        .as_bool()
        .expect("media report decode verification field");
    println!(
        "site={site_key} status=verified evidence=media_signature resolve_ms={resolve_ms} inspect_ms={inspect_ms} media_kind={kind} confidence=signature decode_verified={decode_verified} live_playback_verified=false"
    );
}

#[tokio::test]
#[ignore = "公网验收：访问看看鸭及媒体 CDN，需显式运行并查看输出"]
async fn kankanya_live_catalogue_cache_and_media_contract() {
    verify_live_site("kankanya", "庆余年").await;
}

#[tokio::test]
#[ignore = "公网验收：检查看看鸭首页最近更新作品的媒体，需显式运行"]
async fn kankanya_live_recent_media_contract() {
    verify_live_site("kankanya", "").await;
}

#[tokio::test]
#[ignore = "公网验收：Animoe 遇 Cloudflare 明确输出 blocked，不代表实时播放通过"]
async fn animoe_live_catalogue_cache_and_media_contract() {
    verify_live_site("animoe", "地狱乐").await;
}
