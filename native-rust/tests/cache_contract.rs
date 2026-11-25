use crayon_core::cache::Cache;
use serde_json::json;
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn abandoning_last_waiter_releases_request_and_allows_retry() {
    let cache = Cache::default();
    let resource = Arc::new(());
    let released = Arc::downgrade(&resource);
    let mut request = Box::pin(cache.coalesce("request".into(), async move {
        let _resource = resource;
        std::future::pending().await
    }));
    assert!(futures_util::poll!(&mut request).is_pending());
    drop(request);
    assert!(
        released.upgrade().is_none(),
        "最后一个等待者退出时必须释放旧请求"
    );
    assert_eq!(
        cache
            .coalesce("request".into(), async { Ok(json!("fresh")) })
            .await
            .unwrap(),
        json!("fresh")
    );
}

#[tokio::test]
async fn abandoning_one_waiter_keeps_other_callers_on_same_request() {
    let cache = Cache::default();
    let (send, receive) = tokio::sync::oneshot::channel();
    let mut first =
        Box::pin(cache.coalesce("request".into(), async move { Ok(receive.await.unwrap()) }));
    assert!(futures_util::poll!(&mut first).is_pending());
    let mut second = Box::pin(cache.coalesce("request".into(), async {
        panic!("不能重复请求");
    }));
    assert!(futures_util::poll!(&mut second).is_pending());
    drop(first);
    let mut third = Box::pin(cache.coalesce("request".into(), async {
        panic!("仍有等待者时必须合并");
    }));
    assert!(futures_util::poll!(&mut third).is_pending());
    send.send(json!("shared")).unwrap();
    assert_eq!(second.await.unwrap(), json!("shared"));
    assert_eq!(third.await.unwrap(), json!("shared"));
    assert_eq!(
        cache
            .coalesce("request".into(), async { Ok(json!("next")) })
            .await
            .unwrap(),
        json!("next")
    );
}

#[tokio::test]
async fn abandoning_nested_requests_releases_both_layers() {
    let cache = Arc::new(Cache::default());
    let inner_cache = cache.clone();
    let resource = Arc::new(());
    let released = Arc::downgrade(&resource);
    let mut request = Box::pin(cache.coalesce("section".into(), async move {
        inner_cache
            .coalesce("http".into(), async move {
                let _resource = resource;
                std::future::pending().await
            })
            .await
    }));
    assert!(futures_util::poll!(&mut request).is_pending());
    drop(request);
    assert!(released.upgrade().is_none());
    assert_eq!(Arc::strong_count(&cache), 1);
}

#[test]
fn capacity_evicts_oldest_insertion_and_replacement_refreshes_it() {
    let cache = Cache::default();
    let ttl = Duration::from_secs(60);
    cache.insert("a".into(), json!(1), ttl, 2);
    cache.insert("b".into(), json!(2), ttl, 2);
    cache.insert("a".into(), json!(3), ttl, 2);
    cache.insert("c".into(), json!(4), ttl, 2);
    assert_eq!(cache.read("a"), Some(json!(3)));
    assert_eq!(cache.read("b"), None);
    assert_eq!(cache.read("c"), Some(json!(4)));
}

#[test]
fn expired_entries_and_zero_capacity_are_never_returned() {
    let cache = Cache::default();
    cache.insert("expired".into(), json!(1), Duration::ZERO, 2);
    cache.insert("fresh".into(), json!(2), Duration::from_secs(60), 2);
    assert_eq!(cache.read("expired"), None);
    assert_eq!(cache.read("fresh"), Some(json!(2)));
    cache.insert("none".into(), json!(3), Duration::from_secs(60), 0);
    assert_eq!(cache.read("none"), None);
    assert_eq!(cache.read("fresh"), None);
}
