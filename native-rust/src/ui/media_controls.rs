use super::*;
use std::sync::Mutex;

#[component]
pub(super) fn PlaybackStage(
    address: String,
    title: String,
    #[props(default)] live: bool,
    #[props(default)] entry: Value,
    onsession: EventHandler<String>,
) -> Element {
    let ui = use_context::<Ui>();
    let mut session = use_signal(String::new);
    let mut external = use_signal(|| None::<String>);
    let mut switching = use_signal(|| false);
    let mut suspended = use_signal(|| false);
    let mut generation = use_signal(|| 0u64);
    let cancel = use_hook(move || ui.backend().shutdown.child_token());
    let owned = use_hook(|| Arc::new(Mutex::new(String::new())));
    let cleanup = owned.clone();
    let stop = cancel.clone();
    let svc = ui.backend();
    use_drop(move || {
        stop.cancel();
        let id = cleanup.lock().ok().map(|id| id.clone()).unwrap_or_default();
        if !id.is_empty() {
            dioxus::core::spawn_forever(async move {
                if let Err(error) = svc.external.stop_session(&id).await {
                    eprintln!("[mpv] 关闭播放页面失败：{error:#}");
                }
            });
        }
    });
    let active = external.read().clone();
    let launch_address = address.clone();
    let launch_cancel = cancel.clone();
    let launch_owned = owned.clone();
    let return_cancel = cancel.clone();
    let return_owned = owned.clone();
    let last = ui.player_event.read().clone();
    let failure = if active.is_none() && s(&last, "session") == *session.read() {
        playback_failure(&last)
    } else {
        String::new()
    };
    rsx! {
        div { class: "player-stage",
            if let Some(id) = active.clone() {
                ExternalStatus { session: id }
            } else if *suspended.read() {
                div { class: "player-stage-note", span { class: "spin" } "正在切换外部播放器…" }
            } else {
                runtime::PlayerSurface {
                    key: "{generation}", address: address.clone(), entry: entry.clone(), live,
                    class: "player-stage-inner",
                    onsession: move |id: String| { session.set(id.clone()); onsession.call(id); }
                }
            }
        }
        div { class: "player-bar",
            if active.is_some() {
                button { class: "btn btn-sm", disabled: *switching.read(), onclick: move |_| {
                    let Some(id) = external.peek().clone() else { return; };
                    switching.set(true);
                    let cancel = return_cancel.clone(); let owned = return_owned.clone();
                    dioxus::core::spawn_forever(async move {
                        let result = ui.backend().external.stop_session(&id).await;
                        if cancel.is_cancelled() { return; }
                        if ui.report(result).is_some() {
                            if let Ok(mut id) = owned.lock() { id.clear(); }
                            ui.refresh_library(); external.set(None); suspended.set(false); generation += 1;
                        }
                        switching.set(false);
                    });
                }, "返回内置播放器" }
            } else {
                button { class: "btn btn-sm", disabled: *switching.read() || address.is_empty() || session.read().is_empty(), onclick: move |_| {
                    switching.set(true);
                    let address = launch_address.clone(); let title = title.clone(); let entry = entry.clone();
                    let cancel = launch_cancel.clone(); let owned = launch_owned.clone(); let id = session.peek().clone();
                    dioxus::core::spawn_forever(async move {
                        let svc = ui.backend();
                        let result = async {
                            svc.suspend_player(&id, &cancel).await?;
                            suspended.set(true);
                            svc.external.open_cancellable(&address, &title, entry, live, &cancel).await
                        }.await;
                        if cancel.is_cancelled() {
                            if let Ok(state) = result
                                && let Err(error) = svc.external.stop_session(&s(&state, "session")).await {
                                eprintln!("[mpv] 取消播放页面失败：{error:#}");
                            }
                            return;
                        }
                        if let Some(state) = ui.report(result) {
                            let id = s(&state, "session");
                            if let Ok(mut current) = owned.lock() { *current = id.clone(); }
                            external.set(Some(id));
                            ui.refresh_library();
                        } else { suspended.set(false); generation += 1; }
                        switching.set(false);
                    });
                }, if *switching.read() { span { class: "spin" } "正在切换…" } else { Icon { name: "external", size: 13 } "外部播放器" } }
            }
            span { class: "field-hint", if live { "直播不写入观看历史" } else { "mpv 支持进度回传与续播" } }
        }
        if !failure.is_empty() {
            div { class: "banner banner-danger", "{failure}"
                button { class: "btn btn-sm", disabled: *switching.read(), onclick: move |_| generation += 1, "重试播放" }
            }
        }
        MediaDiagnostics { address }
    }
}

