use super::*;
use crate::{dedupe, media};
mod ai_summary;
use ai_summary::AiSummary;
use tokio_util::sync::CancellationToken;

fn sources_for(ui: Ui, detail: &Value) -> Vec<Value> {
    let mut out = vec![detail.clone()];
    let mut results = ui.results.read().clone();
    results.extend(ui.browse_results.read().clone());
    let merged = dedupe::merge_results(
        &results,
        "",
        flag(&ui.settings.read()["filterCommentary"], true),
        None,
    );
    for vod in array(&merged["merged"]) {
        let mut group = vec![vod.clone()];
        group.extend(array(&vod["alternatives"]));
        if group
            .iter()
            .any(|v| v["siteKey"] == detail["siteKey"] && v["vod_id"] == detail["vod_id"])
        {
            for item in group {
                if !out
                    .iter()
                    .any(|v| v["siteKey"] == item["siteKey"] && v["vod_id"] == item["vod_id"])
                {
                    out.push(item);
                }
            }
            break;
        }
    }
    out
}
fn default_line(detail: &Value) -> usize {
    array(&detail["lines"])
        .iter()
        .position(|v| {
            array(&v["episodes"])
                .first()
                .is_some_and(|v| media::is_direct(&s(v, "url")))
        })
        .unwrap_or(0)
}

