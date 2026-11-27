use crayon_core::{mpv::MpvConnection, storage::Store};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    sync::watch,
    time::{Duration, timeout},
};
use tokio_util::sync::CancellationToken;

fn entry(ep: usize) -> Value {
    json!({"siteKey":"fixture","vodId":"1","name":"测试影片","lineIndex":0,"epIndex":ep})
}

async fn until_status(
    status: &mut watch::Receiver<Value>,
    predicate: impl Fn(&Value) -> bool,
) -> Value {
    timeout(Duration::from_secs(3), async {
        loop {
            let value = status.borrow().clone();
            if predicate(&value) {
                return value;
            }
            status.changed().await.unwrap();
        }
    })
    .await
    .expect("mpv status did not reach expected state")
}

async fn send(writer: &mut (impl AsyncWrite + Unpin), value: Value) {
    let mut bytes = serde_json::to_vec(&value).unwrap();
    bytes.push(b'\n');
    writer.write_all(&bytes).await.unwrap();
}

async fn fake_player<T: AsyncRead + AsyncWrite + Unpin>(
    transport: T,
    progress: f64,
    final_position: f64,
    expect_resume: Option<f64>,
) {
    let (reader, mut writer) = tokio::io::split(transport);
    let mut lines = BufReader::new(reader).lines();
    let mut observed = 0;
    while observed < 4 {
        let request: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(request["command"][0], "observe_property");
        send(
            &mut writer,
            json!({"request_id":request["request_id"],"error":"success"}),
        )
        .await;
        observed += 1;
    }
    // Fragmented UTF-8 JSON must survive reads that do not align with JSON lines.
    let events = format!(
        "{}\n{}\n",
        json!({"event":"property-change","name":"duration","data":100.0}),
        json!({"event":"property-change","name":"time-pos","data":progress,"comment":"分片"})
    );
    for bytes in events.as_bytes().chunks(3) {
        writer.write_all(bytes).await.unwrap();
    }
    if let Some(position) = expect_resume {
        let request: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(
            request["command"],
            json!(["seek", position, "absolute+exact"])
        );
        send(
            &mut writer,
            json!({"request_id":request["request_id"],"error":"success"}),
        )
        .await;
        send(&mut writer, json!({"event":"playback-restart"})).await;
        let request: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(request["command"], json!(["get_property", "time-pos"]));
        send(
            &mut writer,
            json!({"request_id":request["request_id"],"error":"success","data":position}),
        )
        .await;
    }
    send(
        &mut writer,
        json!({"event":"property-change","name":"time-pos","data":progress}),
    )
    .await;
    send(
        &mut writer,
        json!({"event":"property-change","name":"pause","data":true}),
    )
    .await;
    while let Some(line) = lines.next_line().await.unwrap() {
        let request: Value = serde_json::from_str(&line).unwrap();
        match request["command"][0].as_str().unwrap() {
            "get_property" => {
                let data = match request["command"][1].as_str().unwrap() {
                    "time-pos" => json!(final_position),
                    "duration" => json!(100.0),
                    "pause" => json!(true),
                    other => panic!("unexpected property {other}"),
                };
                send(
                    &mut writer,
                    json!({"request_id":request["request_id"],"error":"success","data":data}),
                )
                .await;
            }
            "quit" => {
                return;
            }
            other => panic!("unexpected command {other}"),
        }
    }
}

#[tokio::test]
async fn fragmented_ipc_saves_pause_and_final_snapshot_and_keeps_episode_identity() {
    let folder = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(folder.path()).unwrap());
    let (client, server) = tokio::io::duplex(4096);
    let connection = MpvConnection::new(store.clone(), "first", entry(0), false).unwrap();
    let mut status = connection.subscribe();
    let stop = CancellationToken::new();
    let runner = tokio::spawn(connection.run(client, stop.clone()));
    let player = tokio::spawn(fake_player(server, 11.0, 14.0, None));
    until_status(&mut status, |value| {
        value["paused"] == true && value["savedAt"].is_number()
    })
    .await;
    assert_eq!(
        store.read("history", json!([])).unwrap()[0]["position"],
        11.0
    );
    stop.cancel();
    runner.await.unwrap().unwrap();
    player.await.unwrap();
    let saved = store.read("history", json!([])).unwrap();
    assert_eq!(saved[0]["position"], 14.0);
    assert_eq!(saved[0]["epIndex"], 0);
    assert_eq!(status.borrow()["state"], "stopped");

    // A new episode using the very same media address gets its own identity.
    let (client, server) = tokio::io::duplex(4096);
    let connection = MpvConnection::new(store.clone(), "second", entry(1), false).unwrap();
    let mut status = connection.subscribe();
    let stop = CancellationToken::new();
    let runner = tokio::spawn(connection.run(client, stop.clone()));
    let player = tokio::spawn(fake_player(server, 3.0, 4.0, None));
    until_status(&mut status, |value| value["paused"] == true).await;
    stop.cancel();
    runner.await.unwrap().unwrap();
    player.await.unwrap();
    let saved = store.read("history", json!([])).unwrap();
    assert_eq!(saved[0]["epIndex"], 1);
    assert_eq!(saved[0]["position"], 4.0);
}

