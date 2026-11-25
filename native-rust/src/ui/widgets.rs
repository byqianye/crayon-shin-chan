use super::*;
pub(super) fn vod_key(item: &Value) -> String {
    format!("{}:{}", s(item, "siteKey"), s(item, "vod_id"))
}

pub(super) fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds % 3600 / 60,
            seconds % 60
        )
    } else {
        format!("{:02}:{:02}", seconds / 60, seconds % 60)
    }
}
pub(super) fn relative(at: &Value) -> String {
    let at = number(at, 0.0) as u64;
    if at == 0 {
        return "从未".into();
    }
    let seconds = now().saturating_sub(at) / 1000;
    if seconds < 60 {
        "刚刚".into()
    } else if seconds < 3600 {
        format!("{} 分钟前", seconds / 60)
    } else if seconds < 86400 {
        format!("{} 小时前", seconds / 3600)
    } else {
        format!("{} 天前", seconds / 86400)
    }
}
pub(super) fn percent(item: &Value) -> Option<u64> {
    let duration = number(&item["duration"], 0.0);
    if duration > 0.0 {
        Some(
            (number(&item["position"], 0.0) / duration * 100.0)
                .clamp(0.0, 100.0)
                .round() as u64,
        )
    } else {
        None
    }
}
pub(super) fn shade(hex: &str) -> String {
    let raw = hex.trim_start_matches('#');
    if raw.len() != 6 {
        return hex.into();
    }
    if let Ok(value) = u32::from_str_radix(raw, 16) {
        let r = ((value >> 16) & 255).saturating_sub(31);
        let g = ((value >> 8) & 255).saturating_sub(31);
        let b = (value & 255).saturating_sub(31);
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        hex.into()
    }
}
pub(super) fn type_label(site: &Value) -> &'static str {
    if !s(site, "spiderClass").is_empty() {
        "Spider"
    } else {
        match site["type"].as_i64().unwrap_or(2) {
            3 => "苹果CMS",
            4 => "JSON 接口",
            1 => "网页规则",
            0 => "Spider",
            _ => "其它",
        }
    }
}
pub(super) fn favorite(detail: &Value) -> Value {
    json!({"siteKey":detail["siteKey"],"siteName":detail["siteName"],"vodId":detail["vod_id"],"name":detail["vod_name"],"pic":detail["vod_pic"],"remarks":detail["vod_remarks"],"score":detail["vod_score"]})
}
pub(super) fn play_history(item: &Value) -> Page {
    Page::Player(
        s(item, "siteKey"),
        s(item, "vodId"),
        number(&item["lineIndex"], 0.0) as usize,
        number(&item["epIndex"], 0.0) as usize,
    )
}
pub(super) async fn confirm(message: &str) -> bool {
    document::eval(&format!("return window.confirm({});", json!(message)))
        .join::<bool>()
        .await
        .unwrap_or(false)
}
pub(super) async fn copy(ui: Ui, text: &str) {
    match document::eval(&format!(
        "await navigator.clipboard.writeText({});return true;",
        json!(text)
    ))
    .join::<bool>()
    .await
    {
        Ok(true) => ui.notify("已复制", "success"),
        _ => ui.notify("复制失败，请手动选择地址复制", "danger"),
    }
}

