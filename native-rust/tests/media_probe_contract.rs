use axum::{
    Router,
    body::Body,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use crayon_core::{
    catalogue::Catalogue, live::Live, media::Media, sources::Sources, storage::Store,
};
use futures_util::{StreamExt, stream};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

#[test]
fn signatures_override_misleading_mime_and_signed_url_suffixes() {
    use crayon_core::media_probe::{identify, kind_from_url};
    let mp4 = b"\0\0\0\x18ftypisom\0\0\0\0isommp42";
    let webm = b"\x1a\x45\xdf\xa3\x42\x82\x84webm";
    let mkv = b"\x1a\x45\xdf\xa3\x42\x82\x88matroska";
    let mut ts = vec![0; 188 * 3];
    for offset in [0, 188, 376] {
        ts[offset] = 0x47;
    }
    for (bytes, kind) in [
        (mp4.as_slice(), "mp4"),
        (webm.as_slice(), "webm"),
        (mkv.as_slice(), "mkv"),
        (ts.as_slice(), "ts"),
        (b"FLV\x01\x05\0\0\0\x09".as_slice(), "flv"),
        (b"ID3\x04\0\0".as_slice(), "audio"),
        (b"#EXTM3U\n#EXT-X-TARGETDURATION:6\n".as_slice(), "hls"),
        (
            b"<?xml version=\"1.0\"?><MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\"/>".as_slice(),
            "dash",
        ),
        (
            b"<!DOCTYPE html><html>access denied</html>".as_slice(),
            "html",
        ),
    ] {
        let found = identify(
            "https://media.invalid/token?next=film.m3u8",
            "application/octet-stream",
            bytes,
        );
        assert_eq!(found["kind"], kind, "{found}");
        assert_eq!(found["confidence"], "signature");
        assert_eq!(found["decodeVerified"], false);
        assert!(found["browserPlayable"].is_null());
    }
    assert_eq!(
        identify(
            "https://media.invalid/fake.mp4",
            "video/mp4",
            b"<html>denied</html>"
        )["kind"],
        "html"
    );
    assert_eq!(
        identify(
            "https://media.invalid/token",
            "application/dash+xml; charset=utf-8",
            b""
        )["kind"],
        "dash"
    );
    assert_eq!(
        kind_from_url("https://media.invalid/MAIN.MPD?token=secret#x"),
        "dash"
    );
    assert_eq!(
        kind_from_url("https://media.invalid/stream?url=x.mp4"),
        "unknown"
    );
    assert_eq!(
        identify("https://media.invalid/token", "", b"<MPD/>")["kind"],
        "dash"
    );
    assert_eq!(
        identify("https://media.invalid/token", "", b"#EXTM3Uinvalid")["kind"],
        "unknown"
    );
    assert_eq!(
        identify(
            "https://media.invalid/token",
            "video/mp4",
            b"<!-- blocked -->\n<html>denied</html>"
        )["kind"],
        "html"
    );
    assert_eq!(
        identify(
            "https://media.invalid/token",
            "application/octet-stream",
            b"\xff\xfb\x90\x00"
        )["kind"],
        "audio"
    );
    assert_eq!(
        identify(
            "https://media.invalid/token",
            "",
            b"\0\0\0\x18ftypavif\0\0\0\0avif"
        )["kind"],
        "unknown"
    );
    assert_eq!(
        identify(
            "https://media.invalid/token",
            "application/octet-stream",
            b"garbage"
        )["kind"],
        "unknown"
    );
    assert!(
        !identify(
            "https://media.invalid/token?token=private",
            "video/mp4",
            mp4
        )
        .to_string()
        .contains("private")
    );
}

#[tokio::test]
async fn inspect_handles_extensionless_redirects_without_leaking_private_headers() {
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let destination = Server::new(Router::new().route(
        "/signed",
        get(move |headers: HeaderMap| {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                assert!(headers.get("authorization").is_none());
                assert!(headers.get("cookie").is_none());
                assert_eq!(headers["range"], "bytes=0-4095");
                (
                    [("content-type", "application/octet-stream")],
                    b"\0\0\0\x18ftypisom\0\0\0\0isommp42".to_vec(),
                )
            }
        }),
    ))
    .await;
    let target = format!("{}/signed?token=private-fixture", destination.address);
    let source = Server::new(Router::new().route(
        "/redirect",
        get(move |headers: HeaderMap| {
            let target = target.clone();
            async move {
                assert_eq!(headers["authorization"], "Bearer fixture");
                assert_eq!(headers["cookie"], "session=fixture");
                axum::response::Redirect::temporary(&target)
            }
        }),
    ))
    .await;
    let folder = tempfile::tempdir().unwrap();
    let media = media(Arc::new(Store::open(folder.path()).unwrap()));
    media.register(
        &source.address,
        &json!({"Authorization":"Bearer fixture","Cookie":"session=fixture"}),
    );
    let report = media
        .inspect(
            &format!("{}/redirect", source.address),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["kind"], "mp4");
    assert_eq!(report["http"]["redirected"], true);
    assert_eq!(report["http"]["status"], 200);
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    for secret in ["private-fixture", "Bearer fixture", "session=fixture"] {
        assert!(!report.to_string().contains(secret));
    }
}

