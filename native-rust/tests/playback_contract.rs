use crayon_core::{playback::Playback, storage::Store};
use serde_json::{Value, json};
use std::sync::Arc;

fn entry(ep: usize) -> Value {
    json!({"siteKey":"fixture","vodId":"1","name":"测试影片","lineIndex":0,"epIndex":ep})
}
fn event(session: &str, kind: &str, position: f64, duration: f64) -> Value {
    json!({"session":session,"event":kind,"position":position,"duration":duration})
}
fn stored(store: &Store) -> Value {
    store.read("history", json!([])).unwrap()[0].clone()
}

#[test]
fn resume_once_and_preserve_progress_before_duration_is_known() {
    let folder = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(folder.path()).unwrap());
    let mut old = entry(0);
    old["position"] = json!(40);
    old["duration"] = json!(120);
    store.record_history(old).unwrap();
    let playback = Playback::new(store.clone());
    playback.attach("one", entry(0)).unwrap();
    assert!(
        playback
            .event(&event("one", "timeupdate", 0.0, 0.0), 1000)
            .unwrap()
            .action
            .is_none()
    );
    assert_eq!(
        playback
            .event(&event("one", "durationchange", 0.0, 120.0), 1001)
            .unwrap()
            .action
            .unwrap()["seek"],
        40.0
    );
    playback
        .event(&event("one", "timeupdate", 0.0, 120.0), 1002)
        .unwrap();
    assert_eq!(stored(&store)["position"], 40);
    playback
        .event(&event("one", "resumed", 40.0, 120.0), 1003)
        .unwrap();
    assert!(
        playback
            .event(&event("one", "canplay", 41.0, 120.0), 1004)
            .unwrap()
            .action
            .is_none()
    );
    playback
        .event(&event("one", "destroy", 0.0, 0.0), 1005)
        .unwrap();
    assert_eq!(stored(&store)["position"], 41.0);
    assert_eq!(stored(&store)["duration"], 120.0);
}

#[test]
fn pause_destroy_and_switch_keep_the_final_few_seconds_and_ignore_stale_events() {
    let folder = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(folder.path()).unwrap());
    let playback = Playback::new(store.clone());
    playback.attach("one", entry(0)).unwrap();
    playback
        .event(&event("one", "timeupdate", 8.0, 100.0), 1000)
        .unwrap();
    assert!(
        playback
            .event(&event("one", "pause", 11.0, 100.0), 1100)
            .unwrap()
            .saved
    );
    assert_eq!(stored(&store)["position"], 11.0);
    playback
        .event(&event("one", "timeupdate", 12.0, 100.0), 1200)
        .unwrap();
    playback.attach("two", entry(1)).unwrap();
    assert_eq!(stored(&store)["position"], 12.0);
    assert!(
        playback
            .event(&event("one", "destroy", 80.0, 100.0), 1300)
            .unwrap()
            .event
            .is_none()
    );
    assert!(
        playback
            .event(&event("two", "loadedmetadata", 0.0, 100.0), 1400)
            .unwrap()
            .action
            .is_none()
    );
    playback
        .event(&event("two", "destroy", 3.0, 100.0), 1500)
        .unwrap();
    assert_eq!(stored(&store)["epIndex"], 1);
    assert_eq!(stored(&store)["position"], 3.0);
}

#[test]
fn capture_and_close_save_the_real_position_instead_of_sampled_frames() {
    let folder = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(folder.path()).unwrap());
    let playback = Playback::new(store.clone());
    playback.attach("one", entry(0)).unwrap();
    playback
        .event(&event("one", "timeupdate", 13.0, 100.0), 1000)
        .unwrap();
    playback
        .event(&event("one", "captureStart", 14.0, 100.0), 1100)
        .unwrap();
    playback
        .event(&event("one", "pause", 90.0, 100.0), 1200)
        .unwrap();
    assert!(
        !playback
            .event(&event("one", "timeupdate", 99.0, 100.0), 10000)
            .unwrap()
            .saved
    );
    playback
        .event(&event("one", "destroy", 99.0, 100.0), 11000)
        .unwrap();
    assert_eq!(stored(&store)["position"], 14.0);
    playback.attach("two", entry(0)).unwrap();
    assert_eq!(
        playback
            .event(&event("two", "loadedmetadata", 0.0, 100.0), 11999)
            .unwrap()
            .action
            .unwrap()["seek"],
        14.0
    );
    playback
        .event(&event("two", "resumed", 14.0, 100.0), 12000)
        .unwrap();
    playback
        .event(&event("two", "captureStart", 15.0, 100.0), 12001)
        .unwrap();
    playback
        .event(&event("two", "captureEnd", 15.0, 100.0), 12002)
        .unwrap();
    playback
        .event(&event("two", "timeupdate", 16.0, 100.0), 12003)
        .unwrap();
    playback.finish().unwrap();
    assert_eq!(stored(&store)["position"], 16.0);
}

#[test]
fn hls_recovers_twice_then_exposes_the_failure_without_recording_live_history() {
    let folder = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(folder.path()).unwrap());
    let playback = Playback::new(store.clone());
    playback.attach("live", Value::Null).unwrap();
    let failure =
        json!({"session":"live","event":"hlsError","fatal":true,"errorType":"networkError"});
    for at in 1..=2 {
        let out = playback.event(&failure, at).unwrap();
        assert_eq!(out.action.unwrap()["recover"], "startLoad");
        assert_eq!(out.event.unwrap()["recovered"], true);
    }
    let out = playback.event(&failure, 3).unwrap();
    assert!(out.action.is_none());
    assert_ne!(out.event.unwrap()["recovered"], true);
    playback
        .event(&event("live", "destroy", 30.0, 100.0), 4)
        .unwrap();
    assert_eq!(store.read("history", json!([])).unwrap(), json!([]));
}