fn playback_failure(event: &Value) -> String {
    match s(event, "event").as_str() {
        "error" => {
            let label = match s(event, "errorCategory").as_str() {
                "network" => "媒体网络请求失败",
                "decode" => "当前环境无法解码此音视频编码",
                "drm" => "此片源需要内容授权",
                "format" => "媒体格式或清单无效",
                "capability" => "当前环境缺少此播放能力",
                _ => "视频加载失败",
            };
            let detail = s(event, "error");
            if detail.is_empty() {
                label.into()
            } else {
                format!("{label}：{detail}")
            }
        }
        "hlsError" if event["fatal"] == true && event["recovered"] != true => {
            match s(event, "errorType").as_str() {
                "networkError" => "HLS 网络请求失败，请重试或换线".into(),
                "mediaError" => "HLS 解码失败，请换线或使用外部播放器".into(),
                _ => "HLS 加载失败，请查看媒体诊断或换线".into(),
            }
        }
        _ => String::new(),
    }
}

#[component]
fn ExternalStatus(session: String) -> Element {
    let ui = use_context::<Ui>();
    let current = ui.external_player.read().clone();
    let state = if s(&current, "session") == session {
        current
    } else {
        json!({"state":"starting"})
    };
    let label = match s(&state, "state").as_str() {
        "playing" => "外部播放器正在播放",
        "paused" => "外部播放器已暂停",
        "ended" => "播放结束",
        "stopped" => "外部播放已停止",
        "error" => "外部播放遇到问题",
        "unlinked" => "已交给外部播放器",
        "stopping" => "正在保存进度并关闭",
        _ => "正在连接外部播放器",
    };
    let progress = format!(
        "{} / {}",
        clock(number(&state["position"], 0.0)),
        clock(number(&state["duration"], 0.0))
    );
    rsx! { div { class: "player-stage-inner live-stage-msg", role: "status",
        Icon { name: "external", size: 24 }
        div { class: "live-stage-msg-title", "{label}" }
        if state["progressLinked"] == true {
            div { class: "live-stage-msg-desc mono", "{progress}" }
            if state["live"] != true && !state["savedAt"].is_null() { span { class: "tag tag-success", "观看进度已保存" } }
        } else if state["state"] == "unlinked" {
            div { class: "live-stage-msg-desc", "此播放器不回传进度，可在设置中选择 mpv 开启联动。" }
        }
        if !s(&state, "error").is_empty() { div { class: "live-stage-msg-desc", {s(&state, "error")} } }
        if !s(&state, "warning").is_empty() { div { class: "live-stage-msg-desc", {s(&state, "warning")} } }
    } }
}

#[component]
fn MediaDiagnostics(address: String) -> Element {
    let ui = use_context::<Ui>();
    let mut result = use_signal(|| Value::Null);
    let mut busy = use_signal(|| false);
    let cancel = use_hook(move || ui.backend().shutdown.child_token());
    let cleanup = cancel.clone();
    use_drop(move || cleanup.cancel());
    let report = result.read().clone();
    let format = s(&report, "container");
    let evidence = array(&report["evidence"])
        .iter()
        .map(|item| {
            let source = match s(item, "source").as_str() {
                "signature" => "内容特征",
                "mime" => "响应类型",
                "url" => "地址后缀",
                _ => "未知",
            };
            format!("{source}：{}", s(item, "value"))
        })
        .collect::<Vec<_>>()
        .join(" · ");
    let sampling = format!(
        "{:.0} ms · 采样 {:.0} 字节",
        number(&report["ms"], 0.0),
        number(&report["sample"]["bytes"], 0.0)
    );
    let error = format!(
        "{}：{}",
        diagnostic_label(&s(&report, "errorCategory")),
        s(&report, "error")
    );
    rsx! { div { class: "panel panel-body-pad",
        div { class: "hstack",
            button { class: "btn btn-sm", disabled: *busy.read() || address.is_empty(), onclick: move |_| {
                busy.set(true);
                let address = address.clone(); let cancel = cancel.clone();
                spawn(async move {
                    let probe = ui.backend().media.inspect(&address, &cancel).await;
                    if cancel.is_cancelled() { return; }
                    if let Some(value) = ui.report(probe) { result.set(value); }
                    busy.set(false);
                });
            }, if *busy.read() { span { class: "spin" } "正在诊断…" } else { "媒体诊断" } }
            span { class: "field-hint", "检查格式与连通性，解码能力以实际播放为准" }
        }
        if !report.is_null() {
            div { class: "hstack",
                span { class: "tag tag-outline", if format.is_empty() { "未识别格式" } else { "{format}" } }
                if let Some(status) = report["http"]["status"].as_u64() { span { class: "text-3 fs-12", "HTTP {status}" } }
                span { class: "text-3 fs-12", "{sampling}" }
            }
            if !evidence.is_empty() { div { class: "field-hint", "{evidence}" } }
            if !s(&report["http"], "contentType").is_empty() { div { class: "field-hint mono", {s(&report["http"], "contentType")} } }
            if report["ok"] != true { div { class: "banner banner-danger", "{error}" } }
        }
    } }
}

fn diagnostic_label(category: &str) -> &'static str {
    match category {
        "http_status" => "服务器拒绝请求",
        "non_media" => "响应为网页或错误内容",
        "unknown_media" => "暂未识别媒体类型",
        "network" => "网络请求失败",
        "timeout" => "探测超时",
        "cancelled" => "探测已取消",
        "unsupported_protocol" => "此协议需要外部播放器",
        "invalid_address" => "播放地址无效",
        _ => "诊断失败",
    }
}
