use crayon_core::{
    health::{self, Batch},
    storage::Store,
};
use serde_json::{Value, json};

#[test]
fn batched_search_records_preserve_cooldown_recovery_and_existing_sites() {
    let folder = tempfile::tempdir().unwrap();
    let store = Store::open(folder.path()).unwrap();
    health::record(&store, "existing", true, 10).unwrap();
    {
        let batch = Batch::new(&store);
        for _ in 0..3 {
            batch.record("failed", false, 25).unwrap();
        }
        batch.flush().unwrap();
        let saved = Store::open(folder.path())
            .unwrap()
            .read("health", json!({}))
            .unwrap();
        assert!(health::cooling(&saved["failed"]));
        assert_eq!(saved["failed"]["samples"].as_array().unwrap().len(), 3);
        batch.record("failed", true, 12).unwrap();
    }
    let saved: Value =
        serde_json::from_slice(&std::fs::read(folder.path().join("health.json")).unwrap()).unwrap();
    assert_eq!(saved["existing"]["samples"].as_array().unwrap().len(), 1);
    assert_eq!(saved["failed"]["samples"].as_array().unwrap().len(), 4);
    assert_eq!(saved["failed"]["consecutiveFails"], 0);
    assert!(!health::cooling(&saved["failed"]));
}

#[test]
fn dropping_a_search_batch_persists_only_the_latest_fifty_samples() {
    let folder = tempfile::tempdir().unwrap();
    let store = Store::open(folder.path()).unwrap();
    {
        let batch = Batch::new(&store);
        for ms in 0..60 {
            batch.record("site", true, ms).unwrap();
        }
    }
    let saved = Store::open(folder.path())
        .unwrap()
        .read("health", json!({}))
        .unwrap();
    assert_eq!(saved["site"]["samples"].as_array().unwrap().len(), 50);
    assert_eq!(saved["site"]["samples"][0]["ms"], 10);
    assert_eq!(saved["site"]["samples"][49]["ms"], 59);
}

#[test]
fn batch_flush_reports_corrupt_data_and_preserves_the_original_file() {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("health.json"), "broken").unwrap();
    let store = Store::open(folder.path()).unwrap();
    let batch = Batch::new(&store);
    assert!(batch.record("site", true, 10).is_err());
    drop(batch);
    assert_eq!(
        std::fs::read_to_string(folder.path().join("health.json")).unwrap(),
        "broken"
    );
}

#[test]
fn deferred_and_immediate_records_keep_completion_order_and_live_health() {
    let folder = tempfile::tempdir().unwrap();
    let store = Store::open(folder.path()).unwrap();
    {
        let batch = Batch::new(&store);
        batch.record("site", false, 1).unwrap();
        assert_eq!(
            store.read("health", json!({})).unwrap()["site"]["consecutiveFails"],
            1.0
        );
        health::record(&store, "site", true, 2).unwrap();
        batch.record("site", false, 3).unwrap();
    }
    let saved = Store::open(folder.path())
        .unwrap()
        .read("health", json!({}))
        .unwrap();
    assert_eq!(
        saved["site"]["samples"]
            .as_array()
            .unwrap()
            .iter()
            .map(|sample| sample["ms"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(saved["site"]["consecutiveFails"], 1.0);
    assert!(!health::cooling(&saved["site"]));
}

#[test]
fn failed_deferred_write_retains_dirty_data_for_retry() {
    let folder = tempfile::tempdir().unwrap();
    let store = Store::open(folder.path()).unwrap();
    store.read("health", json!({})).unwrap();
    let blocked = folder.path().join("health.json");
    std::fs::create_dir(&blocked).unwrap();
    let batch = Batch::new(&store);
    batch.record("site", true, 15).unwrap();
    assert!(batch.flush().is_err());
    assert_eq!(
        store.read("health", json!({})).unwrap()["site"]["samples"][0]["ms"],
        15
    );
    std::fs::remove_dir(&blocked).unwrap();
    batch.flush().unwrap();
    assert_eq!(
        Store::open(folder.path())
            .unwrap()
            .read("health", json!({}))
            .unwrap()["site"]["samples"][0]["ms"],
        15
    );
}