#[tokio::test]
async fn resume_seeks_once_and_waits_for_a_confirmed_position() {
    let folder = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(folder.path()).unwrap());
    let mut previous = entry(0);
    previous["position"] = json!(40);
    previous["duration"] = json!(100);
    store.record_history(previous).unwrap();
    let (client, server) = tokio::io::duplex(4096);
    let connection = MpvConnection::new(store.clone(), "resume", entry(0), false).unwrap();
    let mut status = connection.subscribe();
    let stop = CancellationToken::new();
    let runner = tokio::spawn(connection.run(client, stop.clone()));
    let player = tokio::spawn(fake_player(server, 42.0, 44.0, Some(40.0)));
    until_status(&mut status, |value| value["paused"] == true).await;
    stop.cancel();
    runner.await.unwrap().unwrap();
    player.await.unwrap();
    assert_eq!(
        store.read("history", json!([])).unwrap()[0]["position"],
        44.0
    );
}

#[tokio::test]
async fn initial_restart_and_stale_position_do_not_abort_a_pending_resume() {
    let folder = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(folder.path()).unwrap());
    let mut previous = entry(0);
    previous["position"] = json!(40);
    previous["duration"] = json!(100);
    store.record_history(previous).unwrap();
    let (client, server) = tokio::io::duplex(4096);
    let connection = MpvConnection::new(store.clone(), "initial-restart", entry(0), false).unwrap();
    let status = connection.subscribe();
    let player = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut lines = BufReader::new(reader).lines();
        for _ in 0..4 {
            let request: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            send(
                &mut writer,
                json!({"request_id":request["request_id"],"error":"success"}),
            )
            .await;
        }
        send(
            &mut writer,
            json!({"event":"property-change","name":"duration","data":100.0}),
        )
        .await;
        send(
            &mut writer,
            json!({"event":"property-change","name":"time-pos","data":0.0}),
        )
        .await;
        let seek: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(seek["command"], json!(["seek", 40.0, "absolute+exact"]));
        // Initial playback can restart before the queued seek has taken effect.
        send(&mut writer, json!({"event":"playback-restart"})).await;
        send(
            &mut writer,
            json!({"request_id":seek["request_id"],"error":"success"}),
        )
        .await;
        for position in [0.0, 40.0] {
            let line = timeout(Duration::from_secs(1), lines.next_line())
                .await
                .expect("resume confirmation should retry")
                .unwrap()
                .unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["command"], json!(["get_property", "time-pos"]));
            send(
                &mut writer,
                json!({"request_id":request["request_id"],"error":"success","data":position}),
            )
            .await;
            if position == 0.0 {
                send(&mut writer, json!({"event":"playback-restart"})).await;
            }
        }
        send(
            &mut writer,
            json!({"event":"property-change","name":"time-pos","data":42.0}),
        )
        .await;
        send(&mut writer, json!({"event":"end-file","reason":"stop"})).await;
    });
    connection
        .run(client, CancellationToken::new())
        .await
        .unwrap();
    player.await.unwrap();
    assert_eq!(status.borrow()["state"], "stopped");
    assert_eq!(
        store.read("history", json!([])).unwrap()[0]["position"],
        42.0
    );
}

