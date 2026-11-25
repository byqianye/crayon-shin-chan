use super::*;
use std::collections::BTreeSet;

fn test_site(ui: Ui, key: String, mut results: Signal<Value>, audit: bool) {
    results.write()[&key] = json!({"running":true});
    spawn(async move {
        let svc = ui.backend();
        let result = if audit {
            crate::diagnostics::audit(&svc.catalogue, &key, "").await
        } else {
            crate::diagnostics::test(&svc.catalogue, &key).await
        };
        results.write()[&key] = match result {
            Ok(value) => value,
            Err(error) => json!({"ok":false,"message":format!("{error:#}")}),
        };
        ui.refresh();
    });
}

#[component]
pub(in crate::ui) fn Sites() -> Element {
    let ui = use_context::<Ui>();
    let mut word = use_signal(String::new);
    let mut kind = use_signal(|| "all".to_owned());
    let mut source = ui.source_filter;
    let mut enabled = use_signal(|| false);
    let mut show_add = use_signal(|| false);
    let tests = use_signal(|| json!({}));
    let audits = use_signal(|| json!({}));
    let mut batch_busy = use_signal(|| false);
    let mut probe = use_signal(|| json!({}));
    let mut cache_sequence = use_signal(|| 0usize);
    let cache = use_resource(move || {
        let _ = *cache_sequence.read();
        async move {
            ui.backend()
                .catalogue
                .spider
                .cache_info(&ui.sites.peek())
                .await
                .map_err(|e| format!("{e:#}"))
        }
    });
    let sites = ui.sites.read().clone();
    let total = sites.len();
    let active = sites.iter().filter(|v| v["enabled"] == true).count();
    let spiders = sites
        .iter()
        .filter(|v| !s(v, "spiderClass").is_empty())
        .count();
    let query = word.read().trim().to_lowercase();
    let names: BTreeSet<_> = sites.iter().map(|v| s(v, "sourceName")).collect();
    let visible: Vec<_> = sites
        .iter()
        .filter(|v| {
            (*kind.read() == "all" || s(v, "type") == *kind.read())
                && (source.read().is_empty() || s(v, "sourceName") == *source.read())
                && (!*enabled.read() || v["enabled"] == true)
                && (query.is_empty()
                    || ["name", "key", "api"]
                        .iter()
                        .any(|key| s(v, key).to_lowercase().contains(&query)))
        })
        .cloned()
        .collect();
    let empty = visible.is_empty();
    let to_disable: Vec<_> = visible
        .iter()
        .filter(|v| v["enabled"] == true)
        .map(|v| s(v, "key"))
        .collect();
    let to_enable: Vec<_> = visible
        .iter()
        .filter(|v| {
            v["enabled"] != true
                && !s(v, "spiderClass").is_empty()
                && s(v, "unsupported").is_empty()
        })
        .map(|v| s(v, "key"))
        .collect();
    let status = ui.backend().catalogue.spider.runtime_status();
    let batch = visible.clone();
    let cache_info = cache
        .read()
        .as_ref()
        .and_then(|v| v.as_ref().ok())
        .cloned()
        .unwrap_or(Value::Null);
    rsx! {div{class:"page",
        div{class:"page-head",div{div{class:"page-title","影视源"}div{class:"page-sub","共 {total} 个站点 · 启用 {active} 个 · 当前列表 {visible.len()} 个" if spiders>0{" · Spider {spiders} 个"}}}div{class:"page-actions",
            button{class:"btn btn-primary",onclick:move |_|show_add.toggle(),Icon{name:"plus",size:14}"添加影视源"}
            button{class:"btn btn-sm",disabled:*batch_busy.read()||batch.is_empty(),onclick:move |_|{let list=batch.clone();batch_busy.set(true);let mut tests=tests;spawn(async move{for site in list{let key=s(&site,"key");tests.write()[&key]=json!({"running":true});tests.write()[&key]=match crate::diagnostics::test(&ui.backend().catalogue,&key).await{Ok(v)=>v,Err(e)=>json!({"ok":false,"message":format!("{e:#}")})};}ui.refresh();batch_busy.set(false);ui.notify("当前列表已测完","success");});},if *batch_busy.read(){span{class:"spin"}}"测试当前列表"}
            button{class:"btn btn-sm",disabled:to_disable.is_empty(),onclick:move |_|{ui.report(ui.backend().sources.override_site(&to_disable,&json!({"enabled":false})));ui.refresh();},"全部禁用"}
            button{class:"btn btn-sm",disabled:to_enable.is_empty()||status["ready"]!=true,onclick:move |_|{ui.report(ui.backend().sources.override_site(&to_enable,&json!({"enabled":true})));ui.refresh();},"启用可见 Spider"}
        }}
        if spiders>0{div{class:if status["ready"]==true{"banner banner-info"}else{"banner banner-danger"},span{class:"grow",if status["ready"]==true{"内置 Spider 运行时可用。Spider 站点开启后即可搜索、详情与播放；首次使用会启动 JVM 并转换 jar。"}else{"Spider 运行时不可用："{s(&status,"reason")}}" · 缓存 "{number(&cache_info["bytes"],0.0).to_string()}"B / "{s(&cache_info,"files")}" 个文件" if !s(&probe.read(),"message").is_empty(){" · 自检："{s(&probe.read(),"message")}}}
            for (action,label) in [("ping","运行时自检"),("clear","清理缓存"),("restart","重新准备运行时")]{button{class:"btn btn-sm",disabled:probe.read()["running"]==true,onclick:move |_|{probe.set(json!({"running":true}));spawn(async move{let svc=ui.backend();let result=async{match action{"clear"=>{svc.catalogue.spider.clear_cache(&ui.sites.peek(),None).await?;},"restart"=>{svc.catalogue.spider.stop().await?;svc.catalogue.spider.ping().await?;},_=>{svc.catalogue.spider.ping().await?;}}Ok::<_,anyhow::Error>(())}.await;probe.set(match result{Ok(_)=>json!({"ok":true,"message":"运行时正常"}),Err(e)=>json!({"ok":false,"message":format!("{e:#}")})});cache_sequence+=1;});},"{label}"}}
        }}
        if *show_add.read(){AddSite{onclose:move |_|show_add.set(false)}}
        div{class:"filter-bar",div{class:"search-box",style:"width:260px",span{class:"search-icon",Icon{name:"search",size:15}}input{value:"{word}",placeholder:"搜索名称、key、接口地址",oninput:move |e|word.set(e.value())}}
            div{class:"chips",for (value,label) in [("all","全部"),("3","苹果CMS"),("4","JSON 接口"),("1","网页规则"),("0","Spider"),("2","其它")]{button{class:if *kind.read()==value{"chip active"}else{"chip"},onclick:move |_|kind.set(value.into()),"{label}"}}}
            div{class:"grow"}
            select{class:"input",style:"width:170px",value:"{source}",onchange:move |e|source.set(e.value()),option{value:"","全部配置源"}for name in names{option{value:name.clone(),"{name}"}}}
            button{class:if *enabled.read(){"chip active"}else{"chip"},onclick:move |_|enabled.toggle(),"只看已启用"}
        }
        div{class:"panel",div{class:"list",for site in visible{SiteRow{key:"{site}",site,tests,audits,cache:cache_info.clone(),oncache:move |_|cache_sequence+=1}}if total==0{Empty{title:"还没有任何影视源",desc:"可以直接添加单个苹果CMS / JSON 接口，也可以在「设置 → 配置源」导入 TVBox 订阅。"}}else if empty{Empty{title:"没有匹配的站点",desc:"换个关键词或清空筛选条件试试。"}}}}
    }}
}

