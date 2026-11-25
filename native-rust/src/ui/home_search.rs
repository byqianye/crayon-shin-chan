use super::*;
use crate::dedupe;
use std::collections::{HashMap, HashSet};
use tokio_util::sync::CancellationToken;

#[component]
pub(super) fn Home() -> Element {
    let ui = use_context::<Ui>();
    let mut force = use_signal(|| 0u64);
    let mut sections = use_signal(Vec::<Value>::new);
    let mut loading = use_signal(|| true);
    let home = use_resource(move || {
        let _revision = *ui.revision.read();
        let force = *force.read();
        async move {
            loading.set(true);
            sections.set(Vec::new());
            let backend = ui.backend();
            let (send, mut receive) = tokio::sync::watch::channel(Value::Null);
            let progress = Arc::new(move |snapshot| {
                send.send_replace(snapshot);
            });
            let task = backend
                .catalogue
                .home_with_progress(force > 0, Some(progress));
            tokio::pin!(task);
            loop {
                tokio::select! {
                    result = &mut task => {
                        loading.set(false);
                        return result.map(|value| sections.set(value)).map_err(|e| format!("{e:#}"));
                    }
                    Ok(()) = receive.changed() => {
                        sections.set(array(&receive.borrow_and_update()));
                    }
                }
            }
        }
    });
    let content = sections.read().clone();
    let sites = ui.sites.read().clone();
    let sources = ui.sources.read().clone();
    let history = array(&ui.library.read()["history"]);
    let usable = sites
        .iter()
        .filter(|v| v["enabled"] == true && s(v, "unsupported").is_empty())
        .count();
    let searchable = sites
        .iter()
        .filter(|v| {
            v["enabled"] == true && v["searchable"] == true && s(v, "unsupported").is_empty()
        })
        .count();
    let tested: Vec<_> = sites
        .iter()
        .filter_map(|v| v["health"].as_f64())
        .filter(|v| *v >= 0.0)
        .collect();
    let avg = if tested.is_empty() {
        "—".into()
    } else {
        format!("{:.0}", tested.iter().sum::<f64>() / tested.len() as f64)
    };
    rsx! {
        div {class:"page",
            div {class:"hero",h1 {class:"hero-title","蜡笔小新"}p {class:"hero-sub","在顶部搜索框查找影片，或从下方继续观看。内置订阅会在启动后自动更新。"}
                div {class:"hero-actions",
                    button {class:"btn btn-primary",onclick:move |_|ui.go(Page::Settings),"管理配置源"}
                    button {class:"btn",onclick:move |_|ui.go(Page::Sites),"管理影视源（{usable}）"}
                    button {class:"btn",disabled:*ui.busy.read(),onclick:move |_|ui.sync(None),Icon {name:"refresh",size:15}"同步全部配置"}
                    button {class:"btn",disabled:*loading.read(),onclick:move |_|force+=1,Icon {name:"refresh",size:15}"刷新内容位"}
                }
            }
            div {class:"stat-grid",
                for (label,value,sub) in [("配置源",sources.len().to_string(),format!("个 / 启用 {}",sources.iter().filter(|v|v["enabled"]==true).count())),("影视源总数",sites.len().to_string(),"个".into()),("可用影视源",usable.to_string(),format!("其中可搜索 {searchable}")),("平均响应",avg,"ms".into())] {
                    div {class:"stat",div {class:"stat-label","{label}"}div {class:"stat-value","{value}" small {"{sub}"}}}
                }
            }
            if !history.is_empty() {
                div {class:"section",div {class:"section-head",div {class:"section-title","继续观看"}a {href:"#",onclick:move |e|{e.prevent_default();ui.go(Page::Library);},"全部记录 →"}}
                    div {class:"continue-row",for group in history.iter().take(6) {ContinueCard {key:"{group}",group:group.clone()}}}
                }
            }
            for section in content {{let key = s(&section, "key"); rsx! {HomeSection {key:"{key}",section}}}}
            if *loading.read() {Loading {text:"内容会陆续显示，其余站点正在加载…"}}
            else if let Some(Err(error))=home.read().as_ref() {div {class:"banner banner-danger","{error}"}}
            div {class:"section",div {class:"section-head",div {class:"section-title","配置源状态"}a {href:"#",onclick:move |e|{e.prevent_default();ui.go(Page::Settings);},"管理 →"}}
                div {class:"panel",div {class:"list",for source in sources {SourceStatus {key:"{source}",source}}}}
            }
            div {class:"section",div {class:"section-head",div {class:"section-title","影视源概览（按类型）"}}
                div {class:"stat-grid",for kind in [3,1,4,0,2] {
                    {let group:Vec<_>=sites.iter().filter(|v|v["type"]==kind).collect();let active=group.iter().filter(|v|v["enabled"]==true).count();rsx! {
                        if !group.is_empty() {div {class:"stat",div {class:"stat-label",{type_label(group[0])}}div {class:"stat-value","{group.len()}" small {"启用 {active}"}}}}
                    }}
                }}
            }
            Milestones {}
        }
    }
}
#[component]
fn Milestones() -> Element {
    rsx! {div {class:"section",div {class:"section-head",div {class:"section-title","开发里程碑"}div {class:"section-more","详见项目根目录「功能框架.txt」"}}
        div {class:"milestones",for (code,title,desc) in [("M1","项目骨架 + 纯白 UI + 配置源管理","设计变量体系、配置源增删同步、影视源列表与连通性测试。"),("M2","聚合搜索 + 详情选集 + 播放器","多站点并发搜索与同名合并、线路与剧集解析、ArtPlayer + hls.js 播放、自动挑可播放线路、一键转外部播放器。"),("M3","直播 + 收藏 / 历史","m3u 与 txt 分组直播源解析、频道分组与多线路、收藏夹、观看历史与首页继续观看。"),("M4","解析接口 + 首页内容位 + 外部播放器","自动提取播放地址、解析接口排行、首页内容位、检测本机播放器。"),("M5","打包发布","安装包与免安装版本。") ] {
            div {class:"milestone done",div {class:"milestone-head",span {class:"milestone-code","{code}"}span {class:"tag tag-success","已完成"}}div {class:"milestone-title","{title}"}div {class:"milestone-desc","{desc}"}}
        }}
    }}
}
#[component]
fn HomeSection(section: Value) -> Element {
    let ui = use_context::<Ui>();
    let items = array(&section["items"]);
    rsx! {div {class:"section",div {class:"section-head",div {class:"section-title",{s(&section,"title")}if section["origin"]=="default"{span{class:"tag tag-outline",style:"margin-left:8px","自动"}}}div {class:"section-more",if section["loading"]==true {span{class:"text-3",role:"status","正在加载…"}}else if !s(&section,"error").is_empty(){span{class:"text-3",{s(&section,"error")}}}else{span{class:"text-3","{items.len()} 条"}a{href:"#",style:"margin-left:8px",onclick:move|e|{e.prevent_default();ui.go(Page::Browse);},"去分类浏览 →"}}}}div {class:"home-row",for vod in items {VodCard{key:"{vod_key(&vod)}",vod}}}}}
}
#[component]
fn ContinueCard(group: Value) -> Element {
    let ui = use_context::<Ui>();
    let item = group["items"][0].clone();
    let target = play_history(&item);
    rsx! {a {href:"#",class:"continue-card",onclick:move|e|{e.prevent_default();ui.go(target.clone());},div {class:"continue-poster",Poster {src:s(&item,"pic"),name:s(&group,"name")}span {class:"continue-play",Icon {name:"play",size:16}}}div {class:"continue-body",div {class:"continue-title truncate",{s(&group,"name")}}div {class:"continue-sub truncate",{s(&item,"epName")}" · "{relative(&item["updatedAt"])}}if let Some(pct)=percent(&item){div {class:"continue-bar",span {style:"width:{pct}%"}}}}}}
}
#[component]
fn SourceStatus(source: Value) -> Element {
    let mut ui = use_context::<Ui>();
    rsx! {div {class:"list-row",span {class:if !s(&source,"error").is_empty(){"dot bad"}else if number(&source["lastSync"],0.0)>0.0{"dot ok"}else{"dot"}}div {class:"list-row-main",div {class:"list-row-title",{s(&source,"name")}}div {class:"list-row-sub",{s(&source,"url")}}}if !s(&source,"error").is_empty(){span {class:"tag tag-danger",title:s(&source,"error"),"同步失败"}}span {class:"tag tag-outline",{s(&source,"siteCount")}" 站"}span {class:"tag tag-outline",{s(&source,"liveCount")}" 直播"}span {class:"tag tag-outline",{s(&source,"parseCount")}" 解析"}span {class:"fs-12 text-3 nowrap",{relative(&source["lastSync"])}}button {class:"btn btn-sm",onclick:move|_|{ui.source_filter.set(s(&source,"name"));ui.go(Page::Sites);},"查看影视源"}}}
}

