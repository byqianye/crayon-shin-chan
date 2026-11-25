use crayon_core::{
    http::Net,
    spider::{Spider, cache_key, make_url, parse_url},
    storage::Store,
};
use serde_json::json;
use std::sync::Arc;

#[test]
fn spider_addresses_and_cache_names_preserve_original_ids() {
    let address = make_url("站点:1", "线路/二", "https://a.invalid/play?a=1&b=中文+%");
    assert_eq!(
        parse_url(&address),
        Some((
            "站点:1".into(),
            "线路/二".into(),
            "https://a.invalid/play?a=1&b=中文+%".into()
        ))
    );
    let site = json!({"sourceId":"a","jar":" https://a.invalid/spider.jar "});
    assert_eq!(
        cache_key(&site),
        format!("{:x}", md5::compute("a|https://a.invalid/spider.jar"))
    );
}

#[tokio::test]
async fn real_java_host_supports_concurrent_calls_shutdown_and_restart() {
    let folder = tempfile::tempdir().unwrap();
    let spider = Spider::new(
        Arc::new(Store::open(folder.path()).unwrap()),
        Net::new().unwrap(),
    );
    assert!(spider.runtime_status()["ready"].as_bool().unwrap());
    let (left, right) = tokio::join!(spider.ping(), spider.ping());
    let result = async {
        let left = left?;
        let right = right?;
        assert_eq!(left["sites"], 0);
        assert_eq!(right["sites"], 0);
        assert!(left["maxMb"].as_u64().unwrap() <= 192);
        anyhow::Ok(())
    }
    .await;
    spider.stop().await.unwrap();
    result.unwrap();
    assert_eq!(spider.ping().await.unwrap()["sites"], 0);
    spider.stop().await.unwrap();
}
