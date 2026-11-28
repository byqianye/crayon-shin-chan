use axum::{
    Router,
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Redirect},
    routing::get,
};
use crayon_core::{
    catalogue::Catalogue, category::leaves, media::Media, model::s, sources::Sources,
    storage::Store,
};
use serde_json::json;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

type Requests = Arc<Mutex<Vec<(String, HeaderMap)>>>;

// DNS is overridden only on this fixture's HTTP client, so the real host
// dispatcher is exercised without reaching Animoe or changing user settings.
struct Fixture {
    base: String,
    media_base: String,
    catalogue: Arc<Catalogue>,
    requests: Requests,
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
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socket = listener.local_addr().unwrap();
        let base = format!("http://animoe.org:{}/", socket.port());
        let media_base = format!("http://{socket}");
        let requests: Requests = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new().fallback(get({
            let requests = requests.clone();
            let media_base = media_base.clone();
            move |uri: Uri, headers: HeaderMap| {
                let requests = requests.clone();
                let media_base = media_base.clone();
                async move {
                    requests.lock().unwrap().push((uri.to_string(),headers.clone()));
                    if uri.path() == "/media/redirect.m3u8" {
                        return Redirect::temporary(&format!("{}/media/good.m3u8", media_base.replace("127.0.0.1", "localhost"))).into_response();
                    }
                    let (status,mime,body) = match uri.path() {
                        "/" if uri.query().is_none() => (StatusCode::OK,"text/html",r#"<html><a href="/type/20.html" title="连载中">连载中</a><a href="/info/264.html" title="地狱乐"><img data-original="/cover.jpg"></a></html>"#.to_owned()),
                        "/index.php/ajax/suggest" => {
                            let query: std::collections::HashMap<_,_> = url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes()).into_owned().collect();
                            assert_eq!(query.get("wd").map(String::as_str),Some("地狱乐"));
                            assert_eq!(headers["x-requested-with"],"XMLHttpRequest");
                            (StatusCode::OK,"application/json",json!({"code":1,"list":[{"id":264,"name":"地狱乐","pic":"/cover.jpg"}]}).to_string())
                        },
                        "/info/264.html" => (StatusCode::OK,"text/html",r#"<html><h1>地狱乐</h1><meta name="description" content="集成测试简介"><div class="module-tab-item" data-dropdown-value="字幕组"></div><div class="module-play-list"><a href="/play/264-1-1.html">第1集</a><a href="/play/264-1-2.html">第2集</a></div></html>"#.to_owned()),
                        "/play/264-1-1.html" | "/play/264-1-2.html" | "/play/264-1-3.html" => {
                            let media = if uri.path().ends_with("1-2.html") {"error"} else if uri.path().ends_with("1-3.html") {"redirect"} else {"good"};
                            (StatusCode::OK,"text/html",format!("<html><script>var player_aaaa = {};</script></html>",json!({"url":format!("{media_base}/media/{media}.m3u8")})))
                        },
                        "/media/good.m3u8" => (StatusCode::OK,"application/vnd.apple.mpegurl","#EXTM3U\n#EXT-X-TARGETDURATION:8\n#EXTINF:8,\nsegment.ts\n#EXT-X-ENDLIST\n".to_owned()),
                        "/media/error.m3u8" => (StatusCode::OK,"application/vnd.apple.mpegurl","<html><title>Access denied</title></html>".to_owned()),
                        _ => (StatusCode::NOT_FOUND,"text/plain","This fixture has no CMS endpoint".to_owned()),
                    };
                    (status,[("Content-Type",mime)],body).into_response()
                }
            }
        }));
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let folder = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(folder.path()).unwrap());
        store
            .write(
                "sources",
                &json!([{
                    "id":"website-fixture","name":"网站集成测试","enabled":true,
                    "url":format!("{base}fixture.json"),
                    "raw":{"headers":[{"host":socket.to_string(),"header":{"Cookie":"media=1","Authorization":"Bearer media-fixture"}}],"sites":[{
                        "key":"animoe","name":"Animoe","type":4,"api":base,"timeout":3,
                        "header":{"User-Agent":"WebsiteDispatchFixture/1.0","Referer":base,"Cookie":"site=1","Authorization":"Bearer site-fixture"}
                    }],"home":{"sections":[{"site":"animoe","title":"Animoe 最近更新"}]}}
                }]),
            )
            .unwrap();
        let mut sources = Sources::new(store).unwrap();
        sources.net.client = reqwest::Client::builder()
            .no_proxy()
            .resolve("animoe.org", socket)
            .build()
            .unwrap();
        let catalogue = Arc::new(Catalogue::new(Arc::new(sources)));
        Self {
            base,
            media_base,
            catalogue,
            requests,
            task,
            _folder: folder,
        }
    }

    fn calls(&self, path: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(uri, _)| uri == path)
            .count()
    }
}

