use axum::{
    Router,
    body::{Body, Bytes},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::IntoResponse,
    routing::any,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use crayon_core::{model::s, sources::Sources, storage::Store, websites::kankanya};
use futures_util::{StreamExt, stream};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

const ABC: &str = "PXhw7UT1B0a9kQDKZsjIASmOezxYG4CHo5Jyfg2b8FLpEvRr3WtVnlqMidu6cN";

fn mode2(value: &str) -> String {
    let abc: Vec<_> = ABC.chars().collect();
    let encoded: String = value
        .chars()
        .flat_map(|c| {
            [
                'P',
                abc.iter()
                    .position(|v| *v == c)
                    .map(|i| abc[(i + 3) % 62])
                    .unwrap_or(c),
                'P',
            ]
        })
        .collect();
    STANDARD.encode(encoded)
}
fn mode1(value: &str) -> String {
    let target: Vec<_> = ('a'..='z').map(|v| v.to_string()).collect();
    let lookup: Vec<_> = target.iter().cycle().skip(7).take(26).cloned().collect();
    let encoded: String = value
        .chars()
        .map(|c| {
            target
                .iter()
                .position(|v| v.starts_with(c))
                .map(|i| lookup[i].chars().next().unwrap())
                .unwrap_or(c)
        })
        .collect();
    let plain = format!(
        "{}/{}/{}",
        STANDARD.encode(serde_json::to_vec(&target).unwrap()),
        STANDARD.encode(serde_json::to_vec(&lookup).unwrap()),
        STANDARD.encode(encoded)
    );
    let inner = STANDARD.encode(plain);
    let key = format!("{:x}", md5::compute(b"test"));
    let raw: Vec<_> = inner
        .bytes()
        .enumerate()
        .map(|(i, v)| v ^ key.as_bytes()[i % key.len()])
        .collect();
    STANDARD.encode(raw)
}
fn gate_decode(value: &str) -> String {
    let abc: Vec<_> = ABC.chars().collect();
    String::from_utf8(STANDARD.decode(value).unwrap())
        .unwrap()
        .chars()
        .skip(1)
        .step_by(3)
        .map(|c| {
            abc.iter()
                .position(|v| *v == c)
                .map(|i| abc[(i + 59) % 62])
                .unwrap_or(c)
        })
        .collect()
}
fn card(id: u32, name: &str) -> String {
    format!(
        r#"<a class="module-poster-item module-item" title="{name}" href="/detail/{id}.html"><img data-original="/cover.jpg" src="/loading.gif" alt="{name}"><div class="module-item-note">36集全</div></a>"#
    )
}
struct Fixture {
    address: String,
    sources: Sources,
    site: Value,
    calls: Arc<Mutex<Vec<String>>>,
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
        let calls = Arc::new(Mutex::new(Vec::new()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new().fallback(any({
            let calls = calls.clone();
            move |method: Method, uri: Uri, headers: HeaderMap, body: Bytes| {
                let calls = calls.clone();
                async move {
                    calls.lock().unwrap().push(format!("{method} {uri}"));
                    let base = format!("http://{}",headers["host"].to_str().unwrap());
                    if uri.path().starts_with("/media/") {
                        assert_eq!(headers["range"],"bytes=0-4095");
                        if base.contains("127.0.0.1") {
                            let cookie=headers["cookie"].to_str().unwrap();
                            assert!(cookie.contains("gate=ready") && cookie.contains("configured=value"));
                            assert_eq!(headers["authorization"],"fixture-credential");
                        } else {
                            assert!(headers.get("cookie").is_none(),"site cookies must not leak to the CDN");
                            assert!(headers.get("authorization").is_none(),"site credentials must not leak to the CDN");
                        }
                        if uri.path()=="/media/redirect" {
                            return (StatusCode::FOUND,[("Location",format!("{}/media/good",base.replacen("127.0.0.1","localhost",1)))]).into_response();
                        }
                        if uri.path()=="/media/slow" {
                            let first=stream::iter([Ok::<_,std::convert::Infallible>(vec![0,0,0,32,b'f',b't',b'y',b'p',b'i',b's',b'o',b'm'])]);
                            let tail=stream::once(async { tokio::time::sleep(Duration::from_secs(10)).await; Ok::<_,std::convert::Infallible>(vec![0;4096]) });
                            return ([("Content-Type","video/mp4")],Body::from_stream(first.chain(tail))).into_response();
                        }
                        if uri.path()=="/media/bad" {
                            return ([("Content-Type","video/mp4")],"<html><title>HTTP 200 error</title></html>").into_response();
                        }
                        let mut media=vec![0,0,0,32,b'f',b't',b'y',b'p',b'i',b's',b'o',b'm'];media.resize(4096,0);
                        return (StatusCode::PARTIAL_CONTENT,[("Content-Type","video/mp4")],media).into_response();
                    }
                    if uri.path()=="/robot.php" {
                        assert_eq!(method,Method::POST);
                        assert_eq!(headers["content-type"],"application/x-www-form-urlencoded");
                        let data:HashMap<_,_>=url::form_urlencoded::parse(&body).into_owned().collect();
                        assert_eq!(gate_decode(&data["token"]),"dynamic-token");
                        assert!(gate_decode(&data["value"]).starts_with(&base));
                        if headers.contains_key("x-reject") { return axum::Json(json!({"msg":"denied"})).into_response(); }
                        return ([("Set-Cookie","gate=ready; Path=/; HttpOnly")],axum::Json(json!({"msg":"ok"}))).into_response();
                    }
                    let authenticated=headers.get("cookie").and_then(|v|v.to_str().ok()).is_some_and(|v|v.contains("gate=ready"));
                    if !authenticated {
                        let script=if headers.contains_key("x-changed-gate") {"var token = unknown('dynamic-token');"} else {"var token = encrypt(\"dynamic-token\");"};
                        return format!("<html><script>{script}request.post('/robot.php');</script></html>").into_response();
                    }
                    if uri.path()=="/yaplay/api.php" {
                        assert_eq!(method,Method::POST);
                        let data:HashMap<_,_>=url::form_urlencoded::parse(&body).into_owned().collect();
                        let vid=data["vid"].as_str();
                        if vid=="api-fail" { return axum::Json(json!({"code":500,"message":"failed"})).into_response(); }
                        let mode=if vid=="good1" {1} else {2};
                        let media=if ["same-origin","redirect"].contains(&vid) {base.clone()} else {base.replacen("127.0.0.1","localhost",1)}
                            +match vid {"bad"=>"/media/bad","slow"=>"/media/slow","redirect"=>"/media/redirect",_=>"/media/good?token=A/B+"};
                        let encoded=if vid=="malformed" {"invalid".to_owned()} else if mode==1 {mode1(&media)} else {mode2(&media)};
                        return axum::Json(json!({"code":200,"data":{"url":encoded,"urlmode":mode}})).into_response();
                    }
                    let result=match uri.path() {
                        "/"=>format!(r#"<html><div class="navbar-item"><a href="/type/1.html">电影</a><a href="/type/2.html">剧集</a><a href="https://outside.invalid/type/3.html">外链</a></div>{}{}</html>"#,card(106162,"庆余年 第二季"),card(106162,"重复作品")),
                        "/show/2--------1---.html"|"/show/2--------2---.html"=>format!(r#"<html>{}<a class="page-link" title="尾页" href="/show/2--------9---.html">尾页</a></html>"#,card(if uri.path().contains("--------2") {61727} else {106162},"庆余年")),
                        "/search/-------------.html"=>{
                            let query:HashMap<_,_>=url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes()).into_owned().collect();
                            if query["wd"]=="无结果" {"<html><div class=nofound>没有找到与 无结果 相关的影片</div><div class=module-prompt-info>可尝试缩短关键词试试</div></html>".to_owned()}
                            else if query["wd"]=="维护" {"<html><title>维护页面</title></html>".to_owned()}
                            else {
                                assert_eq!(query["wd"],"庆余年 & 第二季");
                                r#"<html><div class="module-card-item module-item"><a class="module-card-item-poster" href="/detail/106162.html"><img data-original="/cover.jpg" alt="庆余年 第二季"><div class="module-item-note">36集全</div></a><a href="/play/106162-1-1.html">播放</a></div></html>"#.to_owned()
                            }
                        },
                        "/detail/106162.html"=>r#"<html><div class="module-info-heading"><h1>庆余年 第二季</h1></div><div class="module-info-poster"><img data-original="/cover.jpg"></div><div class="module-info-introduction-content">测试简介</div><div class="module-info-tag-link"><a>2024</a><a>中国大陆</a><a>剧情</a></div><div class="module-info-item"><span class="module-info-item-title">导演：</span><div class="module-info-item-content">孙皓</div></div><div class="module-tab-item" data-dropdown-value="自营c"></div><div class="module-tab-item" data-dropdown-value="自营c"></div><div class="module-play-list-content"><a class="module-play-list-link" href="/play/good2.html"><span>第01集</span></a><a class="module-play-list-link" href="/play/good1.html">第02集</a><a class="module-play-list-link" href="https://outside.invalid/play/3.html">外链</a></div><div class="module-play-list-content"><a class="module-play-list-link" href="/play/good2.html">01</a></div></html>"#.to_owned(),
                        path if path.starts_with("/play/")=>{
                            let vid=path.strip_prefix("/play/").unwrap().trim_end_matches(".html");
                            format!("<html><script>var player_aaaa={};document.title='done';</script></html>",json!({"url":vid,"encrypt":0,"nested":{"allowed":true}}))
                        },
                        _=>"<html></html>".to_owned(),
                    };
                    ([("Content-Type","text/html; charset=utf-8")],result).into_response()
                }
            }
        }));
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let folder = tempfile::tempdir().unwrap();
        let sources = Sources::new(Arc::new(Store::open(folder.path()).unwrap())).unwrap();
        let site = json!({"key":"kankanya","type":4,"api":format!("{address}/"),"timeout":2,
            "header":{"Cookie":"configured=value","Authorization":"fixture-credential"}});
        Self {
            address,
            sources,
            site,
            calls,
            task,
            _folder: folder,
        }
    }
    async fn request(&self, query: Value) -> anyhow::Result<Value> {
        kankanya::request(&self.sources, &self.site, &query, &CancellationToken::new()).await
    }
    async fn resolve(&self, name: &str) -> anyhow::Result<Value> {
        kankanya::resolve(
            &self.sources,
            &self.site,
            &format!("{}/play/{name}.html", self.address),
            &CancellationToken::new(),
        )
        .await
    }
}

#[tokio::test]
async fn public_check_session_is_reused_and_homepage_is_cached() {
    let fixture = Fixture::new().await;
    let classes = fixture.request(json!({"ac":"list"})).await.unwrap();
    assert_eq!(classes["class"].as_array().unwrap().len(), 2);
    assert_eq!(classes["class"][0]["type_id"], "1");
    let mapped: Vec<_> = classes["class"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| json!({"id":v["type_id"],"name":v["type_name"],"pid":v["type_pid"]}))
        .collect();
    assert_eq!(crayon_core::category::leaves(&mapped).len(), 2);
    let home = fixture.request(json!({"ac":"detail"})).await.unwrap();
    assert_eq!(home["list"].as_array().unwrap().len(), 1);
    assert_eq!(home["list"][0]["vod_name"], "庆余年 第二季");
    assert_eq!(
        home["list"][0]["vod_pic"],
        format!("{}/cover.jpg", fixture.address)
    );
    fixture.request(json!({"ids":"106162"})).await.unwrap();
    let calls = fixture.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|v| v.as_str() == "POST /robot.php")
            .count(),
        1
    );
    assert_eq!(calls.iter().filter(|v| v.as_str() == "GET /").count(), 2);
}
#[tokio::test]
async fn search_and_pagination_follow_the_observed_routes() {
    let fixture = Fixture::new().await;
    let found = fixture
        .request(json!({"wd":"庆余年 & 第二季"}))
        .await
        .unwrap();
    assert_eq!(
        found["list"][0]["vod_id"],
        format!("{}/detail/106162.html", fixture.address)
    );
    assert_eq!(found["list"][0]["vod_remarks"], "36集全");
    assert!(
        fixture.request(json!({"wd":"无结果"})).await.unwrap()["list"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let second = fixture.request(json!({"t":"2","pg":2})).await.unwrap();
    assert_eq!(second["page"], 2);
    assert_eq!(second["pagecount"], 9);
    assert_eq!(
        second["list"][0]["vod_id"],
        format!("{}/detail/61727.html", fixture.address)
    );
}
#[tokio::test]
async fn detail_preserves_duplicate_line_names_and_absolute_episode_urls() {
    let fixture = Fixture::new().await;
    let data = fixture.request(json!({"ids":"106162"})).await.unwrap();
    let detail = &data["list"][0];
    assert_eq!(detail["vod_play_from"], "自营c$$$自营c");
    assert_eq!(detail["vod_content"], "测试简介");
    assert_eq!(detail["vod_year"], "2024");
    assert_eq!(detail["vod_area"], "中国大陆");
    assert_eq!(detail["vod_director"], "孙皓");
    let lines =
        crayon_core::cms::parse_lines(&s(detail, "vod_play_from"), &s(detail, "vod_play_url"), "");
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["episodes"].as_array().unwrap().len(), 2);
    assert_eq!(
        lines[0]["episodes"][0]["url"],
        format!("{}/play/good2.html", fixture.address)
    );
}
#[tokio::test]
async fn both_frontend_encodings_resolve_to_signed_mp4_bytes_without_cookie_leaks() {
    let fixture = Fixture::new().await;
    for name in ["good1", "good2"] {
        let result = fixture.resolve(name).await.unwrap();
        assert_eq!(result["probe"]["kind"], "mp4");
        assert_eq!(result["probe"]["confidence"], "signature");
        assert!(result["header"].get("Range").is_none());
        assert!(result["header"].get("Cookie").is_none());
        assert!(result["header"].get("Authorization").is_none());
        assert!(s(&result, "url").ends_with("/media/good?token=A/B+"));
    }
}
#[tokio::test]
async fn parser_and_media_failures_are_explicit() {
    let fixture = Fixture::new().await;
    assert!(
        fixture
            .resolve("bad")
            .await
            .unwrap_err()
            .to_string()
            .contains("未返回可识别媒体")
    );
    assert!(
        fixture
            .resolve("api-fail")
            .await
            .unwrap_err()
            .to_string()
            .contains("API 返回失败")
    );
    assert!(fixture.resolve("malformed").await.is_err());
    assert!(
        fixture
            .request(json!({"wd":"维护"}))
            .await
            .unwrap_err()
            .to_string()
            .contains("未返回有效结果页面")
    );
    assert!(
        fixture
            .request(json!({"ids":"https://outside.invalid/detail/1.html"}))
            .await
            .unwrap_err()
            .to_string()
            .contains("详情地址无效")
    );
    assert!(fixture.request(json!({"t":"../../escape"})).await.is_err());
    assert!(
        kankanya::resolve(
            &fixture.sources,
            &fixture.site,
            "https://outside.invalid/play/1.html",
            &CancellationToken::new()
        )
        .await
        .is_err()
    );
}
#[tokio::test]
async fn same_origin_media_reuses_the_public_check_cookie() {
    let fixture = Fixture::new().await;
    let result = fixture.resolve("same-origin").await.unwrap();
    assert_eq!(result["probe"]["kind"], "mp4");
    assert!(s(&result["header"], "Cookie").contains("gate=ready"));
}
#[tokio::test]
async fn media_redirect_does_not_register_site_credentials_for_the_cdn() {
    let fixture = Fixture::new().await;
    let result = fixture.resolve("redirect").await.unwrap();
    assert_eq!(result["probe"]["kind"], "mp4");
    assert!(s(&result, "url").contains("localhost"));
    assert!(result["header"].get("Cookie").is_none());
    assert!(result["header"].get("Authorization").is_none());
}
#[tokio::test]
async fn public_check_failure_and_changed_script_are_reported() {
    let mut fixture = Fixture::new().await;
    fixture.site["header"]["X-Reject"] = json!("true");
    assert!(
        fixture
            .request(json!({"ac":"list"}))
            .await
            .unwrap_err()
            .to_string()
            .contains("公开检查失败")
    );
    fixture.site["header"]["X-Changed-Gate"] = json!("true");
    assert!(
        fixture
            .request(json!({"ac":"list"}))
            .await
            .unwrap_err()
            .to_string()
            .contains("页面结构已变化")
    );
}
#[tokio::test]
async fn cancellation_and_body_timeout_are_bounded() {
    let mut fixture = Fixture::new().await;
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(
        kankanya::request(
            &fixture.sources,
            &fixture.site,
            &json!({"ac":"list"}),
            &cancel
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("取消")
    );
    assert!(fixture.calls.lock().unwrap().is_empty());
    fixture.site["timeout"] = json!(1);
    let start = std::time::Instant::now();
    let error = fixture.resolve("slow").await.unwrap_err();
    assert!(start.elapsed() < Duration::from_secs(3));
    assert!(format!("{error:#}").contains("超时") || format!("{error:#}").contains("timed out"));
}