#[tokio::test]
async fn failed_required_command_is_visible_and_does_not_create_history() {
    let folder = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(folder.path()).unwrap());
    let (client, server) = tokio::io::duplex(4096);
    let connection = MpvConnection::new(store.clone(), "failed", entry(0), false).unwrap();
    let status = connection.subscribe();
    let player = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let line = BufReader::new(reader)
            .lines()
            .next_line()
            .await
            .unwrap()
            .unwrap();
        let request: Value = serde_json::from_str(&line).unwrap();
        send(
            &mut writer,
            json!({"request_id":request["request_id"],"error":"command not found"}),
        )
        .await;
    });
    let error = connection
        .run(client, CancellationToken::new())
        .await
        .unwrap_err();
    player.await.unwrap();
    assert!(error.to_string().contains("command not found"));
    assert_eq!(status.borrow()["state"], "error");
    assert!(
        status.borrow()["error"]
            .as_str()
            .unwrap()
            .contains("command not found")
    );
    assert_eq!(store.read("history", json!([])).unwrap(), json!([]));
}

#[tokio::test]
async fn cancelled_external_request_does_not_resolve_or_replace_current_status() {
    use crayon_core::mpv::ExternalPlayback;
    let folder = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(folder.path()).unwrap());
    store
        .save_settings(&json!({"externalPlayer":{"command":"missing-player.exe"}}))
        .unwrap();
    let manager = ExternalPlayback::new(store.clone());
    let cancel = CancellationToken::new();
    cancel.cancel();
    let error = manager
        .open_cancellable(
            "https://fixture.invalid/media.mp4",
            "cancelled",
            entry(0),
            false,
            &cancel,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("已取消"));
    assert_eq!(manager.status()["state"], "idle");
    assert_eq!(store.read("history", json!([])).unwrap(), json!([]));
}

#[tokio::test]
async fn live_progress_is_reported_without_writing_film_history() {
    let folder = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(folder.path()).unwrap());
    let (client, server) = tokio::io::duplex(4096);
    let connection = MpvConnection::new(store.clone(), "live", entry(0), true).unwrap();
    let mut status = connection.subscribe();
    let stop = CancellationToken::new();
    let runner = tokio::spawn(connection.run(client, stop.clone()));
    let player = tokio::spawn(fake_player(server, 11.0, 14.0, None));
    until_status(&mut status, |value| value["paused"] == true).await;
    stop.cancel();
    runner.await.unwrap().unwrap();
    player.await.unwrap();
    assert_eq!(status.borrow()["position"], 14.0);
    assert_eq!(store.read("history", json!([])).unwrap(), json!([]));
}

#[tokio::test]
async fn eof_marks_completion_and_shutdown_preserves_the_last_reliable_position() {
    let folder = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(folder.path()).unwrap());
    let (client, server) = tokio::io::duplex(4096);
    let connection = MpvConnection::new(store.clone(), "eof", entry(0), false).unwrap();
    let player = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut lines = BufReader::new(reader).lines();
        for _ in 0..4 {
            let request: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            send(
                &mut writer,
                json!({"request_id":request["request_id"],"error":"success"}),
            )
            .await;
        }
        send(
            &mut writer,
            json!({"event":"property-change","name":"duration","data":100.0}),
        )
        .await;
        send(
            &mut writer,
            json!({"event":"property-change","name":"time-pos","data":99.0}),
        )
        .await;
        send(
            &mut writer,
            json!({"event":"property-change","name":"eof-reached","data":true}),
        )
        .await;
        send(&mut writer, json!({"event":"end-file","reason":"eof"})).await;
    });
    connection
        .run(client, CancellationToken::new())
        .await
        .unwrap();
    player.await.unwrap();
    assert_eq!(
        store.read("history", json!([])).unwrap()[0]["position"],
        100.0
    );
}

#[tokio::test]
async fn lost_ipc_and_unresponsive_final_queries_preserve_observed_history() {
    for disconnect in [false, true] {
        let folder = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(folder.path()).unwrap());
        let (client, server) = tokio::io::duplex(4096);
        let connection = MpvConnection::new(store.clone(), "closing", entry(0), false).unwrap();
        let mut status = connection.subscribe();
        let stop = CancellationToken::new();
        let runner = tokio::spawn(connection.run(client, stop.clone()));
        let (release, released) = tokio::sync::oneshot::channel::<()>();
        let player = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(server);
            let mut lines = BufReader::new(reader).lines();
            for _ in 0..4 {
                let request: Value =
                    serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
                send(
                    &mut writer,
                    json!({"request_id":request["request_id"],"error":"success"}),
                )
                .await;
            }
            send(
                &mut writer,
                json!({"event":"property-change","name":"duration","data":100.0}),
            )
            .await;
            send(
                &mut writer,
                json!({"event":"property-change","name":"time-pos","data":21.0}),
            )
            .await;
            send(
                &mut writer,
                json!({"event":"property-change","name":"pause","data":true}),
            )
            .await;
            if !disconnect {
                let _ = released.await;
            }
        });
        until_status(&mut status, |value| value["paused"] == true).await;
        if disconnect {
            assert!(runner.await.unwrap().is_err());
            assert_eq!(status.borrow()["state"], "error");
        } else {
            stop.cancel();
            timeout(Duration::from_secs(2), runner)
                .await
                .expect("final snapshot must be bounded")
                .unwrap()
                .unwrap();
            assert!(
                status.borrow()["warning"]
                    .as_str()
                    .unwrap()
                    .contains("超时")
            );
        }
        let _ = release.send(());
        player.await.unwrap();
        assert_eq!(
            store.read("history", json!([])).unwrap()[0]["position"],
            21.0
        );
    }
}

