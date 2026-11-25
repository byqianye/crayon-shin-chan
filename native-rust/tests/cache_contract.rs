use crayon_core::cache::Cache;
use serde_json::json;
use std::time::Duration;

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