#[derive(Clone, Copy, PartialEq)]
pub(super) struct BrowseState {
    bucket: Signal<String>,
    sub: Signal<String>,
    results: Signal<Vec<Value>>,
    busy: Signal<bool>,
    page: Signal<usize>,
    pages: Signal<usize>,
    error: Signal<String>,
    generation: Signal<u64>,
    cancel: Signal<CancellationToken>,
    order: Signal<HashMap<String, usize>>,
    revision: Signal<u64>,
}
pub(super) fn use_browse() -> BrowseState {
    BrowseState {
        bucket: use_signal(String::new),
        sub: use_signal(String::new),
        results: use_signal(Vec::new),
        busy: use_signal(|| false),
        page: use_signal(|| 0),
        pages: use_signal(|| 0),
        error: use_signal(String::new),
        generation: use_signal(|| 0),
        cancel: use_signal(CancellationToken::new),
        order: use_signal(HashMap::new),
        revision: use_signal(|| 0),
    }
}
impl BrowseState {
    fn load(mut self, ui: Ui, bucket: String, sub: String, page: usize) {
        if page > 1 && *self.busy.peek() {
            return;
        }
        let generation = if page == 1 {
            self.generation += 1;
            self.bucket.set(bucket.clone());
            self.sub.set(sub.clone());
            self.results.set(vec![]);
            self.order.set(HashMap::new());
            *self.generation.peek()
        } else {
            *self.generation.peek()
        };
        self.cancel.peek().cancel();
        let cancel = CancellationToken::new();
        self.cancel.set(cancel.clone());
        self.busy.set(true);
        self.error.set(String::new());
        let base = if page == 1 {
            Vec::new()
        } else {
            self.results.peek().clone()
        };
        let revision = ui.backend().sources.revision();
        spawn(async move {
            let backend = ui.backend();
            let (send, mut receive) = tokio::sync::watch::channel(Value::Null);
            let progress = Arc::new(move |snapshot| {
                send.send_replace(snapshot);
            });
            let task = backend
                .catalogue
                .browse_with_progress(&bucket, &sub, page, Some(progress));
            tokio::pin!(task);
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    result = &mut task => {
                        if *self.generation.peek() != generation || backend.sources.revision() != revision { return; }
                        match result {
                            Ok(result) => self.apply_page(ui, &base, &result, page),
                            Err(error) => {
                                self.error.set(format!("{error:#}"));
                                ui.notify(format!("加载分类失败：{error:#}"), "danger");
                            }
                        }
                        self.busy.set(false);
                        return;
                    }
                    Ok(()) = receive.changed() => {
                        if *self.generation.peek() != generation || backend.sources.revision() != revision { return; }
                        let result = receive.borrow_and_update().clone();
                        self.apply_page(ui, &base, &result, page);
                    }
                }
            }
        });
    }

    fn apply_page(mut self, mut ui: Ui, base: &[Value], result: &Value, page: usize) {
        let results = crate::category::merge_browse_pages(base, &array(&result["results"]));
        let merged = dedupe::merge_results(
            &results,
            "",
            flag(&ui.settings.peek()["filterCommentary"], true),
            Some(&self.order.peek()),
        );
        for vod in array(&merged["merged"]) {
            let mut order = self.order.write();
            let next = order.len();
            order.entry(dedupe::key(&vod)).or_insert(next);
        }
        self.page.set(page);
        self.pages
            .set(number(&result["pageCount"], page as f64) as usize);
        ui.browse_results.set(results.clone());
        self.results.set(results);
    }
}

