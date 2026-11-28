use crate::{
    model::{array, flag, now, number, s},
    storage::Store,
};
use anyhow::Result;
use base64::Engine;
use dioxus::{
    desktop::{Config, LogicalSize, WindowBuilder},
    prelude::*,
};
use serde_json::{Value, json};
use std::sync::Arc;
mod detail_player;
mod home_search;
mod live_library;
mod media_controls;
mod runtime;
mod sites_settings;
mod widgets;
use detail_player::*;
use home_search::*;
use live_library::*;
use media_controls::*;
use runtime::Services;
use sites_settings::*;
use widgets::*;
const TOKENS: &str = include_str!("../../src/renderer/src/styles/tokens.css");
const GLOBAL: &str = include_str!("../../src/renderer/src/styles/global.css");
const PAGES: &str = include_str!("../../src/renderer/src/styles/pages.css");
const ARTPLAYER: &str = include_str!("../../node_modules/artplayer/dist/artplayer.js");
const HLS: &str = include_str!("../../node_modules/hls.js/dist/hls.min.js");
const MPEGTS: &str = include_str!("../../node_modules/mpegts.js/dist/mpegts.js");
const SHAKA: &str = include_str!("../../node_modules/shaka-player/dist/shaka-player.compiled.js");
const PLAYER_ADAPTERS: &str = include_str!("ui/player_bridge.js");
const ICON: &[u8] = include_bytes!("../../build/icon.png");

#[derive(Clone, PartialEq, Debug)]
enum Page {
    Home,
    Browse,
    Search(String),
    Live,
    Library,
    Sites,
    Settings,
    Detail(String, String),
    Player(String, String, usize, usize),
    PlayerByEpisodeName(String, String, usize, String),
}
#[derive(Clone, Copy)]
struct Ui {
    services: Signal<Arc<Services>>,
    settings: Signal<Value>,
    sources: Signal<Vec<Value>>,
    sites: Signal<Vec<Value>>,
    library: Signal<Value>,
    page: Signal<Page>,
    origin: Signal<Page>,
    keyword: Signal<String>,
    search_key: Signal<Option<(String, u64)>>,
    source_filter: Signal<String>,
    browse: BrowseState,
    results: Signal<Vec<Value>>,
    browse_results: Signal<Vec<Value>>,
    categories: Signal<Vec<Value>>,
    busy: Signal<bool>,
    toasts: Signal<Vec<Value>>,
    ready: Signal<bool>,
    revision: Signal<u64>,
    player_event: Signal<Value>,
    external_player: Signal<Value>,
}
impl Ui {
    fn backend(self) -> Arc<Services> {
        self.services.read().clone()
    }
    fn refresh(self) {
        let svc = self.backend();
        let result = (|| -> Result<()> {
            replace(self.settings, svc.store.settings()?);
            replace(self.sources, svc.sources.list()?);
            replace(self.sites, svc.sources.sites()?);
            replace(self.library, svc.store.library()?);
            replace(self.revision, svc.sources.revision());
            Ok(())
        })();
        if let Err(error) = result {
            self.notify(format!("{error:#}"), "danger");
        }
    }
    fn refresh_library(self) {
        if let Some(value) = self.report(self.backend().store.library()) {
            replace(self.library, value);
        }
    }
    fn notify(mut self, text: impl Into<String>, kind: &str) {
        let id = uuid::Uuid::new_v4().to_string();
        self.toasts
            .write()
            .push(json!({"id":id,"text":text.into(),"kind":kind}));
        let delay = if kind == "danger" { 6000 } else { 3000 };
        dioxus::core::spawn_forever(async move {
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            self.toasts.write().retain(|v| s(v, "id") != id);
        });
    }
    fn report<T>(self, result: Result<T>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(error) => {
                self.notify(format!("{error:#}"), "danger");
                None
            }
        }
    }
    fn go(mut self, page: Page) {
        if matches!(
            page,
            Page::Detail(_, _) | Page::Player(_, _, _, _) | Page::PlayerByEpisodeName(_, _, _, _)
        )
            && !matches!(
                *self.page.peek(),
                Page::Detail(_, _) | Page::Player(_, _, _, _) | Page::PlayerByEpisodeName(_, _, _, _)
            )
        {
            self.origin.set(self.page.peek().clone());
        }
        if let Page::Search(keyword) = &page {
            replace(self.keyword, keyword.clone());
        }
        if !matches!(page, Page::Search(_)) {
            self.backend().catalogue.cancel_search();
        }
        self.page.set(page);
        let _ = document::eval("document.querySelector('.app-content')?.scrollTo({top:0});");
    }
    fn patch(self, patch: Value) {
        if self
            .report(self.backend().store.save_settings(&patch))
            .is_some()
        {
            self.refresh();
        }
    }
    fn sync(mut self, id: Option<String>) {
        if *self.busy.peek() {
            return;
        }
        self.busy.set(true);
        spawn(async move {
            let svc = self.backend();
            let result = if let Some(id) = id {
                svc.sources.sync(&id).await.map(|v| vec![v])
            } else {
                svc.sources.sync_all().await
            };
            if let Some(result) = self.report(result) {
                let failed = result.iter().filter(|v| !s(v, "error").is_empty()).count();
                self.notify(
                    format!(
                        "同步完成：{} 个成功，{failed} 个失败",
                        result.len() - failed
                    ),
                    if failed > 0 { "warn" } else { "success" },
                );
            }
            self.refresh();
            self.busy.set(false);
        });
    }
}
fn replace<T: PartialEq + 'static>(mut signal: Signal<T>, value: T) {
    if *signal.peek() != value {
        signal.set(value);
    }
}
pub fn launch(store: Arc<Store>) -> Result<()> {
    let settings = store.settings()?;
    let width = number(&settings["window"]["width"], 1440.0).max(1100.0);
    let height = number(&settings["window"]["height"], 900.0).max(700.0);
    let image = image::load_from_memory(ICON)?.into_rgba8();
    let icon = dioxus::desktop::tao::window::Icon::from_rgba(
        image.to_vec(),
        image.width(),
        image.height(),
    )?;
    let window = WindowBuilder::new()
        .with_title("蜡笔小新")
        .with_inner_size(LogicalSize::new(width, height))
        .with_min_inner_size(LogicalSize::new(1100.0, 700.0))
        .with_window_icon(Some(icon));
    let config = Config::new()
        .with_window(window)
        .with_data_directory(store.root.join("session"))
        .with_menu(None)
        .with_custom_head(format!(
            "<style>{TOKENS}\n{GLOBAL}\n{PAGES}\n#main{{height:100%}}</style>"
        ));
    let services = Arc::new(Services::new(store)?);
    dioxus::LaunchBuilder::desktop()
        .with_cfg(config)
        .with_context(services)
        .launch(App);
    Ok(())
}
fn icon_url() -> String {
    format!(
        "data:{};base64,{}",
        "image/png",
        base64::engine::general_purpose::STANDARD.encode(ICON)
    )
}

