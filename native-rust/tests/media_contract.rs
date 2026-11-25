use axum::{Router, routing::get};
use crayon_core::{
    catalogue::Catalogue,
    dedupe,
    media::{Media, extract_url, is_direct, parse_address, rewrite_playlist},
    model::{array, s},
    sources::Sources,
    storage::Store,
};
use serde_json::json;
use std::sync::Arc;

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
fn parser_ignores_images_scripts_and_accepts_nested_streams() {
    assert_eq!(
        crayon_core::media::redirect(
            "location.href='HTTPS://a.invalid/play'",
            "https://a.invalid/"
        ),
        Some("HTTPS://a.invalid/play".into())
    );
    let rewritten = rewrite_playlist(
        "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"HTTPS://a.invalid/key\"\nHTTP://a.invalid/1.ts",
        "https://a.invalid/live.m3u8",
        "http://127.0.0.1/proxy",
    );
    assert!(rewritten.contains("url=HTTPS%3A%2F%2Fa.invalid%2Fkey"));
    assert!(rewritten.contains("url=HTTP%3A%2F%2Fa.invalid%2F1.ts"));
    assert!(extract_url("<script src='https://a.invalid/wasm_exec.js'></script><img src='https://a.invalid/avatar.png'>",false).is_none());
    assert_eq!(
        extract_url(
            r#"{"data":{"url":"https://a.invalid/live/123?token=a"}}"#,
            false
        )
        .unwrap(),
        "https://a.invalid/live/123?token=a"
    );
    assert!(extract_url(r#"{"url":"https://a.invalid/page/123"}"#, true).is_none());
    assert_eq!(
        extract_url("const vid='https://a.invalid/1.m3u8?x=1&amp;y=2';", true).unwrap(),
        "https://a.invalid/1.m3u8?x=1&y=2"
    );
    assert_eq!(
        parse_address("https://jx.invalid/?url={url}", "https://a.invalid/中文").unwrap(),
        "https://jx.invalid/?url=https%3A%2F%2Fa.invalid%2F%E4%B8%AD%E6%96%87"
    );
    assert!(!is_direct("https://a.invalid/page"));
    assert!(is_direct("https://a.invalid/a.m3u8?token=1"));
}

#[test]
fn playlist_rewrites_variants_keys_maps_and_segments() {
    let playlist = "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"../key.bin\"\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXT-X-STREAM-INF:BANDWIDTH=100\nsub/low.m3u8\n#EXTINF:5,\n1.ts";
    let rewritten = rewrite_playlist(
        playlist,
        "https://a.invalid/live/main.m3u8",
        "http://127.0.0.1:1/token/media",
    );
    assert!(rewritten.contains("url=https%3A%2F%2Fa.invalid%2Fkey.bin"));
    assert!(rewritten.contains("url=https%3A%2F%2Fa.invalid%2Flive%2F1.ts"));
    assert!(rewritten.contains("url=https%3A%2F%2Fa.invalid%2Flive%2Fsub%2Flow.m3u8"));
}

#[tokio::test]
async fn proxy_streams_ranges_and_never_leaks_spider_headers_to_another_origin() {
    let destination = Server::new(Router::new().route(
        "/stream.mp4",
        get(|headers: axum::http::HeaderMap| async move {
            assert!(headers.get("authorization").is_none());
            assert!(headers.get("cookie").is_none());
            assert_eq!(headers["range"], "bytes=2-4");
            (
                axum::http::StatusCode::PARTIAL_CONTENT,
                [
                    ("content-type", "video/mp4"),
                    ("content-range", "bytes 2-4/10"),
                ],
                "234",
            )
        }),
    ))
    .await;
    let destination_url = format!("{}/stream.mp4", destination.address);
    let source = Server::new(
        Router::new()
            .route(
                "/redirect",
                get(move |headers: axum::http::HeaderMap| {
                    let destination_url = destination_url.clone();
                    async move {
                        assert_eq!(headers["authorization"], "Bearer fixture");
                        assert_eq!(headers["cookie"], "session=fixture");
                        axum::response::Redirect::temporary(&destination_url)
                    }
                }),
            )
            .route(
                "/index.m3u8",
                get(|| async {
                    (
                        [("content-type", "application/vnd.apple.mpegurl")],
                        "#EXTM3U\n#EXTINF:5,\na.ts",
                    )
                }),
            ),
    )
    .await;
    let folder = tempfile::tempdir().unwrap();
    let catalogue = Arc::new(Catalogue::new(Arc::new(
        Sources::new(Arc::new(Store::open(folder.path()).unwrap())).unwrap(),
    )));
    let media = Arc::new(Media::new(catalogue).unwrap());
    media.register(
        &source.address,
        &json!({"Authorization":"Bearer fixture","Cookie":"session=fixture","X-Invalid":"denied"}),
    );
    let proxy = media.start_proxy().await.unwrap();
    let client = reqwest::Client::new();
    let response = client
        .get(proxy.url(&format!("{}/redirect", source.address)))
        .header("Range", "bytes=2-4")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 206);
    assert_eq!(response.headers()["content-range"], "bytes 2-4/10");
    assert_eq!(response.text().await.unwrap(), "234");
    let playlist = client
        .get(proxy.url(&format!("{}/index.m3u8", source.address)))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(playlist.contains(&proxy.base));
    let address = proxy.base.clone();
    drop(proxy);
    tokio::task::yield_now().await;
    assert!(client.get(address).send().await.is_err());
}

#[test]
fn dedupe_keeps_versions_seasons_and_unknown_type_identity() {
    let vod = |name: &str, year: &str, kind: &str, site: &str| json!({"vod_id":site,"vod_name":name,"vod_year":year,"type_name":kind,"siteKey":site,"vod_pic":"","vod_remarks":""});
    let result = dedupe::merge_results(
        &[
            json!({"list":[vod("庆余年第二季[全36集]","2024","国产剧","a"),vod("庆余年 第二季","","","b"),vod("庆余年第一季","2019","国产剧","c"),vod("庆余年第二季[电影解说]","2024","","d"),vod("射雕英雄传","1983","电视剧","e"),vod("射雕英雄传","2017","电视剧","f") ]}),
        ],
        "庆余年",
        true,
        None,
    );
    assert_eq!(result["hidden"], 1);
    assert_eq!(array(&result["merged"]).len(), 4);
    assert_eq!(dedupe::season("第二十季"), "20");
    assert_eq!(dedupe::season("Season 02"), "2");
    assert!(
        array(&result["merged"])
            .iter()
            .any(|v| array(&v["allSiteKeys"]).len() == 2)
    );
}

#[test]
fn merge_large_distinct_catalogues_uses_bounded_candidate_groups() {
    for year in ["", "2024"] {
        let list:Vec<_>=(0..1200).map(|i|json!({"vod_id":i.to_string(),"siteKey":"fixture","vod_name":format!("独立电影编号{i}"),"vod_year":year,"type_name":"电影"})).collect();
        let merged = dedupe::merge_results(&[json!({"list":list})], "", true, None);
        assert_eq!(array(&merged["merged"]).len(), 1200);
    }
    assert!(!s(&json!({"name":"中文"}), "name").is_empty());
}