#[component]
pub(super) fn Search(keyword: String) -> Element {
    let mut ui = use_context::<Ui>();
    let browsing = keyword.is_empty();
    let wd = keyword.clone();
    let mut running = use_signal(|| !browsing);
    let mut done = use_signal(|| 0usize);
    let mut total = use_signal(|| 0usize);
    let mut filter = use_signal(String::new);
    let mut browse = ui.browse;
    let mut category_loading = use_signal(|| browsing);
    use_drop(move || {
        browse.cancel.peek().cancel();
        browse.generation += 1;
        browse.busy.set(false);
    });
    use_resource(move || {
        let revision = *ui.revision.read();
        async move {
            if browsing {
                if *browse.revision.peek() != revision {
                    browse.cancel.peek().cancel();
                    browse.generation += 1;
                    browse.bucket.set(String::new());
                    browse.sub.set(String::new());
                    browse.results.set(Vec::new());
                    browse.order.set(HashMap::new());
                    browse.busy.set(false);
                    browse.page.set(0);
                    browse.pages.set(0);
                    ui.categories.set(Vec::new());
                    browse.revision.set(revision);
                }
                category_loading.set(true);
                let backend = ui.backend();
                let (send, mut receive) = tokio::sync::watch::channel(Value::Null);
                let progress = Arc::new(move |snapshot| {
                    send.send_replace(snapshot);
                });
                let task = backend.catalogue.categories_with_progress(Some(progress));
                tokio::pin!(task);
                loop {
                    tokio::select! {
                        result = &mut task => {
                            match result {
                                Ok(value) => ui.categories.set(value),
                                Err(error) => ui.notify(format!("读取分类失败：{error:#}"), "danger"),
                            }
                            category_loading.set(false);
                            break;
                        }
                        Ok(()) = receive.changed() => { ui.categories.set(array(&receive.borrow_and_update())); }
                    }
                }
            }
        }
    });
    use_future(move || {
        let wd = wd.clone();
        async move {
            if browsing {
                return;
            }
            let revision = ui.backend().sources.revision();
            if ui.search_key.peek().as_ref() == Some(&(wd.clone(), revision)) {
                done.set(ui.results.peek().len());
                total.set(ui.results.peek().len());
                running.set(false);
                return;
            }
            ui.results.set(vec![]);
            let catalogue = ui.backend().catalogue.clone();
            let (send, mut receive) = tokio::sync::mpsc::unbounded_channel();
            let progress = Arc::new(move |result, n, all| {
                let _ = send.send((result, n, all));
            });
            let task = catalogue.search(&wd, json!({}), Some(progress));
            tokio::pin!(task);
            loop {
                tokio::select! {result=&mut task=>{if let Some(result)=ui.report(result){ui.results.set(array(&result["results"]));total.set(number(&result["total"],0.0) as usize);done.set(ui.results.peek().len());}break;},Some((result,n,all))=receive.recv()=>{let mut results=ui.results.write();if let Some(old)=results.iter_mut().find(|v|v["siteKey"]==result["siteKey"]){*old=result;}else{results.push(result);}done.set(n);total.set(all);}}
            }
            running.set(false);
            ui.search_key.set(Some((wd.clone(), revision)));
        }
    });
    let source = if browsing {
        browse.results.read().clone()
    } else {
        ui.results.read().clone()
    };
    let filtered = flag(&ui.settings.read()["filterCommentary"], true);
    let order = browse.order.read().clone();
    let merged = dedupe::merge_results(
        &source,
        &keyword,
        filtered,
        if browsing { Some(&order) } else { None },
    );
    let list = array(&merged["merged"]);
    let busy = if browsing {
        *browse.busy.read()
    } else {
        *running.read()
    };
    let usable =
        ui.sites.read().iter().any(|v| {
            v["enabled"] == true && v["searchable"] == true && s(v, "unsupported").is_empty()
        });
    let category_list = ui.categories.read().clone();
    let selected_bucket = browse.bucket.read().clone();
    let selected_sub = browse.sub.read().clone();
    let subs = category_list
        .iter()
        .find(|v| s(v, "name") == selected_bucket)
        .map(|v| array(&v["subs"]))
        .unwrap_or_default();
    rsx! {
        div {class:"page",
            div {class:"page-head",div {div {class:"page-title",if browsing{"分类浏览"}else{"聚合搜索"}}div {class:"page-sub",if browsing{if selected_bucket.is_empty(){"按类型浏览各个影视源的内容，选一个分类开始"}else{"「{selected_bucket} · {selected_sub}」· 累计 {list.len()} 部"}}else{"「{keyword}」· {done}/{total} 个站点有响应 · 合并后 {list.len()} 部"}}}
                div {class:"page-actions",if !browsing&&busy{span {class:"text-3 fs-13 hstack",span {class:"spin"}"正在搜索 {done}/{total}"}button {class:"btn btn-sm",onclick:move |_|ui.backend().catalogue.cancel_search(),"停止搜索"}}button {class:"btn btn-sm",onclick:move |_|ui.go(Page::Browse),"清空结果"}button {class:if filtered{"chip active"}else{"chip"},onclick:move |_|ui.patch(json!({"filterCommentary":!filtered})),Icon {name:"filter",size:13}"过滤解说" span {class:"chip-count",{s(&merged,"hidden")}}}}
            }
            if !usable {Empty {title:"没有可用的影视源",desc:"先到「影视源」页启用几个苹果CMS、网页规则或 Spider 类型的站点。",button {class:"btn btn-primary",onclick:move |_|ui.go(Page::Sites),"前往影视源"}}}
            else {
                if browsing {
                    if *category_loading.read() {div {class:"text-3 fs-13",role:"status","分类会陆续显示，其余站点正在加载…"}}
                    div {class:"cat-grid",for category in category_list {
                        {let name=s(&category,"name");let subs=array(&category["subs"]);let sub=if subs.len()>1{s(&subs[0],"name")}else{String::new()};let count:HashSet<_>=subs.iter().flat_map(|v|array(&v["sites"])).map(|v|s(&v,"siteKey")).collect();rsx! {
                            button {class:if selected_bucket==name{"cat-card active"}else{"cat-card"},onclick:move |_|browse.load(ui,name.clone(),sub.clone(),1),span {class:"cat-name",{s(&category,"name")}}span {class:"cat-meta","{count.len()} 个源 · {subs.len()} 个子分类"}}
                        }}
                    }}
                    if subs.len()>1 {
                        div {class:"chips sub-bar",button {class:if selected_sub.is_empty(){"chip active"}else{"chip"},onclick:move |_|{let bucket=browse.bucket.peek().clone();browse.load(ui,bucket,String::new(),1);},"全部"}
                            for sub in subs {
                                {let name=s(&sub,"name");let count=array(&sub["sites"]).len();rsx! {button {class:if selected_sub==name{"chip active"}else{"chip"},onclick:move |_|{let bucket=browse.bucket.peek().clone();browse.load(ui,bucket,name.clone(),1);},{s(&sub,"name")}span {class:"chip-count","{count}"}}}}
                            }
                        }
                    }
                }else {
                    div {class:"chips",style:"margin-bottom:14px",button {class:if filter.read().is_empty(){"chip active"}else{"chip"},onclick:move |_|filter.set(String::new()),"全部站点" span {class:"chip-count","{list.len()}"}}
                        for result in &source {
                            {let key=s(result,"siteKey");let count=array(&result["list"]).len();rsx! {button {class:if *filter.read()==key{"chip active"}else{"chip"},title:s(result,"message"),onclick:move |_|filter.set(key.clone()),span {class:if result["ok"]==true{"dot ok"}else{"dot bad"}}{s(result,"siteName")}span {class:"chip-count","{count}"}}}}
                        }
                    }
                }
                if list.is_empty()&&(busy||!source.is_empty()) {Empty {title:if busy{"正在加载…"}else{"这里还没有内容"},desc:if busy{"还有站点在返回数据，结果会陆续出现。"}else{"换个分类或关键词试试，或者检查站点是否可用。"}}}
                else {div {class:"poster-grid",for vod in list.into_iter().filter(|v|filter.read().is_empty()||array(&v["allSiteKeys"]).iter().any(|k|crate::model::text(k)==*filter.read())) {VodCard {key:"{vod_key(&vod)}",vod}}}}
                if browsing&&!selected_bucket.is_empty() {BrowseMore {state:browse}}
                if !busy {details {class:"failed-sites",summary {"查看站点响应详情"}div {class:"list",for result in source.iter().filter(|v|v["ok"]!=true) {div {class:"list-row",div {class:"list-row-main",div {class:"list-row-title",{s(result,"siteName")}}div {class:"list-row-sub",{s(result,"message")}}}}}}}}
            }
        }
    }
}