#[component]
pub(super) fn Detail(site_key: String, vod_id: String) -> Element {
    let ui = use_context::<Ui>();
    let key = site_key.clone();
    let id = vod_id.clone();
    let detail = use_resource(move || {
        let key = key.clone();
        let id = id.clone();
        async move {
            ui.backend()
                .catalogue
                .detail(&key, &id)
                .await
                .map_err(|e| format!("{e:#}"))
        }
    });
    let mut line = use_signal(|| 0usize);
    let mut reversed = use_signal(|| false);
    let mut desc_open = use_signal(|| false);
    use_effect(move || {
        if let Some(Ok(detail)) = detail.read().as_ref() {
            line.set(default_line(detail));
        }
    });
    let result = detail.read().clone();
    let preview = ui
        .results
        .read()
        .iter()
        .chain(ui.browse_results.read().iter())
        .flat_map(|v| array(&v["list"]))
        .find(|v| s(v, "siteKey") == site_key && s(v, "vod_id") == vod_id);
    let data = result
        .as_ref()
        .and_then(|v| v.as_ref().ok())
        .cloned()
        .or(preview)
        .unwrap_or_else(|| json!({"siteKey":site_key,"vod_id":vod_id}));
    let loaded = matches!(result, Some(Ok(_)));
    let lines = array(&data["lines"]);
    let episodes = array(&lines.get(*line.read()).unwrap_or(&Value::Null)["episodes"]);
    let count: usize = lines.iter().map(|v| array(&v["episodes"]).len()).sum();
    let favorited = array(&ui.library.read()["favorites"])
        .iter()
        .any(|v| v["siteKey"] == data["siteKey"] && v["vodId"] == data["vod_id"]);
    let alternatives = sources_for(ui, &data);
    let detail_for_play = data.clone();
    let detail_for_favorite = data.clone();
    rsx! {div{class:"page",div{class:"detail-head",button{class:"btn btn-sm",onclick:move|_|ui.go(ui.origin.peek().clone()),Icon{name:"chevron-left",size:14}"返回"}span{class:"text-3 fs-12",{s(&data,"siteName")}}}
        if let Some(Err(error))=result{Empty{title:"获取详情失败",desc:error,button{class:"btn",onclick:move|_|ui.go(Page::Sites),"检查影视源"}}}else{
            div{class:"detail-hero",div{class:"poster-detail",Poster{src:s(&data,"vod_pic"),name:s(&data,"vod_name")}}div{class:"detail-info",h1{class:"detail-title",{s(&data,"vod_name")}}div{class:"detail-meta",if number(&data["vod_score"],0.0)>0.0{span{class:"detail-score",{s(&data,"vod_score")}}}for value in [s(&data,"type_name"),s(&data,"vod_year"),s(&data,"vod_area"),s(&data,"vod_lang"),s(&data,"vod_remarks")].into_iter().filter(|v|!v.is_empty()){span{class:"tag tag-outline","{value}"}}}div{class:"detail-crew",for (key,label) in [("vod_director","导演"),("vod_actor","主演")]{if !s(&data,key).is_empty(){div{span{class:"text-3","{label} "}{s(&data,key)}}}}if !s(&data,"vod_content").is_empty(){div{class:if *desc_open.read(){"detail-desc open"}else{"detail-desc"},{s(&data,"vod_content")}button{class:"detail-desc-toggle",onclick:move|_|desc_open.toggle(),if *desc_open.read(){"收起"}else{"展开"}}}}}
                if !loaded{div{class:"detail-pending",span{class:"spin"}"正在获取线路与剧集…"}}else if !lines.is_empty(){div{class:"detail-actions",button{class:"btn btn-primary",onclick:move|_|ui.go(Page::Player(s(&detail_for_play,"siteKey"),s(&detail_for_play,"vod_id"),*line.peek(),0)),Icon{name:"play",size:15}"立即播放"}button{class:"btn",onclick:move|_|{ui.report(ui.backend().store.toggle_favorite(favorite(&detail_for_favorite)));ui.refresh();},Icon{name:"star",size:15}if favorited{"已收藏"}else{"收藏"}}span{class:"text-3 fs-12","共 {lines.len()} 条线路，{count} 个剧集"}}}else{div{class:"banner banner-warn","这个站点没有返回可播放的剧集地址，换一个来源试试。"}}
            }}
            if alternatives.len()>1{div{class:"section",id:"source-picker",div{class:"section-head",div{class:"section-title","选择片源"}span{class:"text-3 fs-12","同一部影片在 {alternatives.len()} 个站点上都有"}}div{class:"source-list",for source in alternatives{SourceItem{key:"{vod_key(&source)}",source,current_key:site_key.clone(),current_id:vod_id.clone(),playing:false}}}}}
            if loaded&&!lines.is_empty(){div{class:"section",div{class:"section-head",div{class:"section-title","选集"}button{class:"btn btn-sm",onclick:move|_|reversed.toggle(),Icon{name:"filter",size:13}if *reversed.read(){"正序"}else{"倒序"}}}div{class:"chips",for (index,item) in lines.iter().enumerate(){button{class:if index==*line.read(){"chip active"}else{"chip"},onclick:move|_|line.set(index),{s(item,"name")}span{class:"chip-count",{array(&item["episodes"]).len().to_string()}}}}}div{class:"episode-grid",for (index,episode) in episodes.iter().enumerate().collect::<Vec<_>>().into_iter().rev().filter(|_|*reversed.read()).chain(episodes.iter().enumerate().filter(|_|!*reversed.read())){EpisodeButton{key:"{index}",episode:episode.clone(),page:Page::Player(site_key.clone(),vod_id.clone(),*line.read(),index),active:false}}}}}
        }
    }}
}
#[component]
fn SourceItem(source: Value, current_key: String, current_id: String, playing: bool) -> Element {
    let ui = use_context::<Ui>();
    let active = s(&source, "siteKey") == current_key && s(&source, "vod_id") == current_id;
    let target = if playing {
        Page::Player(s(&source, "siteKey"), s(&source, "vod_id"), 0, 0)
    } else {
        Page::Detail(s(&source, "siteKey"), s(&source, "vod_id"))
    };
    rsx! {a{href:"#",class:if active{"source-item active"}else{"source-item"},onclick:move|e|{e.prevent_default();if !active{ui.go(target.clone());}},span{class:if active{"source-icon on"}else{"source-icon"},Icon{name:if active{"check"}else{"play"},size:14}}div{class:"source-main",div{class:"source-name",{s(&source,"siteName")}if active{span{class:"tag tag-accent","正在看"}}}div{class:"source-sub",{s(&source,"vod_remarks")}}}if !active{span{class:"source-go","切换" Icon{name:"chevron-right",size:13}}}}}
}
#[component]
fn EpisodeButton(episode: Value, page: Page, active: bool) -> Element {
    let ui = use_context::<Ui>();
    rsx! {button{class:if active{"episode episode-sm active"}else{"episode"},title:s(&episode,"name"),onclick:move|_|ui.go(page.clone()),{s(&episode,"name")}}}
}
#[component]
pub(super) fn Player(
    site_key: String,
    vod_id: String,
    line_index: usize,
    ep_index: usize,
) -> Element {
    let ui = use_context::<Ui>();
    let detail = use_resource(move || {
        let key = site_key.clone();
        let id = vod_id.clone();
        async move {
            ui.backend()
                .catalogue
                .detail(&key, &id)
                .await
                .map_err(|e| format!("{e:#}"))
        }
    });
    let result = detail.read().clone();
    match result {
        None => rsx! {div{class:"page",Loading{text:"正在加载影片信息…"}}},
        Some(Err(error)) => rsx! {div{class:"page",Empty{title:"无法加载影片",desc:error}}},
        Some(Ok(detail)) => rsx! {EpisodePlayer{detail,line_index,ep_index}},
    }
}
#[component]
fn EpisodePlayer(detail: Value, line_index: usize, ep_index: usize) -> Element {
    let mut ui = use_context::<Ui>();
    let line = detail["lines"][line_index].clone();
    let episodes = array(&line["episodes"]);
    let episode = episodes.get(ep_index).cloned().unwrap_or(Value::Null);
    let raw = s(&episode, "url");
    let detail2 = detail.clone();
    let line2 = line.clone();
    let raw2 = raw.clone();
    let mut retry = use_signal(|| 0u64);
    let mut session = use_signal(String::new);
    let cancel = use_hook(CancellationToken::new);
    let cleanup = cancel.clone();
    use_drop(move || cleanup.cancel());
    let resolved = use_resource(move || {
        let detail = detail2.clone();
        let line = line2.clone();
        let raw = raw2.clone();
        let cancel = cancel.clone();
        let _ = *retry.read();
        async move {
            if raw.is_empty() {
                return Err("这条线路没有这个集数，换一条线路试试".into());
            }
            ui.backend()
                .media
                .resolve(
                    &raw,
                    &json!({"siteKey":detail["siteKey"],"flag":line["name"]}),
                    &cancel,
                )
                .await
                .map_err(|e| format!("{e:#}"))
        }
    });
    let result = resolved.read().clone();
    let address = result
        .as_ref()
        .and_then(|v| v.as_ref().ok())
        .filter(|v| v["ok"] == true)
        .map(|v| s(v, "url"))
        .unwrap_or_default();
    let error = result
        .as_ref()
        .and_then(|v| match v {
            Err(error) => Some(error.clone()),
            Ok(v) if v["ok"] != true => Some(s(v, "error")),
            _ => None,
        })
        .unwrap_or_default();
    let entry = json!({"siteKey":detail["siteKey"],"siteName":detail["siteName"],"vodId":detail["vod_id"],"name":detail["vod_name"],"pic":detail["vod_pic"],"lineIndex":line_index,"epIndex":ep_index,"epName":episode["name"]});
    let input = json!({"vodName":detail["vod_name"],"typeName":detail["type_name"],"year":detail["vod_year"],"area":detail["vod_area"],"actor":detail["vod_actor"],"director":detail["vod_director"],"score":detail["vod_score"],"remarks":detail["vod_remarks"],"content":detail["vod_content"],"epName":episode["name"],"epIndex":ep_index,"epTotal":episodes.len(),"lineName":line["name"]});
    let back = Page::Detail(s(&detail, "siteKey"), s(&detail, "vod_id"));
    let prev = Page::Player(
        s(&detail, "siteKey"),
        s(&detail, "vod_id"),
        line_index,
        ep_index.saturating_sub(1),
    );
    let next = Page::Player(
        s(&detail, "siteKey"),
        s(&detail, "vod_id"),
        line_index,
        ep_index + 1,
    );
    let alternatives = sources_for(ui, &detail);
    let external = address.clone();
    let clipboard = address.clone();
    let browser = if address.is_empty() {
        raw.clone()
    } else {
        address.clone()
    };
    let title = format!("{} {}", s(&detail, "vod_name"), s(&episode, "name"));
    let last = ui.player_event.read().clone();
    let video_error = !session.read().is_empty()
        && s(&last, "session") == *session.read()
        && (s(&last, "event") == "error"
            || (s(&last, "event") == "hlsError"
                && last["fatal"] == true
                && last["recovered"] != true));
    rsx! {div{class:"page page-player",div{class:"detail-head",button{class:"btn btn-sm",onclick:move|_|ui.go(back.clone()),Icon{name:"chevron-left",size:14}"返回"}span{class:"detail-title-inline",{s(&detail,"vod_name")}}span{class:"text-3 fs-12",{s(&line,"name")}" · "{s(&episode,"name")}}}
        div{class:"player-layout",div{class:"player-main",div{class:"player-stage",if !address.is_empty(){runtime::PlayerSurface{key:"{address}",address:address.clone(),class:"player-stage-inner",entry,onsession:move |id|session.set(id)}}if result.is_none(){div{class:"player-stage-note",span{class:"spin"}"这条线路给的不是直链，正在换播放地址…"}}}
            if !error.is_empty(){div{class:"banner banner-danger","{error}" button{class:"btn btn-sm",onclick:move|_|retry+=1,"重试"}}}else if video_error{div{class:"banner banner-danger","视频加载失败，可能是地址已失效或浏览器不支持。请换线路或使用外部播放器。"}}
            div{class:"player-bar",button{class:"btn btn-sm",disabled:ep_index==0,onclick:move|_|ui.go(prev.clone()),Icon{name:"chevron-left",size:13}"上一集"}button{class:"btn btn-sm",disabled:ep_index+1>=episodes.len(),onclick:move|_|ui.go(next.clone()),"下一集" Icon{name:"chevron-right",size:13}}span{class:"grow"}button{class:"btn btn-sm",disabled:external.is_empty(),onclick:move|_|{let address=external.clone();let title=title.clone();spawn(async move{if ui.report(crate::player::open(&ui.backend().store,&address,&title).await).is_some(){ui.notify("已交给外部播放器","success");}});},Icon{name:"external",size:13}"外部播放器"}button{class:"btn btn-sm",disabled:clipboard.is_empty(),onclick:move|_|{let text=clipboard.clone();spawn(async move{copy(ui,&text).await;});},Icon{name:"link",size:13}"复制地址"}}
            button{class:"player-url mono",title:browser.clone(),onclick:move|_|{ui.report(opener::open(&browser).map_err(anyhow::Error::from));},span{class:"player-url-text","{raw}"}span{class:"player-url-go",Icon{name:"external",size:12}"浏览器打开"}}
            AiSummary{input}
        }aside{class:"player-side",div{class:"player-side-head",Poster{src:s(&detail,"vod_pic"),name:s(&detail,"vod_name"),class:"poster-side"}div{div{class:"player-side-title",{s(&detail,"vod_name")}}div{class:"text-3 fs-12",{s(&line,"name")}}div{class:"text-3 fs-12","第 {ep_index+1} / {episodes.len()} 集"}}}
            div{class:"player-sources",div{class:"player-side-label","片源"}div{class:"source-chips",for source in alternatives{SourceItem{source,current_key:s(&detail,"siteKey"),current_id:s(&detail,"vod_id"),playing:true}}}button{class:"btn btn-sm",onclick:move|_|{let name=s(&detail,"vod_name");spawn(async move{let svc=ui.backend();if let Some(result)=ui.report(svc.catalogue.search(&name,json!({}),None).await){ui.results.set(array(&result["results"]));}});},Icon{name:"search",size:13}"找其它片源"}}
            div{class:"chips",for (index,item) in array(&detail["lines"]).iter().enumerate(){EpisodeButton{episode:item.clone(),page:Page::Player(s(&detail,"siteKey"),s(&detail,"vod_id"),index,ep_index.min(array(&item["episodes"]).len().saturating_sub(1))),active:index==line_index}}}div{class:"player-episodes",for (index,episode) in episodes.iter().cloned().enumerate(){EpisodeButton{episode,page:Page::Player(s(&detail,"siteKey"),s(&detail,"vod_id"),line_index,index),active:index==ep_index}}}
        }}
    }}
}