#[component]
fn SiteRow(
    site: Value,
    tests: Signal<Value>,
    audits: Signal<Value>,
    cache: Value,
    oncache: Callback<()>,
) -> Element {
    let ui = use_context::<Ui>();
    let mut editing = use_signal(|| false);
    let key = s(&site, "key");
    let key_toggle = key.clone();
    let key_test = key.clone();
    let key_audit = key.clone();
    let key_delete = key.clone();
    let key_cache = key.clone();
    let test = tests.read()[&key].clone();
    let audit = audits.read()[&key].clone();
    let manual = s(&site, "origin") == "manual";
    let health = ui
        .backend()
        .store
        .read("health", json!({}))
        .unwrap_or_else(|_| json!({}));
    let health = crate::health::summary(&health[&key]);
    let cached = array(&cache["sites"])
        .into_iter()
        .find(|v| s(v, "siteKey") == key)
        .unwrap_or(Value::Null);
    rsx! {if *editing.read(){LocalEdit{site:site.clone(),onclose:move |_|editing.set(false)}}else{div{class:"list-row",
        Toggle{checked:site["enabled"]==true,disabled:!s(&site,"unsupported").is_empty(),title:"启用 / 停用该站点",onchange:move |value|{ui.report(ui.backend().sources.override_site(std::slice::from_ref(&key_toggle),&json!({"enabled":value})));ui.refresh();}}
        div{class:"list-row-main",div{class:"list-row-title site-name",span{class:"truncate",{s(&site,"name")}}span{class:"tag tag-outline",{type_label(&site)}}if manual{span{class:"tag tag-accent","手动"}}span{class:"tag tag-outline",if site["searchable"]==true{"可搜索"}else{"仅浏览"}}}
            div{class:"list-row-sub",span{class:"site-key","{key}"}" · " if !s(&site,"unsupported").is_empty(){{s(&site,"unsupported")}}else{{s(&site,"api")}}if !s(&site,"spiderClass").is_empty(){" · 内置 JVM 运行 · 来源 "{s(&site,"jar")}if cached["cached"]==true{" · 缓存 "{s(&cached,"bytes")}"B · SHA256 "{s(&cached,"sha256").chars().take(12).collect::<String>()}}}}
            if audit["running"]==true{div{class:"list-row-sub",span{class:"spin"}" 正在审计…"}}else if !audit.is_null(){div{class:"list-row-sub",if audit["ok"]==true{"审计通过：分类 "{s(&audit,"categories")}" · 搜索 "{s(&audit,"searchHits")}" 条 · 线路 "{s(&audit,"detailLines")}" / "{s(&audit,"episodes")}" 集"}else{"审计失败（"{s(&audit,"failStage")}"）："{s(&audit,"reason")}}}}
        }
        span{class:"health",if test["running"]==true{span{class:"spin"}}else if !test.is_null(){span{class:if test["ok"]==true{"dot ok"}else{"dot bad"}}{s(&test,"message")}}else if number(&health["samples"],0.0)>0.0{{format!("{}% · {}ms",(number(&health["successRate"],0.0)*100.0).round(),s(&health,"avgMs"))}}else{"未测"}}
        div{class:"list-row-actions",button{class:"btn btn-sm",disabled:test["running"]==true||!s(&site,"unsupported").is_empty(),onclick:move |_|test_site(ui,key_test.clone(),tests,false),"测试"}button{class:"btn btn-sm",disabled:audit["running"]==true||!s(&site,"unsupported").is_empty(),onclick:move |_|test_site(ui,key_audit.clone(),audits,true),"审计"}
            if !s(&site,"spiderClass").is_empty(){button{class:"btn btn-sm",onclick:move |_|{let key=key_cache.clone();spawn(async move{if ui.report(ui.backend().catalogue.spider.clear_cache(&ui.sites.peek(),Some(&key)).await).is_some(){oncache.call(());}});},"清缓存"}}
            if manual{button{class:"btn btn-sm",onclick:move |_|editing.set(true),"编辑"}}
            button{class:"btn btn-sm",onclick:move |_|{let key=key_delete.clone();spawn(async move{if confirm(if manual{"删除这个手动影视源？"}else{"忽略该站点，后续同步也不会复活。继续？"}).await{let result=if manual{ui.backend().sources.remove_local(&key)}else{ui.backend().sources.tombstone("siteKeys",&key,true)};ui.report(result);ui.refresh();}});},if manual{"删除"}else{"忽略"}}
        }
    }}}
}