#[component]
fn BrowseMore(state: BrowseState) -> Element {
    let ui = use_context::<Ui>();
    let listener = use_hook(|| uuid::Uuid::new_v4().to_string());
    let cleanup = listener.clone();
    use_drop(move || {
        let _ = document::eval(&format!(
            "window.__crayonScrollCleanup?.get({0})?.();window.__crayonScrollCleanup?.delete({0});",
            json!(cleanup)
        ));
    });
    use_future(move || {
        let listener = listener.clone();
        async move {
            let mut events = document::eval(&format!(
                "const root=document.querySelector('.app-content');const onscroll=()=>{{if(root&&root.scrollHeight-root.scrollTop-root.clientHeight<700)dioxus.send(true);}};root?.addEventListener('scroll',onscroll);window.__crayonScrollCleanup??=new Map();window.__crayonScrollCleanup.set({},()=>root?.removeEventListener('scroll',onscroll));await new Promise(()=>{{}});",
                json!(listener)
            ));
            while events.recv::<bool>().await.is_ok() {
                if !*state.busy.peek()
                    && state.error.peek().is_empty()
                    && *state.page.peek() < *state.pages.peek()
                {
                    state.load(
                        ui,
                        state.bucket.peek().clone(),
                        state.sub.peek().clone(),
                        *state.page.peek() + 1,
                    );
                }
            }
        }
    });
    rsx! {div {class:"load-more",
        if !state.error.read().is_empty() {span {class:"text-3 fs-13","下一页加载失败：{state.error}"}}
        if *state.busy.read() {Loading {text:"正在加载分类内容…"}}
        else if *state.page.read()<*state.pages.read() {button {class:"btn",onclick:move |_|state.load(ui,state.bucket.peek().clone(),state.sub.peek().clone(),*state.page.peek()+1),"加载更多 · 第 {state.page} / {state.pages} 页"}}
        else {span {class:"text-3 fs-13","已经到底了 · 共 {state.page} 页"}}
    }}
}
