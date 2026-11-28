use axum::{
    Router,
    body::Body,
    http::{HeaderMap, StatusCode, Uri},
    response::IntoResponse,
    routing::get,
};
use crayon_core::{model::s, sources::Sources, storage::Store, websites::animoe};
use futures_util::{StreamExt, stream};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

// Semantic fixtures follow the published Animoe rules and the 2025-10-19
// homepage snapshot; they are not a claim that the live site is reachable.
struct Fixture {
    address: String,
    sources: Sources,
    site: Value,
    calls: Arc<Mutex<Vec<String>>>,
    media_requests: Arc<Mutex<Vec<HeaderMap>>>,
    task: tokio::task::JoinHandle<()>,
    _folder: tempfile::TempDir,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn new() -> Self {
        Self::with_redirect(None).await
    }
    async fn with_redirect(redirect_target: Option<String>) -> Self {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let media_requests = Arc::new(Mutex::new(Vec::new()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new().fallback(get({
            let calls = calls.clone();
            let media_requests = media_requests.clone();
            let redirect_target = redirect_target.clone();
            move |uri: Uri, headers: HeaderMap| {
                let calls = calls.clone();
                let media_requests = media_requests.clone();
                let redirect_target = redirect_target.clone();
                async move {
                    calls.lock().unwrap().push(uri.to_string());
                    if uri.path().starts_with("/media/") {
                        media_requests.lock().unwrap().push(headers.clone());
                    }
                    if uri.path() == "/media/redirect.m3u8" {
                        return axum::http::Response::builder()
                            .status(StatusCode::FOUND)
                            .header("Location", redirect_target.expect("redirect fixture target"))
                            .body(Body::empty())
                            .unwrap()
                            .into_response();
                    }
                    let base = format!("http://{}",headers["host"].to_str().unwrap());
                    if uri.path() == "/media/slow.m3u8" {
                        let first = stream::iter([Ok::<_,std::convert::Infallible>("#EXTM3U\n".to_owned())]);
                        let tail = stream::once(async {
                            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                            Ok::<_,std::convert::Infallible>("#EXT-X-ENDLIST\n".to_owned())
                        });
                        return ([("Content-Type","application/vnd.apple.mpegurl")],Body::from_stream(first.chain(tail))).into_response();
                    }
                    let (status,mime,body) = if headers.contains_key("x-challenge") {
                        (StatusCode::FORBIDDEN,"text/html","<html><title>Just a moment...</title><script>window._cf_chl_opt={};</script></html>".to_owned())
                    } else {
                        let path = uri.path();
                        let mut body = match path {
                            "/" => r#"<html><a href="/type/20.html" title="连载中">连载中</a><a href="/type/21.html" title="已完结">已完结</a><a href="/type/23.html" title="剧场版">剧场版</a><a href="/info/264.html" title="地狱乐"><img data-original="/cover.jpg"></a><a href="/info/264.html" title="地狱乐">地狱乐</a><a href="/info/226.html" title="尼尔：自动人形 Ver1.1a">尼尔</a></html>"#.to_owned(),
                            "/index.php/ajax/suggest" => {
                                assert_eq!(headers["x-requested-with"],"XMLHttpRequest");
                                assert_eq!(headers["referer"],format!("{base}/"));
                                let query: std::collections::HashMap<_,_> = url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes()).into_owned().collect();
                                if query.get("wd").is_some_and(|v|v=="无结果") { json!({"code":0,"list":[]}).to_string() }
                                else {
                                    assert_eq!(query.get("wd").map(String::as_str),Some("地狱乐 & 尼尔"));
                                    json!({"code":1,"list":[{"id":264,"name":"地狱乐","pic":"/cover.jpg"}]}).to_string()
                                }
                            },
                            "/type/20.html" => r#"<html><a href="/info/264.html" title="地狱乐">地狱乐</a><a href="/show/from-real-link/page-two.html">2</a></html>"#.to_owned(),
                            "/show/from-real-link/page-two.html" => r#"<html><a href="/info/226.html" title="尼尔">尼尔</a><a href="/show/from-real-link/page-two.html">2</a></html>"#.to_owned(),
                            "/type/99.html" => r#"<a href="/info/264.html" title="地狱乐">地狱乐</a><a href="/type/window-2.html">2</a>"#.to_owned(),
                            "/type/window-2.html" => r#"<a href="/info/264.html" title="地狱乐">地狱乐</a><a href="/type/window-3.html">3</a>"#.to_owned(),
                            "/type/window-3.html" => r#"<a href="/info/264.html" title="地狱乐">地狱乐</a><a href="/type/window-4.html">4</a>"#.to_owned(),
                            "/type/window-4.html" => r#"<a href="/info/226.html" title="尼尔">尼尔</a>"#.to_owned(),
                            "/type/98.html" => r#"<a href="/type/98.html" rel="next">下一页</a>"#.to_owned(),
                            "/info/264.html" => r#"<html><h1>地狱乐</h1><meta property="og:image" content="/cover.jpg"><meta name="description" content="测试简介"><div class="module-tab-item" data-dropdown-value="字幕组A"></div><div class="module-tab-item" data-dropdown-value="字幕组B"></div><div class="module-play-list"><a class="module-play-list-link" href="/play/264-1-1.html">第1集</a><a href="/play/264-1-2.html">第2集</a></div><div class="module-play-list"><a href="/play/264-2-1.html">01</a></div></html>"#.to_owned(),
                            "/media/good.m3u8" => "#EXTM3U\n#EXT-X-TARGETDURATION:8\n#EXTINF:8,\nclip.ts\n#EXT-X-ENDLIST\n".to_owned(),
                            "/media/bad.m3u8" => "<html><title>HTTP 200 error page</title></html>".to_owned(),
                            "/media/encoded.m3u8" => "enc!opaque bytes".to_owned(),
                            "/play/264-1-1.html" | "/play/264-1-2.html" | "/play/264-2-1.html" | "/play/slow.html" | "/play/redirect.html" => {
                                let media = if path.ends_with("slow.html") {"slow"} else if path.ends_with("redirect.html") {"redirect"} else if path.ends_with("1-2.html") {"bad"} else if path.ends_with("2-1.html") {"encoded"} else {"good"};
                                format!("<html><script>var player_aaaa = {};document.title='ready';</script></html>",json!({"url":format!("{base}/media/{media}.m3u8"),"extra":{"keep":true}}))
                            },
                            _ => "<html></html>".to_owned(),
                        };
                        if path == "/" {
                            body.push_str(r#"<a href="https://foreign.invalid/type/999.html" title="外域分类">外域分类</a><a href="https://foreign.invalid/info/999.html" title="外域作品">外域作品</a>"#);
                        }
                        if path == "/info/264.html" {
                            body = body.replace(r#"<div class="module-play-list">"#,r#"<div class="module-play-list"><a href="https://foreign.invalid/play/999.html">外域分集</a>"#);
                        }
                        (StatusCode::OK,if path=="/index.php/ajax/suggest" {"application/json"} else if path.starts_with("/media/") {"application/vnd.apple.mpegurl"} else {"text/html"},body)
                    };
                    (status,[("Content-Type",mime)],body).into_response()
                }
            }
        }));
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let folder = tempfile::tempdir().unwrap();
        let sources = Sources::new(Arc::new(Store::open(folder.path()).unwrap())).unwrap();
        let site = json!({"key":"animoe","name":"Animoe","type":4,"api":format!("{address}/"),"timeout":2});
        Self {
            address,
            sources,
            site,
            calls,
            media_requests,
            task,
            _folder: folder,
        }
    }
    async fn request(&self, query: Value) -> anyhow::Result<Value> {
        animoe::request(&self.sources, &self.site, &query, &CancellationToken::new()).await
    }
    async fn resolve(&self, path: &str) -> anyhow::Result<Value> {
        animoe::resolve(
            &self.sources,
            &self.site,
            &format!("{}{path}", self.address),
            &CancellationToken::new(),
        )
        .await
    }
}

#[tokio::test]
async fn homepage_and_classes_follow_documented_links() {
    let fixture = Fixture::new().await;
    let classes = fixture.request(json!({"ac":"list"})).await.unwrap();
    assert_eq!(classes["class"].as_array().unwrap().len(), 4);
    assert_eq!(classes["class"][0]["type_id"], "anime");
    assert_eq!(classes["class"][1]["type_id"], "20");
    let mapped: Vec<_> = classes["class"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| json!({"id":v["type_id"],"name":v["type_name"],"pid":v["type_pid"]}))
        .collect();
    let leaves = crayon_core::category::leaves(&mapped);
    assert_eq!(leaves.len(), 3);
    assert!(leaves.iter().all(|v| v["bucket"] == "动漫"));
    let home = fixture.request(json!({"ac":"detail"})).await.unwrap();
    assert_eq!(home["list"].as_array().unwrap().len(), 2);
    assert_eq!(home["list"][0]["vod_name"], "地狱乐");
    assert_eq!(
        home["list"][0]["vod_pic"],
        format!("{}/cover.jpg", fixture.address)
    );
}

#[tokio::test]
async fn search_encodes_keyword_and_accepts_empty_results() {
    let fixture = Fixture::new().await;
    let found = fixture
        .request(json!({"wd":"地狱乐 & 尼尔"}))
        .await
        .unwrap();
    assert_eq!(found["list"][0]["vod_name"], "地狱乐");
    assert_eq!(
        found["list"][0]["vod_id"],
        format!("{}/info/264.html", fixture.address)
    );
    let empty = fixture.request(json!({"wd":"无结果"})).await.unwrap();
    assert!(empty["list"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn category_follows_real_pagination_link_and_stops_at_end() {
    let fixture = Fixture::new().await;
    let second = fixture.request(json!({"t":"20","pg":2})).await.unwrap();
    assert_eq!(second["list"][0]["vod_name"], "尼尔");
    assert_eq!(second["pagecount"], 2);
    assert!(
        fixture
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|v| v == "/show/from-real-link/page-two.html")
    );
    let end = fixture.request(json!({"t":"20","pg":3})).await.unwrap();
    assert!(end["list"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn category_discovers_pages_beyond_the_first_pagination_window() {
    let fixture = Fixture::new().await;
    let fourth = fixture.request(json!({"t":"99","pg":4})).await.unwrap();
    assert_eq!(fourth["list"][0]["vod_name"], "尼尔");
    assert_eq!(fourth["pagecount"], 4);
    let calls = fixture.calls.lock().unwrap();
    for path in [
        "/type/window-2.html",
        "/type/window-3.html",
        "/type/window-4.html",
    ] {
        assert!(calls.iter().any(|v| v == path));
    }
}

#[tokio::test]
async fn cyclic_pagination_is_an_error_instead_of_a_false_end_page() {
    let fixture = Fixture::new().await;
    let error = fixture.request(json!({"t":"98","pg":2})).await.unwrap_err();
    assert!(error.to_string().contains("循环"));
}

#[tokio::test]
async fn foreign_origin_links_are_filtered_from_classes_lists_and_episodes() {
    let fixture = Fixture::new().await;
    for query in [
        json!({"ac":"list"}),
        json!({"ac":"detail"}),
        json!({"ids":"264"}),
    ] {
        let result = fixture.request(query).await.unwrap();
        assert!(!result.to_string().contains("foreign.invalid"));
        assert!(!result.to_string().contains("外域"));
    }
}

#[tokio::test]
async fn detail_preserves_lines_and_absolutizes_episodes() {
    let fixture = Fixture::new().await;
    let result = fixture.request(json!({"ids":"264"})).await.unwrap();
    let detail = &result["list"][0];
    assert_eq!(detail["vod_play_from"], "字幕组A$$$字幕组B");
    assert_eq!(detail["vod_content"], "测试简介");
    let lines =
        crayon_core::cms::parse_lines(&s(detail, "vod_play_from"), &s(detail, "vod_play_url"), "");
    assert_eq!(lines[0]["episodes"].as_array().unwrap().len(), 2);
    assert_eq!(
        lines[1]["episodes"][0]["url"],
        format!("{}/play/264-2-1.html", fixture.address)
    );
}

#[tokio::test]
async fn playback_requires_identifiable_media_instead_of_http_200() {
    let fixture = Fixture::new().await;
    let result = fixture.resolve("/play/264-1-1.html").await.unwrap();
    assert_eq!(result["probe"]["kind"], "hls");
    assert_eq!(result["probe"]["confidence"], "signature");
    assert!(result["header"].get("Range").is_none());
    let error = fixture.resolve("/play/264-1-2.html").await.unwrap_err();
    assert!(error.to_string().contains("未返回可识别媒体"));
}

#[tokio::test]
async fn media_rejects_cross_origin_redirect_before_forwarding_source_headers() {
    let external_requests = Arc::new(Mutex::new(Vec::<HeaderMap>::new()));
    let external_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let external_address = format!("http://{}", external_listener.local_addr().unwrap());
    let external_task = tokio::spawn({
        let external_requests = external_requests.clone();
        async move {
            let app = Router::new().fallback(get(move |headers: HeaderMap| {
                let external_requests = external_requests.clone();
                async move {
                    external_requests.lock().unwrap().push(headers);
                    (
                        StatusCode::OK,
                        [("Content-Type", "application/vnd.apple.mpegurl")],
                        "#EXTM3U\n#EXT-X-ENDLIST\n",
                    )
                        .into_response()
                }
            }));
            axum::serve(external_listener, app).await.unwrap();
        }
    });
    let mut fixture = Fixture::with_redirect(Some(format!("{external_address}/media.m3u8"))).await;
    fixture.site["header"] = json!({
        "X-Origin-Secret":"site-secret",
        "Cookie":"site-cookie=secret",
        "Authorization":"Bearer site-secret"
    });

    let error = fixture.resolve("/play/redirect.html").await.unwrap_err();
    assert!(error.to_string().contains("必须属于 Animoe 源"));
    let initial = fixture.media_requests.lock().unwrap();
    assert_eq!(initial.len(), 1);
    assert_eq!(initial[0]["x-origin-secret"], "site-secret");
    assert_eq!(initial[0]["cookie"], "site-cookie=secret");
    assert_eq!(initial[0]["authorization"], "Bearer site-secret");
    assert!(external_requests.lock().unwrap().is_empty());
    external_task.abort();
}

#[tokio::test]
async fn proprietary_playlist_format_is_an_explicit_error() {
    let fixture = Fixture::new().await;
    let error = fixture.resolve("/play/264-2-1.html").await.unwrap_err();
    assert!(error.to_string().contains("非标准封装"));
}

#[tokio::test]
async fn media_probe_timeout_also_covers_a_stalled_response_body() {
    let mut fixture = Fixture::new().await;
    fixture.site["timeout"] = json!(1);
    let started = std::time::Instant::now();
    let error = fixture.resolve("/play/slow.html").await.unwrap_err();
    let elapsed = started.elapsed();
    assert!(elapsed >= std::time::Duration::from_millis(800));
    assert!(elapsed < std::time::Duration::from_secs(3));
    assert!(format!("{error:#}").contains("超时") || format!("{error:#}").contains("timed out"));
}

#[tokio::test]
async fn challenge_is_reported_without_trying_to_parse_content() {
    let mut fixture = Fixture::new().await;
    fixture.site["header"] = json!({"X-Challenge":"1"});
    let error = fixture.request(json!({"wd":"地狱乐"})).await.unwrap_err();
    assert!(error.to_string().contains("Cloudflare"));
}

#[tokio::test]
async fn invalid_detail_and_cancelled_request_do_not_reach_network() {
    let fixture = Fixture::new().await;
    assert!(
        fixture
            .request(json!({"ids":"https://foreign.invalid/info/264.html"}))
            .await
            .is_err()
    );
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(
        animoe::request(
            &fixture.sources,
            &fixture.site,
            &json!({"ac":"list"}),
            &cancel
        )
        .await
        .is_err()
    );
    assert!(fixture.calls.lock().unwrap().is_empty());
}
