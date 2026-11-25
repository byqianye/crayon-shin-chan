use axum::{Router, extract::Query, routing::get};
use crayon_core::{
    catalogue::Catalogue,
    category::leaves,
    cms,
    http::match_headers,
    model::{array, s},
    sources::Sources,
    storage::Store,
};
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

struct Fixture {
    address: String,
    task: tokio::task::JoinHandle<()>,
    _folder: tempfile::TempDir,
    catalogue: Arc<Catalogue>,
    calls: Arc<AtomicUsize>,
    queries: Arc<Mutex<Vec<HashMap<String, String>>>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn new() -> Self {
        let calls = Arc::new(AtomicUsize::new(0));
        let queries = Arc::new(Mutex::new(Vec::new()));
        let app=Router::new().route("/cms",get({let calls=calls.clone();let queries=queries.clone();move|Query(query):Query<HashMap<String,String>>|{let calls=calls.clone();let queries=queries.clone();async move{
            calls.fetch_add(1,Ordering::SeqCst);queries.lock().unwrap().push(query.clone());
            tokio::time::sleep(Duration::from_millis(if query.get("wd").is_some_and(|v|v=="slow"){2000}else{20})).await;
            let tid=query.get("t").cloned().unwrap_or_else(||"1".into());
            axum::Json(json!({"code":1,"pagecount":if tid=="6"{2}else{5},"class":[{"type_id":1,"type_name":"电影"},{"type_id":6,"type_name":"动作片","type_pid":1},{"type_id":7,"type_name":"喜剧片","type_pid":1}],"list":[{"vod_id":tid,"vod_name":"测试影片","vod_pic":"/p.jpg","vod_content":"<p>简介</p>","vod_play_from":"HLS","vod_play_url":"第1集$https://media.invalid/a.m3u8?x=$keep"}]}))
        }}}));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let folder = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(folder.path()).unwrap());
        store.write("sources",&json!([{"id":"s","name":"测试","url":format!("{address}/source.json"),"enabled":true,"raw":{"sites":[{"key":"cms","name":"测试CMS","api":format!("{address}/cms"),"type":"cms"}]}}])).unwrap();
        let catalogue = Arc::new(Catalogue::new(Arc::new(Sources::new(store).unwrap())));
        Self {
            address,
            task,
            _folder: folder,
            catalogue,
            calls,
            queries,
        }
    }
}

#[test]
fn cms_templates_xml_and_episode_delimiters_match_original() {
    let address = cms::build_url(
        &json!({"api":"https://a.invalid/?ac={ac}&q={wd}&page={pg}"}),
        &json!({"ac":"videolist","wd":"中文 +","pg":1,"t":6}),
    )
    .unwrap();
    let url = url::Url::parse(&address).unwrap();
    let params: HashMap<_, _> = url.query_pairs().collect();
    assert_eq!(params["q"], "中文 +");
    assert_eq!(params["t"], "6");
    assert!(!address.contains("{ac}"));
    let xml = r#"<?xml version="1.0"?><rss><class><ty id="6" pid="1">动作片</ty></class><list page="1" pagecount="4"><video><id>3</id><name><![CDATA[中文 & 原名]]></name><des><![CDATA[<p>简介</p>]]></des><dl><dd flag="m3u8"><![CDATA[第1集$https://a.invalid/1.m3u8?token=$yes#https://a.invalid/2.mp4]]></dd></dl></video></list></rss>"#;
    let payload = cms::parse_payload(xml).unwrap();
    assert_eq!(payload["class"][0]["type_pid"], "1");
    let detail = cms::detail(
        &payload["list"][0],
        &json!({"key":"xml","name":"XML","api":"https://a.invalid"}),
    );
    assert_eq!(detail["vod_name"], "中文 & 原名");
    assert_eq!(detail["vod_content"], "简介");
    assert_eq!(
        detail["lines"][0]["episodes"][0]["url"],
        "https://a.invalid/1.m3u8?token=$yes"
    );
    assert_eq!(detail["lines"][0]["episodes"][1]["name"], "第2集");
    assert!(cms::parse_payload("<html>error</html>").is_err());
    assert!(cms::parse_payload("{\"message\":\"error\"}").is_err());
}

