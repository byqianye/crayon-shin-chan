use super::*;
use std::collections::HashSet;

fn channels(data: &Value, group: &str, word: &str) -> Vec<Value> {
    let word = word.trim().to_lowercase();
    array(&data["groups"])
        .into_iter()
        .filter(|v| !word.is_empty() || group.is_empty() || s(v, "name") == group)
        .flat_map(|v| array(&v["channels"]))
        .filter(|v| word.is_empty() || s(v, "name").to_lowercase().contains(&word))
        .collect()
}

fn health_of(channel: &Value, table: &Value) -> (usize, usize) {
    let mut ok = 0;
    let mut unknown = 0;
    for item in array(&channel["urls"]) {
        let url = s(&item, "url");
        if !crate::http::is_http_url(&url) || table[&url].is_null() {
            unknown += 1;
        } else if table[&url]["ok"] == true {
            ok += 1;
        }
    }
    (ok, unknown)
}

fn probe_live(
    ui: Ui,
    urls: Vec<String>,
    mut health: Signal<Value>,
    mut probing: Signal<usize>,
    force: bool,
) {
    if urls.is_empty() {
        return;
    }
    probing += 1;
    spawn(async move {
        for part in urls.chunks(30) {
            if let Some(value) = ui.report(ui.backend().live.probe(part, force).await) {
                health.set(value);
            }
        }
        probing -= 1;
    });
}

