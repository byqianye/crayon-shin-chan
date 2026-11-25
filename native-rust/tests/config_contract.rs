use axum::{Router, routing::get};
use base64::Engine;
use crayon_core::{
    config::{image_tail, normalize_site, normalize_url, parse_config, parse_json_text},
    http::Net,
    sources::Sources,
    storage::Store,
};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[test]
fn editing_manual_source_recomputes_derived_support_and_preserves_creation_time() {
    let folder = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(folder.path()).unwrap());
    let sources = Sources::new(store).unwrap();
    let old = sources
        .add_local(&json!({"name":"旧接口","api":"https://a.invalid/spider.py","type":0}))
        .unwrap();
    assert!(old["unsupported"].is_string());
    let next = sources
        .update_local(
            old["key"].as_str().unwrap(),
            &json!({"api":"https://a.invalid/api.php","type":3,"name":"新接口"}),
        )
        .unwrap();
    assert!(next["unsupported"].is_null());
    assert!(next["spiderClass"].is_null());
    assert_eq!(next["searchable"], true);
    assert_eq!(next["createdAt"], old["createdAt"]);
    let renamed = sources
        .update_local(next["key"].as_str().unwrap(), &json!({"name":"改名"}))
        .unwrap();
    assert_eq!(renamed["key"], next["key"]);
}