#[tokio::test]
async fn inspect_rejects_disguised_html_errors_and_unknown_payloads() {
    let server = Server::new(
        Router::new()
            .route(
                "/fake.mp4",
                get(|| async {
                    (
                        [("content-type", "video/mp4")],
                        "<!doctype html><html>sign in</html>",
                    )
                }),
            )
            .route(
                "/unknown",
                get(|| async {
                    (
                        [("content-type", "application/octet-stream")],
                        "not a media header",
                    )
                }),
            )
            .route(
                "/empty.mp4",
                get(|| async { ([("content-type", "video/mp4")], "") }),
            )
            .route(
                "/bad.m3u8",
                get(|| async {
                    (
                        StatusCode::FORBIDDEN,
                        [("content-type", "text/html")],
                        "denied",
                    )
                }),
            ),
    )
    .await;
    let folder = tempfile::tempdir().unwrap();
    let media = media(Arc::new(Store::open(folder.path()).unwrap()));
    for (path, kind, category, status) in [
        ("/fake.mp4", "html", "non_media", 200),
        ("/unknown", "unknown", "unknown_media", 200),
        ("/empty.mp4", "mp4", "unknown_media", 200),
        ("/bad.m3u8", "html", "http_status", 403),
    ] {
        let report = media
            .inspect(
                &format!("{}{path}", server.address),
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(report["ok"], false, "{report}");
        assert_eq!(report["kind"], kind);
        assert_eq!(report["errorCategory"], category);
        assert_eq!(report["http"]["status"], status);
        assert_eq!(report["decodeVerified"], false);
    }
    let report = media
        .inspect("rtsp://media.invalid/live", &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(report["errorCategory"], "unsupported_protocol");
    let report = media
        .inspect("not an address", &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(report["errorCategory"], "invalid_address");
}

#[tokio::test]
async fn inspect_limits_range_and_stops_after_a_streaming_container_header() {
    let server = Server::new(Router::new()
        .route("/hls", get(|headers: HeaderMap| async move {
            assert_eq!(headers["range"], "bytes=0-4095");
            let body = "#EXTM3U\n#EXT-X-STREAM-INF:RESOLUTION=1920x1080,BANDWIDTH=2300000\nvariant.m3u8\n".to_owned() + &" ".repeat(32 * 1024);
            Response::builder().header("content-type", "application/vnd.apple.mpegurl").body(Body::from_stream(stream::once(async move { Ok::<_, std::io::Error>(body) }).chain(stream::pending()))).unwrap()
        }))
        .route("/flv", get(|| async {
            Response::builder().header("content-type", "application/octet-stream").body(Body::from_stream(stream::once(async { Ok::<_, std::io::Error>(b"FLV\x01\x05\0\0\0\x09".to_vec()) }).chain(stream::pending()))).unwrap()
        }))).await;
    let folder = tempfile::tempdir().unwrap();
    let media = media(Arc::new(Store::open(folder.path()).unwrap()));
    let started = Instant::now();
    let hls = media
        .inspect(
            &format!("{}/hls", server.address),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(hls["ok"], true, "{hls}");
    assert_eq!(hls["sample"]["bytes"], 4096);
    assert_eq!(hls["sample"]["truncated"], true);
    assert_eq!(hls["height"], 1080);
    let flv = media
        .inspect(
            &format!("{}/flv", server.address),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(flv["ok"], true, "{flv}");
    assert_eq!(flv["kind"], "flv");
    assert_eq!(flv["sample"]["bytes"], 9);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn inspect_cancels_and_times_out_during_headers_or_body() {
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = hits.clone();
    let server = Server::new(
        Router::new()
            .route(
                "/headers",
                get(move || {
                    let seen = seen.clone();
                    async move {
                        seen.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        "slow"
                    }
                }),
            )
            .route(
                "/body",
                get(|| async {
                    Response::builder()
                        .header("content-type", "application/octet-stream")
                        .body(Body::from_stream(
                            stream::once(async { Ok::<_, std::io::Error>("x") })
                                .chain(stream::pending()),
                        ))
                        .unwrap()
                }),
            ),
    )
    .await;
    let folder = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(folder.path()).unwrap());
    store.save_settings(&json!({"timeout":1})).unwrap();
    let media = media(store);
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let report = media
        .inspect(&format!("{}/headers", server.address), &cancelled)
        .await
        .unwrap();
    assert_eq!(report["errorCategory"], "cancelled");
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    for path in ["/headers", "/body"] {
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        let timer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            stop.cancel();
        });
        let report = media
            .inspect(&format!("{}{path}", server.address), &cancel)
            .await
            .unwrap();
        timer.await.unwrap();
        assert_eq!(report["errorCategory"], "cancelled", "{report}");
        assert!(report["ms"].as_u64().unwrap() < 1000);
        let started = Instant::now();
        let report = media
            .inspect(
                &format!("{}{path}", server.address),
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(report["errorCategory"], "timeout", "{report}");
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}

#[tokio::test]
async fn resolve_recognizes_extensionless_streams_without_waiting_for_the_body_to_end() {
    let hits = Arc::new(AtomicUsize::new(0));
    let ranged = Arc::new(AtomicUsize::new(0));
    let seen = hits.clone();
    let seen_ranges = ranged.clone();
    let server = Server::new(Router::new().route(
        "/{kind}",
        get(
            move |axum::extract::Path(kind): axum::extract::Path<String>, headers: HeaderMap| {
                let seen = seen.clone();
                let seen_ranges = seen_ranges.clone();
                async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    if headers.get("range").and_then(|v| v.to_str().ok()) == Some("bytes=0-4095") {
                        seen_ranges.fetch_add(1, Ordering::SeqCst);
                    }
                    let prefix = match kind.as_str() {
                        "flv" => b"FLV\x01\x05\0\0\0\x09".to_vec(),
                        "mp4" => b"\0\0\0\x18ftypisom\0\0\0\0isommp42".to_vec(),
                        "hls" => ("#EXTM3U\n#EXTINF:5,\npart.ts\n".to_owned()
                            + &" ".repeat(32 * 1024))
                            .into_bytes(),
                        "dash" => ("<MPD><Period/></MPD>".to_owned() + &" ".repeat(32 * 1024))
                            .into_bytes(),
                        _ => unreachable!(),
                    };
                    Response::builder()
                        .header("content-type", "application/octet-stream")
                        .body(Body::from_stream(
                            stream::once(async { Ok::<_, std::io::Error>(prefix) })
                                .chain(stream::pending()),
                        ))
                        .unwrap()
                }
            },
        ),
    ))
    .await;
    let folder = tempfile::tempdir().unwrap();
    let media = media(Arc::new(Store::open(folder.path()).unwrap()));
    for kind in ["flv", "mp4", "hls", "dash"] {
        let address = format!("{}/{kind}?token=fixture", server.address);
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            media.resolve(&address, &json!({}), &CancellationToken::new()),
        )
        .await
        .expect("resolve must stop after a bounded media sample")
        .unwrap();
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["url"], address);
        assert_eq!(result["direct"], true);
    }
    assert_eq!(hits.load(Ordering::SeqCst), 4);
    assert_eq!(ranged.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn resolve_applies_site_headers_before_probing_and_keeps_them_for_playback() {
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = hits.clone();
    let server = Server::new(Router::new().route(
        "/stream",
        get(move |headers: HeaderMap| {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                if headers.get("authorization").and_then(|v| v.to_str().ok())
                    != Some("Bearer fixture")
                    || headers.get("cookie").and_then(|v| v.to_str().ok())
                        != Some("session=fixture")
                    || headers.get("referer").and_then(|v| v.to_str().ok())
                        != Some("https://page.invalid/watch")
                    || headers.get("user-agent").and_then(|v| v.to_str().ok())
                        != Some("fixture-agent")
                {
                    return (StatusCode::FORBIDDEN, "<html>denied</html>").into_response();
                }
                assert_eq!(headers["range"], "bytes=0-4095");
                (
                    [("content-type", "application/octet-stream")],
                    b"\0\0\0\x18ftypisom\0\0\0\0isommp42".to_vec(),
                )
                    .into_response()
            }
        }),
    ))
    .await;
    let folder = tempfile::tempdir().unwrap();
    let media = media(Arc::new(Store::open(folder.path()).unwrap()));
    let header = json!({
        "Authorization":"Bearer fixture", "Cookie":"session=fixture",
        "Referer":"https://page.invalid/watch", "User-Agent":"fixture-agent"
    });
    let site = media
        .catalogue
        .sources
        .add_local(&json!({"api":format!("{}/api", server.address),"header":header}))
        .unwrap();
    let address = format!("{}/stream", server.address);
    let result = media
        .resolve(
            &address,
            &json!({"siteKey":site["key"]}),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(result["ok"], true, "{result}");
    let playback_headers = media.headers(&address).unwrap();
    for key in ["Authorization", "Cookie", "Referer", "User-Agent"] {
        assert_eq!(playback_headers[key], header[key]);
    }
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn resolve_preserves_existing_origin_grants_without_a_site() {
    let server = Server::new(Router::new().route(
        "/stream",
        get(|headers: HeaderMap| async move {
            assert_eq!(headers["authorization"], "Bearer fixture");
            assert_eq!(headers["cookie"], "session=fixture");
            (
                [("content-type", "application/octet-stream")],
                b"\0\0\0\x18ftypisom\0\0\0\0isommp42".to_vec(),
            )
        }),
    ))
    .await;
    let folder = tempfile::tempdir().unwrap();
    let media = media(Arc::new(Store::open(folder.path()).unwrap()));
    let address = format!("{}/stream", server.address);
    let header = json!({"Authorization":"Bearer fixture","Cookie":"session=fixture"});
    media.register(&address, &header);
    let result = media
        .resolve(&address, &json!({}), &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(result["ok"], true, "{result}");
    let playback_headers = media.headers(&address).unwrap();
    assert_eq!(playback_headers["Authorization"], header["Authorization"]);
    assert_eq!(playback_headers["Cookie"], header["Cookie"]);
}

#[tokio::test]
async fn resolve_stops_on_cancellation_without_retrying_the_page() {
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = hits.clone();
    let body_hits = hits.clone();
    let server = Server::new(
        Router::new()
            .route(
                "/headers",
                get(move || {
                    let seen = seen.clone();
                    async move {
                        seen.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        "slow"
                    }
                }),
            )
            .route(
                "/body",
                get(move || {
                    let body_hits = body_hits.clone();
                    async move {
                        body_hits.fetch_add(1, Ordering::SeqCst);
                        Response::builder()
                            .body(Body::from_stream(
                                stream::once(async { Ok::<_, std::io::Error>("x") })
                                    .chain(stream::pending()),
                            ))
                            .unwrap()
                    }
                }),
            ),
    )
    .await;
    let folder = tempfile::tempdir().unwrap();
    let media = media(Arc::new(Store::open(folder.path()).unwrap()));
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let error = media
        .resolve(
            &format!("{}/headers", server.address),
            &json!({}),
            &cancelled,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("取消"));
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    for (index, path) in ["/headers", "/body"].into_iter().enumerate() {
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        let timer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            stop.cancel();
        });
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            media.resolve(&format!("{}{path}", server.address), &json!({}), &cancel),
        )
        .await
        .expect("cancelled resolve must stop promptly");
        timer.await.unwrap();
        assert!(result.unwrap_err().to_string().contains("取消"));
        assert_eq!(hits.load(Ordering::SeqCst), index + 1);
    }
}

#[tokio::test]
async fn resolve_falls_back_to_the_full_html_page_after_a_bounded_sample() {
    let probes = Arc::new(AtomicUsize::new(0));
    let pages = Arc::new(AtomicUsize::new(0));
    let seen_probes = probes.clone();
    let seen_pages = pages.clone();
    let target = "https://media.invalid/film.mp4?token=fixture";
    let page = format!(
        "<html>{}<video src=\"{target}\"></video></html>",
        " ".repeat(8 * 1024)
    );
    let server = Server::new(Router::new().route(
        "/watch",
        get(move |headers: HeaderMap| {
            let seen_probes = seen_probes.clone();
            let seen_pages = seen_pages.clone();
            let page = page.clone();
            async move {
                if headers.get("range").is_some() {
                    seen_probes.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(headers["range"], "bytes=0-4095");
                    (
                        StatusCode::PARTIAL_CONTENT,
                        [("content-type", "text/html")],
                        page[..4096].to_owned(),
                    )
                        .into_response()
                } else {
                    seen_pages.fetch_add(1, Ordering::SeqCst);
                    ([("content-type", "text/html")], page).into_response()
                }
            }
        }),
    ))
    .await;
    let folder = tempfile::tempdir().unwrap();
    let media = media(Arc::new(Store::open(folder.path()).unwrap()));
    let result = media
        .resolve(
            &format!("{}/watch", server.address),
            &json!({}),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["url"], target);
    assert_eq!(result["direct"], false);
    assert_eq!(probes.load(Ordering::SeqCst), 1);
    assert_eq!(pages.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn dash_proxy_exposes_redirected_manifest_base_and_forwards_segment_ranges() {
    let segment_hits = Arc::new(AtomicUsize::new(0));
    let seen = segment_hits.clone();
    let destination = Server::new(Router::new()
        .route("/dir/main.mpd", get(|headers: HeaderMap| async move {
            assert!(headers.get("authorization").is_none());
            ([("content-type", "application/dash+xml")], "<MPD><Period><AdaptationSet><Representation><BaseURL>parts/</BaseURL><SegmentList><Initialization sourceURL=\"init.mp4\" range=\"0-9\"/><SegmentURL media=\"a.m4s\" mediaRange=\"10-20\"/></SegmentList></Representation></AdaptationSet></Period></MPD>")
        }))
        .route("/dir/parts/a.m4s", get(move |headers: HeaderMap| {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                assert!(headers.get("authorization").is_none());
                assert_eq!(headers["range"], "bytes=10-20");
                (StatusCode::PARTIAL_CONTENT, [("content-range", "bytes 10-20/30")], "01234567890").into_response()
            }
        }))).await;
    let target = format!("{}/dir/main.mpd?token=fixture", destination.address);
    let redirect_target = target.clone();
    let source = Server::new(Router::new().route(
        "/entry",
        get(move |headers: HeaderMap| {
            let target = redirect_target.clone();
            async move {
                assert_eq!(headers["authorization"], "Bearer fixture");
                axum::response::Redirect::temporary(&target)
            }
        }),
    ))
    .await;
    let folder = tempfile::tempdir().unwrap();
    let media = media(Arc::new(Store::open(folder.path()).unwrap()));
    media.register(&source.address, &json!({"Authorization":"Bearer fixture"}));
    let proxy = media.start_proxy().await.unwrap();
    let client = reqwest::Client::new();
    let response = client
        .get(proxy.url(&format!("{}/entry", source.address)))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.headers()["x-crayon-media-url"],
        crayon_core::spider::encode(&target)
    );
    assert!(
        response.headers()["access-control-expose-headers"]
            .to_str()
            .unwrap()
            .contains("X-Crayon-Media-Url")
    );
    assert!(
        response
            .text()
            .await
            .unwrap()
            .contains("mediaRange=\"10-20\"")
    );
    let segment = url::Url::parse(&target)
        .unwrap()
        .join("parts/a.m4s")
        .unwrap();
    let response = client
        .get(proxy.url(segment.as_str()))
        .header("Range", "bytes=10-20")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.headers()["content-range"], "bytes 10-20/30");
    assert_eq!(response.text().await.unwrap(), "01234567890");
    assert_eq!(segment_hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn proxy_recognizes_extensionless_hls_and_preserves_unknown_range_bytes() {
    let server = Server::new(
        Router::new()
            .route(
                "/token/manifest",
                get(|| async {
                    let chunks = [
                        b"#EX".to_vec(),
                        b"TM3U\n#EXTINF:5,\nparts/one.ts?token=fixture\n".to_vec(),
                    ];
                    Response::builder()
                        .header("content-type", "application/octet-stream")
                        .body(Body::from_stream(stream::iter(
                            chunks.into_iter().map(Ok::<_, std::io::Error>),
                        )))
                        .unwrap()
                }),
            )
            .route(
                "/range",
                get(|headers: HeaderMap| async move {
                    assert_eq!(headers["range"], "bytes=2-4");
                    (
                        StatusCode::PARTIAL_CONTENT,
                        [
                            ("content-type", "application/octet-stream"),
                            ("content-range", "bytes 2-4/10"),
                        ],
                        "234",
                    )
                }),
            ),
    )
    .await;
    let folder = tempfile::tempdir().unwrap();
    let media = media(Arc::new(Store::open(folder.path()).unwrap()));
    let proxy = media.start_proxy().await.unwrap();
    let client = reqwest::Client::new();
    let response = client
        .get(proxy.url(&format!("{}/token/manifest?token=fixture", server.address)))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.headers()["content-type"],
        "application/vnd.apple.mpegurl"
    );
    let playlist = response.text().await.unwrap();
    assert!(playlist.contains(&proxy.base), "{playlist}");
    assert!(playlist.contains(&crayon_core::spider::encode(&format!(
        "{}/token/parts/one.ts?token=fixture",
        server.address
    ))));
    let response = client
        .get(proxy.url(&format!("{}/range", server.address)))
        .header("Range", "bytes=2-4")
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers()["content-range"], "bytes 2-4/10");
    assert_eq!(response.headers()["content-length"], "3");
    assert_eq!(response.text().await.unwrap(), "234");
}

struct Server {
    address: String,
    task: tokio::task::JoinHandle<()>,
}
impl Server {
    async fn new(router: Router) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self { address, task }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn media(store: Arc<Store>) -> Arc<Media> {
    Arc::new(
        Media::new(Arc::new(Catalogue::new(Arc::new(
            Sources::new(store).unwrap(),
        ))))
        .unwrap(),
    )
}

#[tokio::test]
async fn live_health_accepts_mp4_and_flv_without_claiming_a_decode() {
    let server = Server::new(
        Router::new()
            .route(
                "/film.mp4",
                get(|| async {
                    (
                        [("content-type", "video/mp4")],
                        b"\0\0\0\x18ftypisom\0\0\0\0isommp42".to_vec(),
                    )
                }),
            )
            .route(
                "/live.flv",
                get(|| async {
                    (
                        [("content-type", "video/x-flv")],
                        b"FLV\x01\x05\0\0\0\x09".to_vec(),
                    )
                }),
            ),
    )
    .await;
    let folder = tempfile::tempdir().unwrap();
    let live = Live::new(media(Arc::new(Store::open(folder.path()).unwrap())));
    let urls = [
        format!("{}/film.mp4", server.address),
        format!("{}/live.flv", server.address),
    ];
    let health = live.probe(&urls, true).await.unwrap();
    for url in &urls {
        assert_eq!(health[url]["ok"], true, "{}", health[url]);
        assert_ne!(health[url]["decodeVerified"], true);
    }
}