#[component]
pub(super) fn LivePage() -> Element {
    let ui = use_context::<Ui>();
    let mut sequence = use_signal(|| 0usize);
    let live = use_resource(move || {
        let sequence = *sequence.read();
        let _revision = *ui.revision.read();
        async move {
            ui.backend()
                .live
                .load(sequence > 0)
                .await
                .map_err(|e| format!("{e:#}"))
        }
    });
    let mut group = use_signal(String::new);
    let mut keyword = use_signal(String::new);
    let mut picked = use_signal(|| Value::Null);
    let mut line = use_signal(|| 0usize);
    let mut health = use_signal(|| json!({}));
    let mut warmed = use_signal(HashSet::<String>::new);
    let probing = use_signal(|| 0usize);
    let mut picking = use_signal(|| false);
    let mut pick_generation = use_signal(|| 0u64);
    let mut show_dead = use_signal(|| false);
    use_effect(move || {
        if let Some(Ok(data)) = live.read().as_ref() {
            let groups = array(&data["groups"]);
            if !groups.iter().any(|v| s(v, "name") == *group.peek()) {
                group.set(groups.first().map(|v| s(v, "name")).unwrap_or_default());
            }
            warmed.set(HashSet::new());
            if let Some(table) = ui.report(ui.backend().live.health()) {
                health.set(table);
            }
        }
    });
    use_effect(move || {
        let data = live
            .read()
            .as_ref()
            .and_then(|v| v.as_ref().ok())
            .cloned()
            .unwrap_or(Value::Null);
        let list = channels(&data, &group.read(), &keyword.read());
        let urls: Vec<_> = list
            .iter()
            .take(60)
            .flat_map(|v| array(&v["urls"]))
            .map(|v| s(&v, "url"))
            .filter(|v| crate::http::is_http_url(v) && warmed.peek().get(v).is_none())
            .collect();
        if urls.is_empty() {
            return;
        }
        warmed.write().extend(urls.iter().cloned());
        probe_live(ui, urls, health, probing, false);
    });
    let result = live.read().clone();
    let data = result
        .as_ref()
        .and_then(|v| v.as_ref().ok())
        .cloned()
        .unwrap_or(Value::Null);
    let groups = array(&data["groups"]);
    let list = channels(&data, &group.read(), &keyword.read());
    let hidden = list
        .iter()
        .filter(|v| health_of(v, &health.read()) == (0, 0))
        .count();
    let shown: Vec<_> = list
        .iter()
        .filter(|v| *show_dead.read() || health_of(v, &health.read()) != (0, 0))
        .cloned()
        .collect();
    let failed: Vec<_> = array(&data["sources"])
        .into_iter()
        .filter(|v| !s(v, "error").is_empty())
        .collect();
    let count: usize = groups.iter().map(|v| array(&v["channels"]).len()).sum();
    let loading = result.is_none();
    let recheck = list.clone();
    rsx! {
        div { class: "page",
            div { class: "page-head",
                div { div { class: "page-title", "电视直播" } div { class: "page-sub",
                    if loading { "正在拉取频道列表…" } else { "{groups.len()} 个分组 · {count} 个频道 · 来自 " {array(&data["sources"]).len().to_string()} " 个直播源" }
                }}
                div { class: "page-actions",
                    if *probing.read()>0 { span { class: "page-actions-note", span {class:"spin"} "正在体检线路…" } }
                    button { class:"btn btn-ghost", disabled: loading || *probing.read()>0, onclick:move |_| {
                        if ui.report(ui.backend().live.clear_health()).is_some() {
                            health.set(json!({})); warmed.set(HashSet::new());
                            let urls = recheck.iter().take(60).flat_map(|v|array(&v["urls"])).map(|v|s(&v,"url")).collect();
                            probe_live(ui,urls,health,probing,true);
                        }
                    }, Icon{name:"check",size:14} "体检线路" }
                    button {class:"btn btn-ghost", disabled:loading, onclick:move |_|sequence+=1, Icon{name:"refresh",size:14} "重新拉取"}
                }
            }
            if let Some(Err(error))=result { div{class:"banner banner-danger","{error}"} }
            if !failed.is_empty() { div {class:"banner banner-warn", "{failed.len()} 个直播源没拉到频道：" {failed.iter().map(|v|s(v,"name")).collect::<Vec<_>>().join("、")} br{} span{class:"text-3 fs-12",{s(&failed[0],"error")}}} }
            if loading { Loading{text:"正在读取直播频道…"} }
            if !loading && groups.is_empty() { Empty{title:"直播源里没有解析出频道",desc:"可能是源已失效，点右上角「重新拉取」再试一次。",button{class:"btn btn-primary",onclick:move |_|ui.go(Page::Settings),"管理配置源"}} }
            if !groups.is_empty() {
                div {class:"live-layout",
                    aside{class:"live-groups",div{class:"live-search",Icon{name:"search",size:14}input{class:"input",placeholder:"搜频道名",value:"{keyword}",oninput:move |e|keyword.set(e.value())}}
                        div{class:"live-group-list",for item in groups {{ let name=s(&item,"name"); rsx!{
                            button{class:if keyword.read().is_empty() && *group.read()==name {"live-group active"}else{"live-group"},onclick:move |_|{keyword.set(String::new());group.set(name.clone());},span{class:"grow truncate",{s(&item,"name")}}span{class:"live-group-count",{array(&item["channels"]).len().to_string()}}}
                        }}}}
                    }
                    div{class:"live-main",
                        if !picked.read().is_null(){LiveStage{key:"{picked}-{line}",channel:picked.read().clone(),line_index:line,health,picking:*picking.read()}}
                        div{class:"live-channels-head",span{"共 {list.len()} 个频道" if hidden>0{" · 其中 {hidden} 个没探到可用线路"}}span{class:"grow"}if hidden>0||*show_dead.read(){button{class:"btn btn-ghost btn-sm",onclick:move |_|show_dead.toggle(),if *show_dead.read(){"隐藏这 {hidden} 个不可用"}else{"显示 {hidden} 个不可用"}}}}
                        div{class:"live-channels",for channel in shown {{let channel2=channel.clone();let (ok,unknown)=health_of(&channel,&health.read());let dead=ok==0&&unknown==0;rsx!{
                            button{class:format!("live-channel{}{}",if picked.read()["name"]==channel["name"]{" active"}else{""},if dead{" is-dead"}else{""}),onclick:move |_|{
                                pick_generation+=1;let generation=*pick_generation.peek();let channel=channel2.clone();picked.set(channel.clone());line.set(0);
                                let urls=array(&channel["urls"]);let known=urls.iter().position(|v|health.peek()[s(v,"url")]["ok"]==true);
                                if let Some(index)=known {line.set(index);}else{
                                    let targets:Vec<_>=urls.iter().map(|v|s(v,"url")).filter(|v|crate::http::is_http_url(v)).collect();
                                    if !targets.is_empty(){picking.set(true);spawn(async move{let result=ui.backend().live.probe(&targets,false).await;if generation==*pick_generation.peek(){if let Some(table)=ui.report(result){if let Some(index)=urls.iter().position(|v|table[s(v,"url")]["ok"]==true){line.set(index);}else{ui.notify(format!("{}：{} 条线路都没拉起来，可以换一个频道试试",s(&channel,"name"),targets.len()),"warn");}health.set(table);}picking.set(false);}});}
                                }
                                let _=document::eval("setTimeout(()=>document.querySelector('.live-stage-wrap')?.scrollIntoView({behavior:'smooth',block:'nearest'}),0);");
                            },span{class:"live-channel-name truncate",{s(&channel,"name")}}if dead{span{class:"live-channel-lines live-channel-dead","不可用"}}else if array(&channel["urls"]).len()>1{span{class:"live-channel-lines",if ok>0{"{ok}/"}{array(&channel["urls"]).len().to_string()}" 线"}}}
                        }}}if list.is_empty(){div{class:"field-hint","没有匹配的频道。"}}}
                    }
                }
            }
        }
    }
}