struct Server {
    address: String,
    task: tokio::task::JoinHandle<()>,
}
impl Server {
    async fn new(router: Router) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self { address, task }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[test]
fn tvbox_types_spider_defaults_and_relative_resources_match_contract() {
    let raw = json!({"spider":"./spider.jar;md5;abc","header":{"Referer":"https://ref.invalid"},"sites":[
        {"key":"json","name":"JSON","type":1,"api":"../api.php?wd={wd}"},
        {"key":"web","name":"网页","type":1,"api":"../search/{wd}","rules":{"list":".item"}},
        {"key":"csp","name":"Spider","type":3,"api":"csp_FishCloud","ext":"./ext.json"},
        {"key":"py","name":"Python","api":"./main.py"}],
        "parses":[{"name":"解析","url":"../parse?url={url}"}],"lives":["./live.txt"]});
    let parsed = parse_config(
        &raw,
        "a",
        "来源",
        "https://host.invalid/sub/index.json",
        20.0,
    );
    assert_eq!(parsed["sites"][0]["type"], 3);
    assert_eq!(
        parsed["sites"][0]["api"],
        "https://host.invalid/api.php?wd={wd}"
    );
    assert_eq!(parsed["sites"][1]["type"], 1);
    assert_eq!(
        parsed["sites"][2]["spiderClass"],
        "com.github.catvod.spider.FishCloud"
    );
    assert_eq!(parsed["sites"][2]["enabled"], false);
    assert_eq!(
        parsed["sites"][2]["jar"],
        "https://host.invalid/sub/spider.jar;md5;abc"
    );
    assert!(
        parsed["sites"][3]["unsupported"]
            .as_str()
            .unwrap()
            .contains("Python")
    );
    assert_eq!(
        parsed["parses"][0]["url"],
        "https://host.invalid/parse?url={url}"
    );
    assert_eq!(
        parsed["lives"][0]["url"],
        "https://host.invalid/sub/live.txt"
    );
    assert!(
        normalize_site(
            &json!({"name":"a","type":"unrecognized","api":"https://host.invalid"}),
            "a",
            "a",
            "",
            "",
            &json!({})
        )
        .unwrap()["unsupported"]
            .is_string()
    );
}

#[test]
fn wrapped_json_comments_chinese_paths_and_image_tails_are_supported() {
    let raw = parse_json_text(
        "prefix {\"url\":\"https://a.invalid/\",/* comment */\"sites\":[],} suffix",
    )
    .unwrap();
    assert_eq!(raw["url"], "https://a.invalid/");
    assert_eq!(
        normalize_url(r"C:\中文路径\配置.json"),
        r"C:\中文路径\配置.json"
    );
    assert_eq!(
        normalize_url("HTTPS://Host.invalid:443/path///#fragment"),
        "https://host.invalid/path"
    );
    let mut bytes = vec![0xff, 0xd8, 0xff, 0xd9];
    bytes.extend(
        base64::engine::general_purpose::STANDARD
            .encode(raw.to_string())
            .as_bytes(),
    );
    assert_eq!(parse_json_text(&image_tail(&bytes).unwrap()).unwrap(), raw);
}

#[test]
fn uppercase_http_and_optional_source_fields_match_desktop_contract() {
    let parsed = parse_config(
        &json!({"sites":[{"name":"CMS","type":0,"api":"HTTP://host.invalid/api.php"}],
            "lives":[{"url":"  ./live.txt  ","epg":123}, {"url":"PrOxY://live"}],
            "parses":[{"url":" ./parse?url={url} "}]}),
        "s",
        "来源",
        "HTTPS://host.invalid/sub/index.json",
        20.0,
    );
    assert_eq!(parsed["sites"][0]["type"], 3);
    assert!(parsed["sites"][0]["unsupported"].is_null());
    assert_eq!(parsed["lives"].as_array().unwrap().len(), 1);
    assert_eq!(parsed["lives"][0]["name"], "直播源");
    assert_eq!(parsed["lives"][0]["type"], 0);
    assert_eq!(parsed["lives"][0]["epg"], "123");
    assert_eq!(
        parsed["lives"][0]["url"],
        "https://host.invalid/sub/live.txt"
    );
    assert_eq!(parsed["parses"][0]["name"], "未命名解析");
    assert_eq!(parsed["parses"][0]["type"].as_f64(), Some(1.0));
    assert_eq!(
        parsed["parses"][0]["url"],
        "https://host.invalid/sub/parse?url={url}"
    );
    assert_eq!(
        crayon_core::html_rules::absolutize("HTTPS://host.invalid/a", "https://base.invalid"),
        "HTTPS://host.invalid/a"
    );
    for (extra, expected) in [
        (json!({"settings":{"timeout":31}}), 31.0),
        (
            json!({"tvboxDesktop":{"timeout":12},"settings":{"timeout":31}}),
            12.0,
        ),
    ] {
        let mut config = extra;
        config["sites"] = json!([{"name":"站点","api":"https://host.invalid/api.php"}]);
        assert_eq!(
            parse_config(&config, "s", "来源", "", 47.0)["sites"][0]["timeout"].as_f64(),
            Some(expected)
        );
    }
    let manual = normalize_site(
        &json!({"name":"手动","api":"https://host.invalid/api.php"}),
        "s",
        "手动",
        "",
        "",
        &json!({}),
    )
    .unwrap();
    assert!(manual["timeout"].is_null());
}

#[tokio::test]
async fn syncing_and_reopening_inherit_global_timeout_and_keep_overrides() {
    let server = Server::new(Router::new().route(
        "/config",
        get(|| async {
            axum::Json(json!({"sites":[
                {"key":"inherited","name":"继承","api":"./api.php"},
                {"key":"explicit","name":"自定","api":"./api.php","timeout":9}
            ]}))
        }),
    ))
    .await;
    let folder = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(folder.path()).unwrap());
    store.save_settings(&json!({"timeout":47})).unwrap();
    let sources = Sources::new(store.clone()).unwrap();
    let source = sources
        .add(&json!({"url":format!("{}/config", server.address).replacen("http:", "HTTP:", 1)}))
        .unwrap();
    sources.sync(source["id"].as_str().unwrap()).await.unwrap();
    for loaded in [sources, Sources::new(store).unwrap()] {
        let config = &loaded.configs().unwrap()[0];
        assert_eq!(config["sites"][0]["timeout"].as_f64(), Some(47.0));
        assert_eq!(config["sites"][1]["timeout"].as_f64(), Some(9.0));
    }
}

#[tokio::test]
async fn multirepo_keeps_child_resources_headers_and_home_sections() {
    let server=Server::new(Router::new()
        .route("/index.json",get(||async{axum::Json(json!({"urls":[{"name":"子仓","url":"./child/index.json"},{"name":"坏仓","url":"./missing"}]}))}))
        .route("/child/index.json",get(|headers:axum::http::HeaderMap|async move{
            assert_eq!(headers["user-agent"],"okhttp/3.12.0");
            axum::Json(json!({"sites":[{"key":"child","name":"站点","api":"./api.php"}],"headers":[{"host":"host.invalid","header":{"Referer":"ref"}}],"home":{"sections":[{"title":"首页","site":"child"}]},"flags":["vip"]}))
        }))).await;
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(temp.path()).unwrap());
    store.save_settings(&json!({"timeout":47})).unwrap();
    let sources = Sources::new(store).unwrap();
    let source = sources
        .add(&json!({"url":format!("{}/index.json",server.address)}))
        .unwrap();
    let result = sources.sync(source["id"].as_str().unwrap()).await.unwrap();
    assert_eq!(result["kind"], "multirepo");
    assert_eq!(result["repos"][1]["ok"], false);
    let configs = sources.configs().unwrap();
    assert_eq!(
        configs[0]["sites"][0]["api"],
        format!("{}/child/api.php", server.address)
    );
    assert_eq!(configs[0]["sites"][0]["sourceName"], "子仓");
    assert_eq!(configs[0]["sites"][0]["timeout"].as_f64(), Some(47.0));
    assert_eq!(configs[0]["flags"][0], "vip");
    assert_eq!(configs[0]["home"]["sections"][0]["site"], "child");
    let reopened = Sources::new(sources.store.clone()).unwrap();
    assert_eq!(
        reopened.sites().unwrap()[0]["api"],
        format!("{}/child/api.php", server.address)
    );
}