#[tokio::test]
async fn website_dispatch_reaches_catalogue_cache_and_media_services() {
    let fixture = Fixture::new().await;
    let site = fixture.catalogue.sources.site("animoe").unwrap();
    assert_eq!(site["type"], 4);
    assert_eq!(site["api"], fixture.base);
    assert!(s(&site, "spiderClass").is_empty());
    let classes = fixture.catalogue.classes(&site).await.unwrap();
    assert!(
        classes
            .iter()
            .any(|v| v["id"] == "20" && v["pid"] == "anime")
    );
    assert_eq!(leaves(&classes)[0]["bucket"], "动漫");

    let home = fixture.catalogue.home(false).await.unwrap();
    assert_eq!(home.len(), 1);
    assert!(home[0].get("error").is_none(), "{}", home[0]);
    assert_eq!(home[0]["siteKey"], "animoe");
    assert_eq!(home[0]["items"][0]["vod_name"], "地狱乐");
    assert_eq!(home[0]["items"][0]["sourceId"], "website-fixture");
    let id = s(&home[0]["items"][0], "vod_id");
    assert_eq!(id, format!("{}info/264.html", fixture.base));

    let cancel = CancellationToken::new();
    let search = fixture
        .catalogue
        .search_site(&site, "地狱乐", 20, &cancel)
        .await;
    assert_eq!(search["ok"], true, "{search}");
    assert_eq!(search["list"][0]["siteKey"], "animoe");
    assert_eq!(search["list"][0]["vod_id"], id);

    let detail = fixture.catalogue.detail("animoe", &id).await.unwrap();
    assert_eq!(detail["siteKey"], "animoe");
    assert_eq!(detail["vod_content"], "集成测试简介");
    assert_eq!(detail["lines"][0]["name"], "字幕组");
    assert_eq!(detail["lines"][0]["episodes"].as_array().unwrap().len(), 2);
    assert_eq!(fixture.calls("/info/264.html"), 1);
    let count_before_repeat = fixture.requests.lock().unwrap().len();
    assert_eq!(
        fixture.catalogue.detail("animoe", &id).await.unwrap(),
        detail
    );
    assert_eq!(
        fixture.requests.lock().unwrap().len(),
        count_before_repeat,
        "repeat detail must use the application cache"
    );

    let media = Media::new(fixture.catalogue.clone()).unwrap();
    let episode = s(&detail["lines"][0]["episodes"][0], "url");
    let resolved = media
        .resolve(&episode, &json!({"siteKey":"animoe"}), &cancel)
        .await
        .unwrap();
    assert_eq!(resolved["ok"], true, "{resolved}");
    assert_eq!(
        resolved["url"],
        format!("{}/media/good.m3u8", fixture.media_base)
    );
    assert_eq!(resolved["parseName"], "Animoe");
    assert_eq!(resolved["header"]["Referer"], fixture.base);
    assert_eq!(
        resolved["header"]["User-Agent"],
        "WebsiteDispatchFixture/1.0"
    );
    assert!(resolved["header"].get("Range").is_none());
    let address = s(&resolved, "url");
    let probe = media.inspect(&address, &cancel).await.unwrap();
    assert_eq!(probe["ok"], true, "{probe}");
    assert_eq!(probe["kind"], "hls");
    assert_eq!(probe["confidence"], "signature");
    assert_eq!(probe["decodeVerified"], false);
    let granted_headers = media.headers(&address).unwrap();
    assert_eq!(granted_headers["Referer"], fixture.base);
    assert_eq!(granted_headers["User-Agent"], "WebsiteDispatchFixture/1.0");
    assert_eq!(granted_headers["Cookie"], "media=1");
    assert_eq!(granted_headers["Authorization"], "Bearer media-fixture");
    let requests = fixture.requests.lock().unwrap();
    let media_calls: Vec<_> = requests
        .iter()
        .filter(|(uri, _)| uri == "/media/good.m3u8")
        .collect();
    assert_eq!(media_calls.len(), 2);
    for (_, headers) in media_calls {
        assert_eq!(headers["range"], "bytes=0-4095");
        assert_eq!(headers["referer"], fixture.base);
        assert_eq!(headers["user-agent"], "WebsiteDispatchFixture/1.0");
        assert_eq!(headers["cookie"], "media=1");
        assert_eq!(headers["authorization"], "Bearer media-fixture");
    }
    let expected_host = fixture
        .base
        .trim_start_matches("http://")
        .trim_end_matches('/');
    assert!(
        requests
            .iter()
            .filter(|(uri, _)| !uri.starts_with("/media/"))
            .all(|(_, headers)| headers["host"] == expected_host)
    );
}

#[tokio::test]
async fn top_level_media_resolution_rejects_a_fake_hls_error_page() {
    let fixture = Fixture::new().await;
    let detail = fixture
        .catalogue
        .detail("animoe", &format!("{}info/264.html", fixture.base))
        .await
        .unwrap();
    let media = Media::new(fixture.catalogue.clone()).unwrap();
    let episode = s(&detail["lines"][0]["episodes"][1], "url");
    let error = media
        .resolve(
            &episode,
            &json!({"siteKey":"animoe"}),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("未返回可识别媒体"));
    assert_eq!(fixture.calls("/media/error.m3u8"), 1);
}

#[tokio::test]
async fn website_episodes_cannot_fall_through_to_generic_external_resolution() {
    let fixture = Fixture::new().await;
    let media = Media::new(fixture.catalogue.clone()).unwrap();
    for path in ["play/264-1-1.html", "media/good.mp4"] {
        let error = media
            .resolve(
                &format!("{}/{path}", fixture.media_base),
                &json!({"siteKey":"animoe"}),
                &CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("必须属于当前影视源"));
    }
    assert!(fixture.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn top_level_media_resolution_rejects_cross_origin_redirects() {
    let fixture = Fixture::new().await;
    let media = Media::new(fixture.catalogue.clone()).unwrap();
    let cancel = CancellationToken::new();
    let error = media
        .resolve(
            &format!("{}play/264-1-3.html", fixture.base),
            &json!({"siteKey":"animoe"}),
            &cancel,
        )
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Animoe 媒体重定向目标必须属于 Animoe 源"),
        "{error:#}"
    );
    assert_eq!(fixture.calls("/media/redirect.m3u8"), 1);
    assert_eq!(fixture.calls("/media/good.m3u8"), 0);
    let requests = fixture.requests.lock().unwrap();
    let initial = requests
        .iter()
        .find(|(path, _)| path == "/media/redirect.m3u8")
        .unwrap();
    assert_eq!(initial.1["cookie"], "media=1");
    assert_eq!(initial.1["authorization"], "Bearer media-fixture");
}
