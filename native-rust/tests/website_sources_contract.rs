use crayon_core::{catalogue::Catalogue, model::s, presets, sources::Sources, storage::Store};
use serde_json::json;
use std::sync::Arc;

#[test]
fn website_dispatch_requires_exact_hosts_and_root_apis() {
    use crayon_core::websites::{handles_address, supported};
    for api in [
        "https://www.kankanya.pro/",
        "https://kankanya.pro",
        "https://animoe.org/",
    ] {
        assert!(supported(&json!({"api":api})));
    }
    for api in [
        "https://www.kankanya.pro.evil.invalid/",
        "https://animoe.org/api.php/provide/vod",
        "https://animoe.org/?api=cms",
        "ftp://animoe.org/",
        "https://user:password@animoe.org/",
    ] {
        assert!(!supported(&json!({"api":api})));
    }
    let site = json!({"api":"https://animoe.org/"});
    assert!(handles_address(
        &site,
        "https://animoe.org/play/264-1-1.html"
    ));
    assert!(!handles_address(
        &site,
        "https://evil.invalid/play/264-1-1.html"
    ));
    assert!(!handles_address(
        &site,
        "https://user:password@animoe.org/play/264-1-1.html"
    ));
}

#[tokio::test]
async fn website_preset_imports_two_native_sources_without_spiders() {
    let folder = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(folder.path()).unwrap());
    let sources = Arc::new(Sources::new(store).unwrap());
    let preset = presets::list()
        .into_iter()
        .find(|v| v["id"] == "website-sources")
        .expect("two website adapters must be available in bundled subscriptions");
    assert_eq!(preset["siteCount"], 2);
    let source = presets::add(&sources, "website-sources").unwrap();
    let synced = sources.sync(&s(&source, "id")).await.unwrap();
    assert!(synced["error"].is_null(), "{synced}");
    let sites = sources.sites().unwrap();
    assert_eq!(sites.len(), 2);
    for site in &sites {
        assert_eq!(site["type"], 4);
        assert!(s(site, "spiderClass").is_empty());
        assert!(s(site, "unsupported").is_empty());
        assert_eq!(site["searchable"], true);
    }
    assert_eq!(
        Catalogue::new(sources.clone()).available().unwrap().len(),
        2
    );
    assert!(presets::add(&sources, "website-sources").is_err());
    let existing = sources
        .add(&json!({"name":"保留原有订阅","url":"https://example.invalid/config.json"}))
        .unwrap();
    sources.remove(&s(&source, "id")).unwrap();
    assert_eq!(sources.list().unwrap(), vec![existing]);
}