#[tokio::test]
async fn sync_coalesces_requests_and_preserves_edits_made_while_loading() {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let server = Server::new(Router::new().route(
        "/source",
        get({
            let started = started.clone();
            let release = release.clone();
            let calls = calls.clone();
            move || {
                let started = started.clone();
                let release = release.clone();
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    started.notify_one();
                    release.notified().await;
                    axum::Json(
                        json!({"sites":[{"key":"a","name":"a","api":"https://a.invalid/api.php"}]}),
                    )
                }
            }
        }),
    ))
    .await;
    let temp = tempfile::tempdir().unwrap();
    let sources = Arc::new(Sources::new(Arc::new(Store::open(temp.path()).unwrap())).unwrap());
    let source = sources
        .add(&json!({"name":"old","url":format!("{}/source",server.address)}))
        .unwrap();
    let id = source["id"].as_str().unwrap().to_owned();
    let first = tokio::spawn({
        let sources = sources.clone();
        let id = id.clone();
        async move { sources.sync(&id).await }
    });
    started.notified().await;
    let second = tokio::spawn({
        let sources = sources.clone();
        let id = id.clone();
        async move { sources.sync(&id).await }
    });
    tokio::task::yield_now().await;
    sources
        .update(&id, &json!({"name":"new","enabled":false}))
        .unwrap();
    release.notify_one();
    let result = first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(result["enabled"], false);
    assert_eq!(result["name"], "new");
    assert!(sources.sites().unwrap().is_empty());
}

#[test]
fn manual_dedup_and_site_tombstones_do_not_remove_other_rules() {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(temp.path()).unwrap());
    store.write("sources",&json!([{"id":"s","name":"配置","url":"https://a.invalid/config.json","enabled":true,"raw":{"sites":[{"key":"a","name":"规则1","api":"https://a.invalid/search","rules":{"list":".a"}},{"key":"b","name":"规则2","api":"https://a.invalid/search","rules":{"list":".b"}}]}}])).unwrap();
    let sources = Sources::new(store).unwrap();
    assert_eq!(sources.sites().unwrap().len(), 2);
    sources.tombstone("siteKeys", "a", true).unwrap();
    assert_eq!(sources.sites().unwrap().len(), 1);
    sources.clear_tombstones().unwrap();
    let local = sources
        .add_local(&json!({"api":"https://a.invalid/search","name":"手动"}))
        .unwrap();
    assert_eq!(sources.sites().unwrap().len(), 1);
    assert_eq!(sources.sites().unwrap()[0]["origin"], "manual");
    sources
        .remove_local(local["key"].as_str().unwrap())
        .unwrap();
    assert_eq!(sources.sites().unwrap().len(), 2);
}

#[tokio::test]
async fn canceled_http_requests_return_promptly() {
    let started = Arc::new(Notify::new());
    let server = Server::new(Router::new().route(
        "/slow",
        get({
            let started = started.clone();
            move || {
                let started = started.clone();
                async move {
                    started.notify_one();
                    tokio::time::sleep(Duration::from_secs(20)).await;
                    "done"
                }
            }
        }),
    ))
    .await;
    let cancel = CancellationToken::new();
    let net = Net::new().unwrap();
    let address = format!("{}/slow", server.address);
    let task = tokio::spawn({
        let cancel = cancel.clone();
        async move { net.get(&address, &json!({}), 30.0, &cancel).await }
    });
    started.notified().await;
    let timer = Instant::now();
    cancel.cancel();
    assert!(
        task.await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("取消")
    );
    assert!(timer.elapsed() < Duration::from_secs(1));
}