#[tokio::test]
async fn failed_resume_does_not_replace_history_with_the_initial_zero() {
    let folder = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(folder.path()).unwrap());
    let mut previous = entry(0);
    previous["position"] = json!(40);
    previous["duration"] = json!(100);
    store.record_history(previous).unwrap();
    let (client, server) = tokio::io::duplex(4096);
    let connection = MpvConnection::new(store.clone(), "failed-seek", entry(0), false).unwrap();
    let player = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut lines = BufReader::new(reader).lines();
        for _ in 0..4 {
            let request: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            send(
                &mut writer,
                json!({"request_id":request["request_id"],"error":"success"}),
            )
            .await;
        }
        send(
            &mut writer,
            json!({"event":"property-change","name":"duration","data":100.0}),
        )
        .await;
        send(
            &mut writer,
            json!({"event":"property-change","name":"time-pos","data":0.0}),
        )
        .await;
        let request: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(request["command"][0], "seek");
        send(
            &mut writer,
            json!({"request_id":request["request_id"],"error":"error running command"}),
        )
        .await;
    });
    assert!(
        connection
            .run(client, CancellationToken::new())
            .await
            .is_err()
    );
    player.await.unwrap();
    assert_eq!(store.read("history", json!([])).unwrap()[0]["position"], 40);
}

#[cfg(windows)]
#[tokio::test]
async fn windows_named_pipe_supports_fragmented_events_resume_and_final_save() {
    use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};
    let folder = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(folder.path()).unwrap());
    let mut previous = entry(0);
    previous["position"] = json!(40);
    previous["duration"] = json!(100);
    store.record_history(previous).unwrap();
    let pipe = format!(r"\\.\pipe\crayon-mpv-contract-{}", uuid::Uuid::new_v4());
    let server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&pipe)
        .unwrap();
    let client = ClientOptions::new().open(&pipe).unwrap();
    server.connect().await.unwrap();
    let connection = MpvConnection::new(store.clone(), "pipe", entry(0), false).unwrap();
    let mut status = connection.subscribe();
    let stop = CancellationToken::new();
    let runner = tokio::spawn(connection.run(client, stop.clone()));
    let player = tokio::spawn(fake_player(server, 42.0, 44.0, Some(40.0)));
    until_status(&mut status, |value| value["paused"] == true).await;
    stop.cancel();
    runner.await.unwrap().unwrap();
    player.await.unwrap();
    assert_eq!(
        store.read("history", json!([])).unwrap()[0]["position"],
        44.0
    );
}

