use axum::{Router, routing::get};
use base64::Engine;
use crayon_core::{
    ai,
    catalogue::Catalogue,
    live::{self, Live},
    media::Media,
    model::{array, s},
    player, presets,
    sources::Sources,
    storage::Store,
};
use serde_json::json;
use std::{sync::Arc, time::Instant};
use tokio_util::sync::CancellationToken;

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
fn live_formats_merge_lines_and_keep_builtin_room() {
    let mut channels = live::parse(
        "•游戏「赛事」,#genre#\n频道 A,https://a.invalid/a.m3u8\n频道 A,https://a.invalid/a.m3u8\n频道 A,https://b.invalid/b.m3u8\n其它,#genre#\n保留参数,https://a.invalid/live#key",
    );
    channels.extend(live::parse("#EXTM3U\n#EXTINF:-1 tvg-name=\"频道 B\" group-title=\"游戏赛事\" tvg-logo=\"https://a.invalid/b.png\",标签\nhttps://a.invalid/b.m3u8"));
    let groups = live::merge_channels(&channels);
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0]["name"], "游戏赛事");
    let group = array(&groups[0]["channels"]);
    assert_eq!(array(&group[0]["urls"]).len(), 2);
    assert_eq!(
        groups[1]["channels"][0]["urls"][0]["url"],
        "https://a.invalid/live#key"
    );
    assert_eq!(live::parse(live::BUILTIN)[0]["url"], "huya://10188");
    assert!(live::huya_lines(&json!({"data":{"liveStatus":"REPLAY"}})).is_err());
    let douyin=live::douyin_result(&json!({"data":{"data":[{"status":2,"stream_url":{"hls_pull_url_map":{"SD1":"low","HD1":"high"}}}]}})).unwrap();
    assert_eq!(douyin["url"], "high");
}

#[tokio::test]
async fn live_loads_uppercase_http_sources_and_preserves_normalized_metadata() {
    let server = Server::new(Router::new().route(
        "/channels",
        get(|| async { "验收分组,#genre#\n测试频道,HTTP://media.invalid/live.m3u8" }),
    ))
    .await;
    let folder = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(folder.path()).unwrap());
    store.write("sources", &json!([{
        "id":"s","name":"来源","enabled":true,"url":server.address,
        "raw":{"lives":[{"url":format!("{}/channels",server.address).replacen("http:", "HTTP:", 1)}]}
    }])).unwrap();
    let live = Live::new(Arc::new(
        Media::new(Arc::new(Catalogue::new(Arc::new(
            Sources::new(store).unwrap(),
        ))))
        .unwrap(),
    ));
    let loaded = live.load(false).await.unwrap();
    assert!(loaded["sources"][1]["error"].is_null());
    assert_eq!(loaded["sources"][1]["name"], "直播源");
    assert_eq!(loaded["sources"][1]["channels"], 1);
    let group = array(&loaded["groups"])
        .into_iter()
        .find(|group| group["name"] == "验收分组")
        .unwrap();
    assert_eq!(
        group["channels"][0]["urls"][0]["url"],
        "HTTP://media.invalid/live.m3u8"
    );
}

#[tokio::test]
async fn live_probes_limit_stream_read_and_store_results() {
    let server=Server::new(Router::new().route("/live",get(||async{axum::response::Response::builder().header("content-type","application/vnd.apple.mpegurl").body(axum::body::Body::from_stream(futures_util::stream::once(async{Ok::<_,std::io::Error>("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=2500000,RESOLUTION=1920x1080\nmain.m3u8\n".to_owned()+&" ".repeat(4096))}).chain(futures_util::stream::pending()))).unwrap()})).route("/bad",get(||async{(axum::http::StatusCode::FORBIDDEN,"denied")}))).await;
    let folder = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(folder.path()).unwrap());
    let live = Live::new(Arc::new(
        Media::new(Arc::new(Catalogue::new(Arc::new(
            Sources::new(store).unwrap(),
        ))))
        .unwrap(),
    ));
    let good = format!("{}/live", server.address).replacen("http:", "HTTP:", 1);
    let bad = format!("{}/bad", server.address);
    let started = Instant::now();
    let health = live
        .probe(
            &[
                good.clone(),
                good.clone(),
                bad.clone(),
                "huya://10188".into(),
            ],
            false,
        )
        .await
        .unwrap();
    assert!(started.elapsed().as_secs() < 3);
    assert_eq!(health.as_object().unwrap().len(), 2);
    assert_eq!(health[&good]["height"], 1080);
    assert_eq!(health[&bad]["ok"], false);
    assert!(live::fresh(&health[&good]));
    assert!(s(&health[&bad], "error").contains("403"));
    live.clear_health().unwrap();
    assert_eq!(live.health().unwrap(), json!({}));
}