#[test]
fn html_rule_fallbacks_relative_images_and_playlists_work() {
    let site = json!({"key":"web","name":"网页","type":1,"api":"https://a.invalid/search","rules":{"list":".item","name":".missing&&Text||a&&Text","link":"a&&href","pic":"img&&data-src,src","detailName":"h1&&Text","detailContent":".intro&&Text","playList":".line","playItem":"a"}});
    let items = crayon_core::html_rules::search(
        "<div class='item'><a href='/detail/1'>测试 影片</a><img src='/p.jpg'></div>",
        &site,
        "https://a.invalid/search?q=1",
        30,
    )
    .unwrap();
    assert_eq!(items[0]["vod_id"], "https://a.invalid/detail/1");
    assert_eq!(items[0]["vod_pic"], "https://a.invalid/p.jpg");
    let detail=crayon_core::html_rules::detail("<h1>中文影片</h1><div class='intro'>故事 <b>简介</b></div><div class='line' data-name='HLS'><a href='https://a.invalid/a.m3u8'>第一集</a></div>",&site,"https://a.invalid/detail/1").unwrap();
    assert_eq!(detail["lines"][0]["name"], "HLS");
    assert_eq!(detail["lines"][0]["episodes"][0]["name"], "第一集");
}

#[test]
fn header_rules_match_only_full_host_and_port_with_wildcards() {
    let rules = vec![
        json!({"host":"*.Example.com","header":{"Cookie":"protected"}}),
        json!({"host":"host.*:81","header":{"Referer":"ref"}}),
    ];
    assert_eq!(
        match_headers("https://a.example.com/p", &rules)["Cookie"],
        "protected"
    );
    assert!(
        match_headers("https://evil.invalid/example.com", &rules)
            .as_object()
            .unwrap()
            .is_empty()
    );
    assert!(
        match_headers("https://example.com", &rules)
            .as_object()
            .unwrap()
            .is_empty()
    );
    assert!(
        match_headers("https://a.example.com:81", &rules)
            .as_object()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        match_headers("http://host.invalid:81/p", &rules)["Referer"],
        "ref"
    );
}

#[test]
fn leaf_categories_keep_animation_movies_and_exclude_commentary_ancestors() {
    let classes = array(
        &json!([{"id":"1","name":"电影","pid":"0"},{"id":"6","name":"动画片","pid":"1"},{"id":"7","name":"纪录片","pid":"1"},{"id":"8","name":"电影解说","pid":"0"},{"id":"9","name":"动作片","pid":"8"}]),
    );
    let result = leaves(&classes);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0]["bucket"], "电影");
    assert_eq!(result[0]["sub"], "动画");
    assert_eq!(result[1]["bucket"], "纪录片");
}

#[tokio::test]
async fn search_primes_detail_cache_and_site_changes_invalidate_it() {
    let f = Fixture::new().await;
    let site = f.catalogue.sources.site("cms").unwrap();
    let result = f
        .catalogue
        .search_site(&site, "测试", 30, &CancellationToken::new())
        .await;
    assert_eq!(result["ok"], true);
    assert_eq!(
        f.catalogue.detail("cms", "1").await.unwrap()["vod_content"],
        "简介"
    );
    assert_eq!(f.calls.load(Ordering::SeqCst), 1);
    f.catalogue
        .sources
        .override_site(&["cms".into()], &json!({"header":{"X-Revision":"2"}}))
        .unwrap();
    f.catalogue.detail("cms", "1").await.unwrap();
    assert_eq!(f.calls.load(Ordering::SeqCst), 2);
    assert!(s(&result["list"][0], "vod_pic").starts_with(&f.address));
}

#[tokio::test]
async fn concurrent_details_coalesce_and_home_uses_cached_data() {
    let f = Fixture::new().await;
    let (a, b) = tokio::join!(
        f.catalogue.detail("cms", "1"),
        f.catalogue.detail("cms", "1")
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(f.calls.load(Ordering::SeqCst), 1);
    let first = f.catalogue.home(false).await.unwrap();
    let second = f.catalogue.home(false).await.unwrap();
    assert_eq!(first, second);
    assert_eq!(f.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn browse_requests_every_leaf_and_keeps_deepest_page() {
    let f = Fixture::new().await;
    let page = f.catalogue.browse("电影", "", 1).await.unwrap();
    let ids: Vec<_> = array(&page["results"][0]["list"])
        .iter()
        .map(|v| s(v, "vod_id"))
        .collect();
    assert_eq!(ids, vec!["6", "7"]);
    assert_eq!(page["pageCount"], 5);
    let queries = f.queries.lock().unwrap();
    assert!(queries.iter().any(|q| q.get("t").is_some_and(|v| v == "6")));
    assert!(queries.iter().any(|q| q.get("t").is_some_and(|v| v == "7")));
}

#[tokio::test]
async fn canceled_search_does_not_record_a_site_failure() {
    let f = Fixture::new().await;
    let catalogue = f.catalogue.clone();
    let task = tokio::spawn(async move { catalogue.search("slow", json!({}), None).await });
    while f.calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    f.catalogue.cancel_search();
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        f.catalogue.sources.store.read("health", json!({})).unwrap(),
        json!({})
    );
}