#[allow(non_snake_case)]
fn App() -> Element {
    let svc = use_context::<Arc<Services>>();
    let settings = svc.store.settings().unwrap_or_else(|_| json!({}));
    let mut ui = Ui {
        services: use_signal(|| svc),
        settings: use_signal(|| settings),
        sources: use_signal(Vec::new),
        sites: use_signal(Vec::new),
        library: use_signal(|| json!({})),
        page: use_signal(|| Page::Home),
        origin: use_signal(|| Page::Browse),
        keyword: use_signal(String::new),
        search_key: use_signal(|| None),
        source_filter: use_signal(String::new),
        browse: use_browse(),
        results: use_signal(Vec::new),
        browse_results: use_signal(Vec::new),
        categories: use_signal(Vec::new),
        busy: use_signal(|| false),
        toasts: use_signal(Vec::new),
        ready: use_signal(|| false),
        revision: use_signal(|| 0),
        player_event: use_signal(|| json!({})),
        external_player: use_signal(|| json!({})),
    };
    use_context_provider(|| ui);
    use_future(move || async move {
        ui.refresh();
        ui.ready.set(true);
        let svc = ui.backend();
        ui.report(crate::presets::ensure(&svc.sources).await);
        ui.refresh();
        ui.report(crate::presets::sync_remote(&svc.sources).await);
        ui.refresh();
    });
    use_effect(move || {
        let settings = ui.settings.read().clone();
        let accent = s(&settings, "accent");
        let density = s(&settings, "density");
        let _ = document::eval(&format!(
            "document.documentElement.style.setProperty('--accent',{});document.documentElement.style.setProperty('--accent-hover',{});document.documentElement.dataset.density={};",
            json!(accent),
            json!(shade(&accent)),
            json!(density)
        ));
    });
    runtime::install(ui);
    if !*ui.ready.read() {
        return rsx! {div{class:"boot",img{class:"boot-logo",src:icon_url(),width:"52",height:"52"}div{class:"boot-text","正在加载本地配置…"}}};
    }
    let page = ui.page.read().clone();
    let collapsed = flag(&ui.settings.read()["sidebarCollapsed"], false);
    let enabled = ui
        .sites
        .read()
        .iter()
        .filter(|v| v["enabled"] == true && s(v, "unsupported").is_empty())
        .count();
    rsx! {div{class:"app-shell",aside{class:if collapsed{"sidebar collapsed"}else{"sidebar"},div{class:"sidebar-brand",img{class:"sidebar-logo",src:icon_url(),alt:"",width:"28",height:"28"}div{class:"sidebar-name","蜡笔小新"}}
        nav{class:"sidebar-nav",for (target,label,icon,count) in [(Page::Home,"首页","home",0),(Page::Browse,"分类浏览","grid",0),(Page::Live,"电视直播","live",0),(Page::Library,"收藏与历史","star",0),(Page::Sites,"影视源","layers",enabled),(Page::Settings,"设置","settings",ui.sources.read().len())]{
            a{href:"#",class:if target==page||(target==Page::Browse&&matches!(page,Page::Search(_))){"nav-item active"}else{"nav-item"},title:label,onclick:move|e|{e.prevent_default();ui.go(target.clone());},span{class:"nav-icon",Icon{name:icon}}span{class:"nav-label","{label}"}if count>0{span{class:"nav-count","{count}"}}}
        }}div{class:"sidebar-foot",button{r#type:"button",class:"nav-item",title:if collapsed{"展开侧栏"}else{"收起侧栏"},onclick:move|_|ui.patch(json!({"sidebarCollapsed":!collapsed})),span{class:"nav-icon",Icon{name:if collapsed{"chevron-right"}else{"chevron-left"}}}span{class:"nav-label","收起侧栏"}}}}
        div{class:"app-main",Topbar{}main{class:"app-content",match page{Page::Home=>rsx!{if ui.sources.read().is_empty(){FirstRun{}}else{Home{}}},Page::Browse=>rsx!{Search{key:"{Page::Browse:?}",keyword:String::new()}},Page::Search(wd)=>rsx!{Search{key:"search-{wd}",keyword:wd}},Page::Live=>rsx!{LivePage{}},Page::Library=>rsx!{Library{}},Page::Sites=>rsx!{Sites{}},Page::Settings=>rsx!{Settings{}},Page::Detail(key,id)=>rsx!{Detail{key:"detail-{key}-{id}",site_key:key,vod_id:id}},Page::Player(key,id,line,ep)=>rsx!{Player{key:"play-{key}-{id}-{line}-{ep}",site_key:key,vod_id:id,line_index:line,ep_index:ep,episode_name:None}},Page::PlayerByEpisodeName(key,id,ep,name)=>rsx!{Player{key:"play-{key}-{id}-source-{ep}",site_key:key,vod_id:id,line_index:0,ep_index:ep,episode_name:Some(name)}}}}}
        Toasts{}
    }}
}
#[component]
fn Topbar() -> Element {
    let ui = use_context::<Ui>();
    let mut keyword = ui.keyword;
    let version = env!("CARGO_PKG_VERSION");
    rsx! {header{class:"topbar",div{class:"search-box",span{class:"search-icon",Icon{name:"search",size:15}}input{value:"{keyword}",placeholder:"搜索影片、剧集、动漫…（回车聚合全部站点）",oninput:move|e|keyword.set(e.value()),onkeydown:move|e|{if e.key()==Key::Enter&&!keyword.read().trim().is_empty(){ui.go(Page::Search(keyword.read().trim().into()));}}}}div{class:"topbar-spacer"}button{r#type:"button",class:"btn btn-ghost btn-sm",disabled:*ui.busy.read(),onclick:move|_|ui.sync(None),if *ui.busy.read(){span{class:"spin"}}else{Icon{name:"refresh",size:14}}"同步配置"}span{class:"tag tag-outline mono",title:format!("用户数据目录：{}",ui.backend().store.root.display()),"v{version}"}}}
}