#[tokio::test]
async fn ai_uses_private_headers_and_keeps_episode_and_frame_contract() {
    let server=Server::new(Router::new().route("/chat/completions",axum::routing::post(|headers:axum::http::HeaderMap,axum::Json(body):axum::Json<serde_json::Value>|async move{
        assert_eq!(headers["authorization"],"Bearer fixture-key");assert_eq!(body["model"],"fixture-model");assert_eq!(body["thinking"]["type"],"disabled");assert!(!body.to_string().contains("fixture-key"));assert!(body["messages"][1]["content"].as_str().unwrap().contains("第 2 集"));axum::Json(json!({"choices":[{"message":{"content":"本集内容"}}],"usage":{"total_tokens":12}}))
    }))).await;
    let folder = tempfile::tempdir().unwrap();
    let store = Store::open(folder.path()).unwrap();
    store.save_settings(&json!({"ai":{"baseUrl":server.address,"apiKey":"fixture-key","model":"fixture-model"}})).unwrap();
    let input = json!({"vodName":"测试影片","epName":"第二集","epIndex":1,"epTotal":8});
    let result = ai::summarize(
        &store,
        &crayon_core::http::Net::new().unwrap(),
        &input,
        vec![],
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(result["text"], "本集内容");
    assert_eq!(result["usage"]["total"], 12.0);
    let frames =
        vec![json!({"t":65,"b64":base64::engine::general_purpose::STANDARD.encode([1;65])}); 150];
    let blocks = ai::visual_blocks(&input, &frames).unwrap();
    assert_eq!(blocks.len(), 282);
    assert_eq!(blocks[2]["text"], "[01:05]");
    assert!(
        blocks[3]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:")
    );
    assert!(ai::visual_blocks(&input, &[json!({"b64":"invalid data"})]).is_err());
}

#[tokio::test]
async fn deleted_presets_do_not_resurrect_and_player_args_never_use_a_shell() {
    let folder = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(folder.path()).unwrap());
    let sources = Sources::new(store.clone()).unwrap();
    let source = presets::add(&sources, "public-cms").unwrap();
    sources.sync(&s(&source, "id")).await.unwrap();
    assert_eq!(sources.sites().unwrap().len(), 5);
    sources.remove(&s(&source, "id")).unwrap();
    store.write("bundled-sources", &json!(true)).unwrap();
    store.write("bundled-remote-sources", &json!(true)).unwrap();
    presets::ensure(&sources).await.unwrap();
    assert!(sources.list().unwrap().is_empty());
    let address = "https://a.invalid/video?x=$1&y=hello world";
    let args = player::arguments(
        &[json!("--title={TITLE}"), json!("{URL}")],
        address,
        "片名 $1 ; calc.exe",
    )
    .unwrap();
    assert_eq!(args, vec!["--title=片名 $1 ; calc.exe", address]);
    assert!(player::arguments(&[], "javascript:alert(1)", "").is_err());
}

use futures_util::StreamExt;

#[test]
fn source_detection_and_frame_sampling_keep_the_original_boundaries() {
    use crayon_core::{config::detect, media::is_hls};
    for (body, kind) in [
        ("#EXTM3U\n", "m3u"),
        ("<rss><class/></rss>", "cms"),
        (r#"{"sites":[] }"#, "tvbox"),
        (r#"{"urls":[{"url":"https://a.invalid/"}]}"#, "multirepo"),
        (r#"{"list":[]}"#, "cms"),
        ("<!doctype html><html></html>", "html"),
        ("2423abcdef0123456789abcdef", "encrypted"),
        ("unrecognized", "unknown"),
    ] {
        assert_eq!(detect(body.as_bytes(), "")["kind"], kind);
    }
    assert_eq!(detect(&[0xff, 0xd8, 0xff], "")["kind"], "image");
    assert!(is_hls("https://a.invalid/MAIN.M3U8?token=1#fragment"));
    assert!(is_hls("https://a.invalid/hls/main"));
    assert!(!is_hls("https://a.invalid/video.mp4?next=.m3u8"));
    assert!(ai::plan_frames(f64::INFINITY).is_empty());
    assert!(ai::plan_frames(1.0).is_empty());
    assert_eq!(ai::plan_frames(2.0), vec![2.0]);
    assert_eq!(ai::plan_frames(65.0), vec![2.0, 22.0, 42.0, 62.0]);
    let frames = ai::plan_frames(10_000.0);
    assert!(frames.len() <= 120);
    assert!(frames.windows(2).all(|v| v[1] > v[0]));
    assert!(frames.last().copied().unwrap() < 9999.0);
}