fn hint(result: &Value) -> String {
    match s(result, "kind").as_str() {
        "cms" => "识别为苹果CMS 接口".into(),
        "html" => "识别为网页，可作为网页规则源添加".into(),
        "tvbox" | "multirepo" | "m3u" | "encrypted" | "image" => {
            "识别为配置或直播源，请到「设置 → 配置源」添加".into()
        }
        _ => "未能识别类型，请手动选择".into(),
    }
}
fn single(result: &Value) -> bool {
    matches!(s(result, "kind").as_str(), "" | "cms" | "html" | "unknown")
}
fn detected_type(result: &Value) -> Value {
    match s(result, "kind").as_str() {
        "cms" => json!(3),
        "html" => json!(1),
        _ => Value::Null,
    }
}

#[component]
fn AddSite(onclose: Callback<()>) -> Element {
    let ui = use_context::<Ui>();
    let mut batch = use_signal(|| false);
    let mut name = use_signal(String::new);
    let mut address = use_signal(String::new);
    let mut kind = use_signal(|| "auto".to_owned());
    let mut detected = use_signal(|| json!({}));
    let mut text = use_signal(String::new);
    let mut busy = use_signal(|| false);
    rsx! {div{class:"panel",style:"margin-bottom:14px",div{class:"section-head",div{div{class:"section-title","添加影视源"}div{class:"section-more","支持苹果CMS 采集接口和网页规则站；TVBox 订阅 / 多仓请到「设置 → 配置源」添加"}}div{class:"page-actions",button{class:if !*batch.read(){"chip active"}else{"chip"},onclick:move |_|batch.set(false),"单个添加"}button{class:if *batch.read(){"chip active"}else{"chip"},onclick:move |_|batch.set(true),"批量导入"}button{class:"btn btn-sm",onclick:move |_|onclose.call(()),"收起"}}}
        if !*batch.read(){div{class:"form-row",div{class:"field",label{"名称（可选）"}input{class:"input",value:"{name}",oninput:move |e|name.set(e.value()),placeholder:"留空则用域名"}}div{class:"field grow",label{"接口地址"}input{class:"input",value:"{address}",oninput:move |e|{address.set(e.value());detected.set(json!({}));}}}div{class:"field",label{"类型"}select{class:"input",value:"{kind}",onchange:move |e|kind.set(e.value()),for (v,label) in [("auto","自动判断"),("3","苹果CMS"),("4","JSON 接口"),("1","网页规则")]{option{value:v,"{label}"}}}}}
            div{class:"form-row",button{class:"btn",disabled:*busy.read(),onclick:move |_|{busy.set(true);spawn(async move{if let Some(result)=ui.report(ui.backend().sources.detect(&address.peek()).await){if name.peek().is_empty(){name.set(s(&result,"name"));}detected.set(result);}busy.set(false);});},"探测类型"}button{class:"btn btn-primary",disabled:*busy.read()||!single(&detected.read()),onclick:move |_|{let input=json!({"name":name.peek().trim(),"api":address.peek().trim(),"type":if *kind.peek()=="auto"{detected_type(&detected.peek())}else{json!(kind.peek().parse::<usize>().unwrap_or(4))}});if ui.report(ui.backend().sources.add_local(&input)).is_some(){ui.refresh();onclose.call(());ui.notify("已添加影视源","success");}},"添加"}span{class:"text-3 fs-13",{hint(&detected.read())}}}
        }else{div{class:"field",label{"每行一个接口地址"}textarea{class:"input",rows:6,value:"{text}",oninput:move |e|text.set(e.value())}}button{class:"btn btn-primary",disabled:*busy.read(),onclick:move |_|{busy.set(true);let addresses:Vec<_>=text.peek().lines().map(str::trim).filter(|v|!v.is_empty()).map(str::to_owned).collect();spawn(async move{let svc=ui.backend();let(mut ok,mut fail,mut skipped)=(0,0,0);for api in addresses{match svc.sources.detect(&api).await{Ok(result)if single(&result)=>{match svc.sources.add_local(&json!({"api":api,"type":detected_type(&result)})){Ok(_)=>ok+=1,Err(_)=>fail+=1}},Ok(_)=>skipped+=1,Err(_)=>fail+=1}}ui.refresh();ui.notify(format!("批量导入完成：成功 {ok}，失败 {fail}，跳过 {skipped} 个非单源配置"),if fail+skipped>0{"warn"}else{"success"});busy.set(false);if ok>0{onclose.call(());}});},if *busy.read(){span{class:"spin"}}"开始导入"}}
    }}
}

#[component]
fn LocalEdit(site: Value, onclose: Callback<()>) -> Element {
    let ui = use_context::<Ui>();
    let initial = site.clone();
    let mut draft = use_signal(|| initial);
    let key = s(&site, "key");
    rsx! {div{class:"list-row",div{class:"form-row grow",for (field,label) in [("name","名称"),("api","接口地址")]{div{class:"field grow",label{"{label}"}input{class:"input",value:s(&draft.read(),field),oninput:move |e|draft.write()[field]=json!(e.value())}}}div{class:"field",label{"类型"}select{class:"input",value:s(&draft.read(),"type"),onchange:move |e|draft.write()["type"]=json!(e.value().parse::<usize>().unwrap_or(4)),for (v,label) in [(3,"苹果CMS"),(4,"JSON 接口"),(1,"网页规则")]{option{value:"{v}","{label}"}}}}}button{class:"btn btn-sm btn-primary",onclick:move |_|{if ui.report(ui.backend().sources.update_local(&key,&json!({"name":draft.peek()["name"],"api":draft.peek()["api"],"type":draft.peek()["type"]}))).is_some(){ui.refresh();onclose.call(());}},"保存"}button{class:"btn btn-sm",onclick:move |_|onclose.call(()),"取消"}}}
}
