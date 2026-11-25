use crayon_core::storage::Store;
use serde_json::json;

#[test]
fn existing_json_unknown_fields_and_unicode_are_preserved() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("settings.json"),
        r##"{"accent":"#123456","future":{"名称":"中文路径"}}"##,
    )
    .unwrap();
    let store = Store::open(temp.path()).unwrap();
    let settings = store.settings().unwrap();
    assert_eq!(settings["accent"], "#123456");
    assert_eq!(settings["future"]["名称"], "中文路径");
    assert_eq!(settings["concurrency"], 8);
    store
        .save_settings(&json!({"ai": {"model": "custom"}}))
        .unwrap();
    assert_eq!(store.settings().unwrap()["ai"]["baseUrl"], "");
}

#[test]
fn corrupt_store_is_reported_and_never_overwritten() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("history.json"), "broken json").unwrap();
    let store = Store::open(temp.path()).unwrap();
    assert!(store.read("history", json!([])).is_err());
    assert!(
        store
            .record_history(json!({"siteKey":"a","vodId":"1"}))
            .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(temp.path().join("history.json")).unwrap(),
        "broken json"
    );
}

#[test]
fn unknown_duration_keeps_progress_but_another_episode_resets_it() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let mut entry = json!({"siteKey":"a","siteName":"站点","vodId":"1","name":"庆余年 第二季","lineIndex":0,"epIndex":1,"position":90,"duration":600});
    store.record_history(entry.clone()).unwrap();
    entry["position"] = json!(0);
    entry["duration"] = json!(0);
    store.record_history(entry.clone()).unwrap();
    let history = store.read("history", json!([])).unwrap();
    assert_eq!(history[0]["position"], 90);
    entry["epIndex"] = json!(2);
    store.record_history(entry).unwrap();
    assert_eq!(store.read("history", json!([])).unwrap()[0]["position"], 0);
}

#[test]
fn same_title_is_grouped_across_sites_but_seasons_stay_separate() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    for (site, name) in [
        ("a", "庆余年 第二季"),
        ("b", "庆余年（第二季）"),
        ("c", "庆余年第一季"),
    ] {
        store.record_history(json!({"siteKey":site,"vodId":"1","name":name,"lineIndex":0,"epIndex":0,"position":20,"duration":100})).unwrap();
    }
    let library = store.library().unwrap();
    assert_eq!(library["history"].as_array().unwrap().len(), 2);
    store
        .toggle_favorite(json!({"siteKey":"a","vodId":"1","name":"影片"}))
        .unwrap();
    assert_eq!(
        store.library().unwrap()["favorites"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    store
        .toggle_favorite(json!({"siteKey":"a","vodId":"1"}))
        .unwrap();
    assert!(
        store.library().unwrap()["favorites"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn store_names_cannot_escape_data_directory() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    assert!(store.read("../settings", json!({})).is_err());
    assert!(store.write("../settings", &json!({})).is_err());
}