#[component]
fn LiveStage(
    channel: Value,
    line_index: Signal<usize>,
    health: Signal<Value>,
    picking: bool,
) -> Element {
    let ui = use_context::<Ui>();
    let mut retry = use_signal(|| 0usize);
    let mut session = use_signal(String::new);
    let index = *line_index.read();
    let lines = array(&channel["urls"]);
    let raw = lines.get(index).map(|v| s(v, "url")).unwrap_or_default();
    let address = raw.clone();
    let resolved = use_resource(move || {
        let address = address.clone();
        let _ = *retry.read();
        async move {
            if address.starts_with("huya:") || address.starts_with("douyin:") {
                ui.backend()
                    .live
                    .resolve_room(&address)
                    .await
                    .map(|v| s(&v, "url"))
                    .map_err(|e| format!("{e:#}"))
            } else {
                Ok(address)
            }
        }
    });
    let total = lines.len();
    use_effect(move || {
        let event = ui.player_event.read();
        if session.read().is_empty() || s(&event, "session") != *session.read() {
            return;
        }
        let failed = s(&event, "event") == "error"
            || (s(&event, "event") == "hlsError"
                && event["fatal"] == true
                && event["recovered"] != true);
        if failed && index + 1 < total {
            line_index.set(index + 1);
            ui.notify(
                format!("这条线路拉不起来，已自动换到第 {} / {total} 条", index + 2),
                "warn",
            );
        }
    });
    let result = resolved.read().clone();
    let url = result
        .as_ref()
        .and_then(|v| v.as_ref().ok())
        .cloned()
        .unwrap_or_default();
    let external = url.clone();
    let title = s(&channel, "name");
    let copied = url.clone();
    rsx! {div{class:"live-stage-wrap",
        div{class:"live-stage-head",span{class:"live-stage-name",{s(&channel,"name")}}span{class:"tag tag-outline",{s(&channel,"group")}}if lines.len()>1{span{class:"source-chips",for (i,item) in lines.iter().enumerate(){button{class:if index==i{"chip chip-source active"}else{"chip chip-source"},title:s(item,"url"),onclick:move |_|line_index.set(i),if index==i{Icon{name:"check",size:12}}if !health.read()[s(item,"url")].is_null(){span{class:if health.read()[s(item,"url")]["ok"]==true{"dot ok"}else{"dot bad"}}}{s(item,"source")}}}}}span{class:"grow"}
            button{class:"btn btn-ghost btn-sm",disabled:url.is_empty(),onclick:move |_|{let address=external.clone();let title=title.clone();spawn(async move{ui.report(crate::player::open(&ui.backend().store,&address,&title).await);});},Icon{name:"external",size:13}"外部播放器"}
            button{class:"btn btn-ghost btn-sm",disabled:url.is_empty(),onclick:move |_|{let value=copied.clone();spawn(async move{copy(ui,&value).await;});},Icon{name:"link",size:13}"复制地址"}
        }
        if let Some(Err(error))=result{div{class:"live-stage live-stage-msg",Icon{name:"alert",size:22}div{class:"live-stage-msg-title","换不到播放地址"}div{class:"live-stage-msg-desc","{error}"}button{class:"btn btn-ghost btn-sm",onclick:move |_|retry+=1,"重试"}}}else if url.is_empty()||picking{div{class:"live-stage live-stage-msg",span{class:"spin"}div{class:"live-stage-msg-desc",if picking{"正在挑一条能用的线路…"}else{"正在向平台换播放地址…"}}}}else{runtime::PlayerSurface{key:"{url}",address:url.clone(),live:true,class:"live-stage",onsession:move |id|session.set(id)}}
        div{class:"player-url mono",title:url.clone(),span{class:"player-url-text",if url.is_empty(){"{raw}"}else{"{url}"}}}
    }}
}

#[component]
pub(super) fn Library() -> Element {
    let ui = use_context::<Ui>();
    let mut tab = use_signal(|| "favorites".to_owned());
    use_effect(move || {
        let library = ui.library.read();
        if array(&library["favorites"]).is_empty() && !array(&library["history"]).is_empty() {
            tab.set("history".into());
        }
    });
    let favorites = array(&ui.library.read()["favorites"]);
    let history = array(&ui.library.read()["history"]);
    let is_favorite = *tab.read() == "favorites";
    let count = if is_favorite {
        favorites.len()
    } else {
        history.len()
    };
    rsx! {div{class:"page",div{class:"page-head",div{div{class:"page-title","收藏与历史"}div{class:"page-sub","收藏 {favorites.len()} 部 · 观看记录 {history.len()} 条"}}div{class:"page-actions",div{class:"chips",for (key,label,n) in [("favorites","收藏",favorites.len()),("history","观看历史",history.len())]{button{class:if *tab.read()==key{"chip active"}else{"chip"},onclick:move |_|tab.set(key.into()),"{label}" span{class:"chip-count","{n}"}}}}if count>0{button{class:"btn btn-ghost btn-sm",onclick:move |_|{spawn(async move{if confirm(if is_favorite{"清空全部收藏？"}else{"清空全部观看记录？"}).await{ui.report(ui.backend().store.write(if is_favorite{"favorites"}else{"history"},&json!([])));ui.refresh();}});},Icon{name:"trash",size:13}if is_favorite{"清空收藏"}else{"清空记录"}}}}}
        if is_favorite{if favorites.is_empty(){Empty{title:"还没有收藏",desc:"在详情页点「收藏」，影片就会出现在这里，方便下次直接找到。"}}else{div{class:"poster-grid",for item in favorites{FavoriteCard{key:"{crate::storage::item_key(&item)}",item}}}}}else if history.is_empty(){Empty{title:"还没有观看记录",desc:"开始播放之后，这里会记下看到第几集、看到哪儿了，方便接着看。"}}else{div{class:"library-history",for group in history{HistoryRow{key:"{group}",group}}}}
    }}
}

