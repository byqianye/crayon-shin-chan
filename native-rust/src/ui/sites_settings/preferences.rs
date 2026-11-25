use super::*;

#[component]
pub(super) fn GeneralSettings() -> Element {
    let ui = use_context::<Ui>();
    let mut draft = use_signal(|| ui.settings.peek().clone());
    let mut players = use_signal(|| None::<Vec<Value>>);
    let mut detecting = use_signal(|| false);
    use_effect(move || draft.set(ui.settings.read().clone()));
    let dirty = *draft.read() != *ui.settings.read();
    let data = draft.read().clone();
    rsx! {section{class:"section",div{class:"section-head",div{class:"section-title","通用"}div{class:"section-more",if dirty{"有未保存的修改"}else{"已保存"}}}div{class:"panel",
        div{class:"form-row",
            for (field,label,min,max,fallback,hint) in [("concurrency","聚合搜索并发数",1,32,8,"同时请求的站点数量，过大可能被部分站点限流（1–32）"),("timeout","请求超时（秒）",3,120,20,"单个站点搜索与详情请求的最长等待时间")]{div{class:"field",label{"{label}"}input{class:"input",r#type:"number",min:"{min}",max:"{max}",value:s(&data,field),oninput:move |e|draft.write()[field]=json!(e.value().parse::<usize>().unwrap_or(fallback).clamp(min,max))}div{class:"field-hint","{hint}"}}}
            div{class:"field",label{"界面密度"}select{class:"input",value:s(&data,"density"),onchange:move |e|draft.write()["density"]=json!(e.value()),option{value:"compact","紧凑（推荐，信息更多）"}option{value:"comfortable","宽松"}}}
            div{class:"field",style:"flex:0 0 140px",label{"强调色"}div{class:"hstack",input{class:"input",r#type:"color",style:"width:48px;padding:2px",value:s(&data,"accent"),oninput:move |e|draft.write()["accent"]=json!(e.value())}span{class:"mono fs-12",{s(&data,"accent")}}}}
            div{class:"field",label{"搜索结果过滤解说"}div{class:"hstack",Toggle{checked:flag(&data["filterCommentary"],true),title:"过滤解说 / 速看 / 混剪",onchange:move |value|draft.write()["filterCommentary"]=json!(value)}span{class:"fs-13 text-2",if flag(&data["filterCommentary"],true){"隐藏解说 / 速看 / 混剪"}else{"全部显示"}}}div{class:"field-hint","采集站常把解说和正片放一起，默认挡掉"}}
        }
        div{class:"field",style:"margin-top:8px",label{"User-Agent"}input{class:"input",value:s(&data,"userAgent"),oninput:move |e|draft.write()["userAgent"]=json!(e.value())}div{class:"field-hint","部分站点会校验 UA，异常时可以先恢复默认"}}
        div{class:"divider"}
        div{class:"field",label{"外部播放器命令"}input{class:"input",placeholder:"留空则自动探测 PotPlayer / mpv / VLC",value:s(&data["externalPlayer"],"command"),oninput:move |e|draft.write()["externalPlayer"]["command"]=json!(e.value())}div{class:"field-hint",{"参数模板用 {url} 代表播放地址，{title} 代表标题，多个参数用空格分隔，例如 --fullscreen {url}"}}}
        div{class:"field",label{"本机播放器"}div{class:"hstack",button{class:"btn btn-sm",disabled:*detecting.read(),onclick:move |_|{detecting.set(true);spawn(async move{if let Some(found)=ui.report(crate::player::detect(&ui.backend().store).await){players.set(Some(found));}detecting.set(false);});},if *detecting.read(){span{class:"spin"}}else{Icon{name:"search",size:14}}"检测本机播放器"}span{class:"fs-12 text-3",if let Some(list)=players.read().as_ref(){if list.is_empty(){"一个都没找到，装在别处的话请直接填上面的路径"}else{"找到 {list.len()} 个"}}else{"留空时按 PotPlayer → mpv → VLC 的顺序自动找"}}}}
        if let Some(list)=players.read().clone(){div{class:"list",for player in list{{let path=s(&player,"path");rsx!{div{class:"list-row",div{class:"list-row-main",div{class:"list-row-title",{s(&player,"name")}if player["source"]=="registry"{span{class:"tag tag-outline","系统默认打开方式"}}}div{class:"list-row-sub mono truncate","{path}"}}if data["externalPlayer"]["command"]==path{span{class:"tag tag-success","已选"}}else{button{class:"btn btn-sm",onclick:move |_|draft.write()["externalPlayer"]["command"]=json!(path),"用这个"}}}}}}}}
        div{class:"field",style:"max-width:420px",label{"参数模板"}input{class:"input mono",value:array(&data["externalPlayer"]["args"]).iter().map(crate::model::text).collect::<Vec<_>>().join(" "),oninput:move |e|draft.write()["externalPlayer"]["args"]=json!(e.value().split_whitespace().collect::<Vec<_>>())}}
        div{class:"hstack",style:"margin-top:14px",button{class:"btn btn-primary",disabled:!dirty,onclick:move |_|{ui.patch(draft.peek().clone());ui.notify("设置已保存","success");},"保存设置"}button{class:"btn",disabled:!dirty,onclick:move |_|draft.set(ui.settings.peek().clone()),"放弃修改"}}
    }}}
}

#[component]
pub(super) fn AiSettings() -> Element {
    let ui = use_context::<Ui>();
    let mut draft = use_signal(|| ui.settings.peek()["ai"].clone());
    let mut show = use_signal(|| false);
    use_effect(move || draft.set(ui.settings.read()["ai"].clone()));
    let data = draft.read().clone();
    let dirty = data != ui.settings.read()["ai"];
    rsx! {section{class:"section",div{class:"section-head",div{class:"section-title","AI 总结"}div{class:"section-more",if dirty{"有未保存的修改"}else if s(&data,"apiKey").is_empty(){"还没配置 API Key"}else{"已配置"}}}div{class:"panel panel-body-pad",
        div{class:"field",label{"DeepSeek API Key"}div{class:"hstack",input{class:"input mono grow",r#type:if *show.read(){"text"}else{"password"},placeholder:"sk-...",autocomplete:"off",spellcheck:"false",value:s(&data,"apiKey"),oninput:move |e|draft.write()["apiKey"]=json!(e.value())}button{class:"btn btn-sm",onclick:move |_|show.toggle(),if *show.read(){"隐藏"}else{"显示"}}}div{class:"field-hint","在 platform.deepseek.com 的「API keys」里创建。只保存在本机 data/settings.json。"}}
        div{class:"field",style:"max-width:260px",label{"模型"}input{class:"input mono",list:"ai-models",value:s(&data,"model"),oninput:move |e|draft.write()["model"]=json!(e.value())}datalist{id:"ai-models",option{value:"deepseek-flash"}option{value:"deepseek-v4-pro"}}div{class:"field-hint","使用支持所选总结方式的模型；画面总结需要视觉能力。"}}
        div{class:"field",label{"接口地址"}input{class:"input mono",placeholder:"留空 = https://api.deepseek.com",value:s(&data,"baseUrl"),oninput:move |e|draft.write()["baseUrl"]=json!(e.value())}div{class:"field-hint","换成任何兼容的服务都可以，会自动拼 /chat/completions"}}
        div{class:"banner banner-info","点播放页的「AI 总结本集」时，会把片名、集名、类型、年份、演员和站点简介发给配置的 AI 服务；画面总结还会发送抽取的画面。不会发送播放地址，也不会在后台自动调用。"}
        div{class:"hstack",style:"margin-top:14px",button{class:"btn btn-primary",disabled:!dirty,onclick:move |_|ui.patch(json!({"ai":draft.peek().clone()})),"保存设置"}button{class:"btn",disabled:!dirty,onclick:move |_|draft.set(ui.settings.peek()["ai"].clone()),"放弃修改"}}
    }}}
}