#[component]
pub(super) fn Icon(name: String, #[props(default = 18)] size: u32) -> Element {
    let inner = match name.as_str() {
        "home" => {
            r#"<path d="m3 9.5 9-7 9 7V20a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><path d="M9.5 22v-9h5v9"/>"#
        }
        "search" => r#"<circle cx="11" cy="11" r="7"/><path d="m20.5 20.5-4.2-4.2"/>"#,
        "live" => {
            r#"<rect x="2" y="7" width="20" height="14" rx="2"/><path d="m16.5 2.5-4.5 4.5-4.5-4.5"/>"#
        }
        "star" => {
            r#"<path d="m12 2.8 2.9 5.9 6.5.9-4.7 4.6 1.1 6.5L12 17.6l-5.8 3.1 1.1-6.5L2.6 9.6l6.5-.9z"/>"#
        }
        "layers" => {
            r#"<path d="m12 2.5 9.5 5-9.5 5-9.5-5zM2.5 12.5l9.5 5 9.5-5M2.5 17l9.5 5 9.5-5"/>"#
        }
        "settings" => {
            r#"<path d="M21 4h-7M10 4H3M21 12h-9M8 12H3M21 20h-5M12 20H3M14 2v4M8 10v4M16 18v4"/>"#
        }
        "grid" => {
            r#"<rect x="3" y="3" width="7.5" height="7.5" rx="1.5"/><rect x="13.5" y="3" width="7.5" height="7.5" rx="1.5"/><rect x="3" y="13.5" width="7.5" height="7.5" rx="1.5"/><rect x="13.5" y="13.5" width="7.5" height="7.5" rx="1.5"/>"#
        }
        "chevron-left" => r#"<path d="m15 18-6-6 6-6"/>"#,
        "chevron-right" => r#"<path d="m9 18 6-6-6-6"/>"#,
        "refresh" => r#"<path d="M20.5 12a8.5 8.5 0 1 1-2.5-6M20.5 3.5V10h-6.5"/>"#,
        "plus" => r#"<path d="M12 5v14M5 12h14"/>"#,
        "check" => r#"<path d="m4.5 12.5 5 5 10-11"/>"#,
        "close" => r#"<path d="M6 6l12 12M18 6 6 18"/>"#,
        "play" => r#"<path d="M6.5 3.5 20 12 6.5 20.5z"/>"#,
        "trash" => r#"<path d="M3.5 6h17M8.5 6V3.5h7V6M5.5 6l1 15h11l1-15M10 10.5v6M14 10.5v6"/>"#,
        "pencil" => r#"<path d="M20.6 7.4a2.6 2.6 0 0 0-3.7-3.7L3.5 17v3.5H7zM16 4.5l3.5 3.5"/>"#,
        "external" => {
            r#"<path d="M14 3.5h6.5V10M20.5 3.5 11 13M18.5 14v4.5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h4.5"/>"#
        }
        "link" => {
            r#"<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a15 15 0 0 1 0 18 15 15 0 0 1 0-18z"/>"#
        }
        "clock" => r#"<circle cx="12" cy="12" r="9"/><path d="M12 6.5V12l3.5 2"/>"#,
        "filter" => r#"<path d="M3 5h18l-7 8v6l-4-2v-4z"/>"#,
        "folder" => {
            r#"<path d="M3 6.5a2 2 0 0 1 2-2h3.5l2 2.5H19a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>"#
        }
        "sparkle" => {
            r#"<path d="M11 3.2 12.8 8.7 18.3 10.5 12.8 12.3 11 17.8 9.2 12.3 3.7 10.5 9.2 8.7zM17.6 14.6 18.5 17.2 21.1 18.1 18.5 19 17.6 21.6 16.7 19 14.1 18.1 16.7 17.2z"/>"#
        }
        _ => r#"<path d="M12 3 2.5 20h19zM12 9.5v4.5M12 17.5v.01"/>"#,
    };
    rsx! {svg{width:"{size}",height:"{size}",view_box:"0 0 24 24",fill:"none",stroke:"currentColor",stroke_width:"1.6",stroke_linecap:"round",stroke_linejoin:"round","aria-hidden":"true",style:"display:block;flex:0 0 auto",dangerous_inner_html:inner}}
}
#[component]
pub(super) fn Toggle(
    checked: bool,
    onchange: Callback<bool>,
    #[props(default)] disabled: bool,
    #[props(default)] title: String,
) -> Element {
    let view = rsx! {button{r#type:"button",class:if checked{"switch on"}else{"switch"},disabled,title,"aria-pressed":checked,onclick:move|_|onchange.call(!checked)}};
    view
}
#[component]
pub(super) fn Empty(
    title: String,
    #[props(default)] desc: String,
    #[props(default)] children: Element,
) -> Element {
    let view = rsx! {div{class:"empty",div{class:"empty-icon",Icon{name:"grid",size:26}}div{class:"empty-title","{title}"}div{class:"empty-desc","{desc}"}div{class:"empty-actions",{children}}}};
    view
}
#[component]
pub(super) fn Loading(#[props(default="正在加载…".into())] text: String) -> Element {
    let view =
        rsx! {div{class:"empty",role:"status",span{class:"spin"}div{class:"empty-desc","{text}"}}};
    view
}
#[component]
pub(super) fn Poster(src: String, name: String, #[props(default)] class: String) -> Element {
    let mut failed = use_signal(|| false);
    rsx! {div{class:"poster {class}",if !src.is_empty()&&!*failed.read(){img{src,alt:"",loading:"lazy",draggable:"false",onerror:move|_|failed.set(true)}}else{span{class:"poster-fallback","{name.trim().chars().next().unwrap_or('?')}"}}}}
}
#[component]
pub(super) fn VodCard(vod: Value) -> Element {
    let ui = use_context::<Ui>();
    let target = Page::Detail(s(&vod, "siteKey"), s(&vod, "vod_id"));
    let name = s(&vod, "vod_name");
    let count = array(&vod["allSiteKeys"]).len();
    let site = s(&vod, "siteName");
    let remarks = s(&vod, "vod_remarks");
    let score = s(&vod, "vod_score");
    rsx! {a{href:"#",class:"vod-card",onclick:move|e|{e.prevent_default();ui.go(target.clone());},Poster{key:"{vod}",src:s(&vod,"vod_pic"),name:name.clone()}div{class:"vod-card-body",div{class:"vod-card-title",title:name.clone(),"{name}"}div{class:"vod-card-meta",span{class:"vod-card-remarks","{remarks}"}if number(&vod["vod_score"],0.0)>0.0{span{class:"vod-card-score","{score}"}}}}div{class:"vod-card-footer",span{class:"tag tag-outline","{site}"}if count>1{span{class:"tag tag-accent","{count} 个源"}}}}}
}
#[component]
pub(super) fn FirstRun() -> Element {
    let ui = use_context::<Ui>();
    rsx! {div{class:"page",div{class:"first-run",img{class:"first-run-logo",src:icon_url(),alt:"",width:"52",height:"52"}h1{class:"first-run-title","欢迎使用 蜡笔小新"}p{class:"first-run-desc","这是一个纯白极简的 Windows 影视聚合客户端。软件本身不提供任何内容，请先添加一个 JSON 配置源（订阅地址或本地配置文件），之后即可聚合搜索、多线路选集播放。"}div{class:"first-run-actions",button{class:"btn btn-primary",onclick:move|_|ui.go(Page::Settings),"前往添加配置源"}button{class:"btn",onclick:move|_|ui.go(Page::Sites),"查看影视源"}}div{class:"first-run-presets",div{class:"first-run-presets-title","或者一键添加内置推荐订阅"}PresetRows{}}div{class:"first-run-steps",for (n,title,desc) in [(1,"添加配置源","填入订阅 URL，或选择一个本地 .json 配置文件"),(2,"启用影视源","在「影视源」页测试连通性，关掉速度慢或失效的站点"),(3,"搜索并播放","顶部搜索框回车即可同时查询全部站点")]{div{class:"first-run-step",span{class:"first-run-num","{n}"}div{b{"{title}"}div{class:"text-3 fs-12","{desc}"}}}}}}}}
}
#[component]
pub(super) fn PresetRows() -> Element {
    let ui = use_context::<Ui>();
    let mut busy = use_signal(String::new);
    rsx! {for preset in crate::presets::list(){{let id=s(&preset,"id");rsx!{div{class:"preset-row",div{class:"grow",div{class:"list-row-title",{s(&preset,"name")}}div{class:"list-row-sub",{s(&preset,"desc")}}}span{class:"tag tag-outline",if preset["remote"]==true{"远程配置"}else{"5 个站点 · 1 个直播源"}}button{class:"btn btn-sm btn-primary",disabled:!busy.read().is_empty(),onclick:move|_|{let id=id.clone();busy.set(id.clone());spawn(async move{let svc=ui.backend();if let Some(source)=ui.report(crate::presets::add(&svc.sources,&id)){ui.refresh();ui.report(svc.sources.sync(&s(&source,"id")).await);ui.refresh();}busy.set(String::new());});},if !busy.read().is_empty(){span{class:"spin"}}else{Icon{name:"plus",size:13}}"添加"}}}}}}
}
#[component]
pub(super) fn Toasts() -> Element {
    let mut ui = use_context::<Ui>();
    rsx! {div{class:"toast-wrap",for toast in ui.toasts.read().iter(){{let id=s(toast,"id");let text=s(toast,"text");let kind=s(toast,"kind");rsx!{div{key:"{id}",class:"toast toast-{kind}",onclick:move|_|ui.toasts.write().retain(|v|s(v,"id")!=id),span{class:"toast-icon",if kind=="success"{"✓"}else if kind=="danger"{"×"}else{"!"}}span{class:"toast-text","{text}"}}}}}}}
}