#[cfg(windows)]
#[tokio::test]
#[ignore = "requires CRAYON_REAL_MPV and CRAYON_REAL_MEDIA_URL; launches a real player"]
async fn real_mpv_manager_saves_resumes_isolates_sessions_and_reaps_processes() {
    use crayon_core::mpv::ExternalPlayback;
    use std::os::windows::process::CommandExt;

    fn process_count() -> usize {
        let output = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "@(Get-CimInstance Win32_Process -Filter \"Name = 'mpv.exe'\" | Where-Object { $_.ExecutablePath -eq $env:CRAYON_REAL_MPV }).Count",
            ])
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(output.status.success(), "process inspection failed");
        String::from_utf8(output.stdout)
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    async fn progress(manager: &ExternalPlayback, predicate: impl Fn(&Value) -> bool) -> Value {
        let mut status = manager.subscribe();
        timeout(Duration::from_secs(12), async {
            loop {
                let value = status.borrow().clone();
                assert_ne!(value["state"], "error", "{value}");
                if predicate(&value) {
                    return value;
                }
                status.changed().await.unwrap();
            }
        })
        .await
        .expect("real mpv progress timed out")
    }

    let command = std::env::var("CRAYON_REAL_MPV").expect("set a real mpv.exe path");
    let address = std::env::var("CRAYON_REAL_MEDIA_URL").expect("set a playable test media URL");
    assert!(std::path::Path::new(&command).is_file());
    assert_eq!(
        process_count(),
        0,
        "test binary must not already be running"
    );
    let folder = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(folder.path()).unwrap());
    store
        .save_settings(&json!({"externalPlayer":{"command":command,"args":[
            "--no-config","--load-scripts=no","--vo=null","--ao=null","--keep-open=yes",
            "--msg-level=all=warn","--","{url}"
        ]}}))
        .unwrap();
    let manager = Arc::new(ExternalPlayback::new(store.clone()));
    let pending_cancel = CancellationToken::new();
    let pending_manager = manager.clone();
    let pending_address = address.clone();
    let pending_token = pending_cancel.clone();
    let mut pending_status = manager.subscribe();
    let pending = tokio::spawn(async move {
        pending_manager
            .open_cancellable(
                &pending_address,
                "取消初始化",
                entry(0),
                false,
                &pending_token,
            )
            .await
    });
    until_status(&mut pending_status, |value| value["state"] == "starting").await;
    pending_cancel.cancel();
    let cancelled = timeout(Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap();
    assert!(
        cancelled.is_err(),
        "cancelled initialization must not succeed"
    );
    assert_eq!(process_count(), 0, "cancelled initialization must reap mpv");
    let first = manager
        .open(&address, "真实 mpv 测试", entry(0), false)
        .await
        .unwrap();
    progress(&manager, |value| {
        value["position"].as_f64().unwrap_or(0.0) >= 4.0
    })
    .await;
    assert_eq!(process_count(), 1);
    manager.finish().await.unwrap();
    assert_eq!(process_count(), 0, "finish must reap the real mpv process");
    let saved: Value =
        serde_json::from_str(&std::fs::read_to_string(folder.path().join("history.json")).unwrap())
            .unwrap();
    let position = saved[0]["position"].as_f64().unwrap();
    assert!((4.0..11.0).contains(&position), "{saved}");
    assert_eq!(saved[0]["epIndex"], 0);
    assert!(saved[0]["duration"].as_f64().unwrap() >= 11.0);

    let resumed_cancel = CancellationToken::new();
    let resumed = manager
        .open_cancellable(&address, "真实 mpv 续播", entry(0), false, &resumed_cancel)
        .await
        .unwrap();
    assert_ne!(first["session"], resumed["session"]);
    let confirmed = progress(&manager, |value| {
        value["position"].as_f64().unwrap_or(0.0) >= position - 0.5
    })
    .await;
    assert!(confirmed["position"].as_f64().unwrap() < position + 3.0);
    let next = manager
        .open(&address, "真实 mpv 新集", entry(1), false)
        .await
        .unwrap();
    assert_ne!(resumed["session"], next["session"]);
    resumed_cancel.cancel();
    manager
        .stop_session(resumed["session"].as_str().unwrap())
        .await
        .unwrap();
    let already_cancelled = CancellationToken::new();
    already_cancelled.cancel();
    assert!(
        manager
            .open_cancellable(&address, "取消旧请求", entry(2), false, &already_cancelled)
            .await
            .is_err()
    );
    let next_progress = progress(&manager, |value| {
        value["session"] == next["session"] && value["position"].as_f64().unwrap_or(0.0) >= 1.0
    })
    .await;
    assert!(next_progress["position"].as_f64().unwrap() < position);
    assert_eq!(process_count(), 1, "only the new session may remain");
    manager.finish().await.unwrap();
    assert_eq!(process_count(), 0);
    let final_saved: Value =
        serde_json::from_str(&std::fs::read_to_string(folder.path().join("history.json")).unwrap())
            .unwrap();
    assert_eq!(final_saved[0]["epIndex"], 1);
    assert!(final_saved[0]["position"].as_f64().unwrap() >= 1.0);
    eprintln!(
        "[PASS] real mpv: saved={position:.0}s, resumed={:.2}s, next-episode={:.2}s, process counts=0/1/0/1/0",
        confirmed["position"].as_f64().unwrap(),
        next_progress["position"].as_f64().unwrap()
    );
}