#[component]
fn FavoriteCard(item: Value) -> Element {
    let ui = use_context::<Ui>();
    let detail = Page::Detail(s(&item, "siteKey"), s(&item, "vodId"));
    let detail2 = detail.clone();
    let key = crate::storage::item_key(&item);
    rsx! {div{class:"vod-card",a{href:"#",onclick:move |e|{e.prevent_default();ui.go(detail.clone());},Poster{src:s(&item,"pic"),name:s(&item,"name")}}div{class:"vod-card-body",a{href:"#",class:"vod-card-title truncate",onclick:move |e|{e.prevent_default();ui.go(detail2.clone());},{s(&item,"name")}}div{class:"vod-card-meta",span{class:"truncate",{s(&item,"remarks")}}span{class:"vod-card-score",{s(&item,"score")}}}div{class:"vod-card-tags",span{class:"tag tag-outline truncate",{s(&item,"siteName")}}button{class:"btn btn-sm btn-ghost",title:"取消收藏",onclick:move |_|{ui.report(ui.backend().store.remove_favorite(&key));ui.refresh();},Icon{name:"trash",size:13}}}}}}
}

#[component]
fn HistoryRow(group: Value) -> Element {
    let ui = use_context::<Ui>();
    let mut pick = use_signal(String::new);
    let items = array(&group["items"]);
    let item = items
        .iter()
        .find(|v| crate::storage::item_key(v) == *pick.read())
        .or(items.first())
        .cloned()
        .unwrap_or(Value::Null);
    let play = play_history(&item);
    let play2 = play.clone();
    let detail = Page::Detail(s(&item, "siteKey"), s(&item, "vodId"));
    let pct = percent(&item);
    let delete = group.clone();
    rsx! {div{class:"history-row",a{href:"#",class:"history-poster",onclick:move |e|{e.prevent_default();ui.go(play.clone());},Poster{src:s(&item,"pic"),name:s(&group,"name")}}div{class:"history-main",div{class:"history-title",a{href:"#",onclick:move |e|{e.prevent_default();ui.go(detail.clone());},{s(&group,"name")}}}div{class:"history-sub",span{if s(&item,"epName").is_empty(){"第 "{(number(&item["epIndex"],0.0) as usize+1).to_string()}" 集"}else{"看到 "{s(&item,"epName")}}}span{class:"text-3","·"}span{class:"text-3",{s(&item,"siteName")}}span{class:"text-3","·"}span{class:"text-3",{relative(&item["updatedAt"])}}}if let Some(pct)=pct{div{class:"history-progress",title:"已看 {pct}%",span{class:"history-progress-track",span{class:"history-progress-fill",style:"width:{pct}%"}}span{class:"history-progress-text","看到 "{clock(number(&item["position"],0.0))}" / "{clock(number(&item["duration"],0.0))}}}}if items.len()>1{div{class:"history-sources",span{class:"text-3 fs-12","{items.len()} 个源"}for alt in items{{let key=crate::storage::item_key(&alt);rsx!{button{class:if key==crate::storage::item_key(&item){"chip active"}else{"chip"},onclick:move |_|pick.set(key.clone()),{s(&alt,"siteName")}}}}}}}div{class:"history-actions",button{class:"btn btn-sm btn-primary",onclick:move |_|ui.go(play2.clone()),Icon{name:"play",size:13}"继续观看"}button{class:"btn btn-sm btn-ghost",title:"从历史里删除",onclick:move |_|{let group=delete.clone();spawn(async move{if array(&group["items"]).len()==1||confirm(&format!("删除「{}」在 {} 个源上的观看记录？",s(&group,"name"),array(&group["items"]).len())).await{ui.report(ui.backend().store.remove_history(&s(&group,"key")));ui.refresh();}});},Icon{name:"trash",size:13}}}}}}
}
