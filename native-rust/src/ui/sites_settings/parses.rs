use super::*;

fn run_test(
    ui: Ui,
    addresses: Vec<String>,
    mut busy: Signal<String>,
    mut results: Signal<Value>,
    mut list: Signal<Vec<Value>>,
) {
    spawn(async move {
        for address in addresses {
            busy.set(address.clone());
            if let Some(value) = ui.report(ui.backend().media.test_parse(&address, "").await) {
                results.write()[&address] = value;
            }
            if let Some(value) = ui.report(ui.backend().media.parses()) {
                list.set(value);
            }
        }
        busy.set(String::new());
    });
}

#[component]
pub(super) fn ParseSettings() -> Element {
    let ui = use_context::<Ui>();
    let mut list = use_signal(Vec::new);
    let busy = use_signal(String::new);
    let mut results = use_signal(|| json!({}));
    let mut name = use_signal(String::new);
    let mut address = use_signal(String::new);
    let mut flags = use_signal(String::new);
    use_effect(move || {
        let _ = *ui.revision.read();
        if let Some(value) = ui.report(ui.backend().media.parses()) {
            list.set(value);
        }
    });
    let all = list.read().clone();
    let count = all.len();
    let custom = all.iter().filter(|v| v["origin"] == "custom").count();
    let test_all = all.iter().map(|v| s(v, "url")).collect::<Vec<_>>();
    rsx! {section{class:"section",div{class:"section-head",div{class:"section-title","解析接口"}div{class:"section-more",if count>0{"{count} 条（配置里 {count-custom} · 手动 {custom}）"}else{"一条都没有"}}}div{class:"panel panel-body-pad",
        div{class:"fs-12 text-3",style:"line-height:1.8;margin-bottom:12px","有些采集站给的线路是分享页地址。播放时应用会先读那个页面，读不出来才按下面的排行依次问解析接口。失败的接口会排到后面，越用越准。"}
        if count>0{div{class:"list",for item in all{{let url=s(&item,"url");let test=url.clone();let delete=url.clone();let result=results.read()[&url].clone();let stat=&item["stat"];rsx!{
            div{class:"list-row",div{class:"list-row-main",div{class:"list-row-title",{s(&item,"name") }span{class:if item["origin"]=="custom"{"tag tag-accent"}else{"tag tag-outline"},if item["origin"]=="custom"{"手动添加"}else{"配置里带的"}}if item["type"]==0{span{class:"tag tag-warn","type=0 需浏览器嗅探"}}}div{class:"list-row-sub mono truncate","{url}"}div{class:"list-row-sub",if array(&item["flags"]).is_empty(){"通配所有线路"}else{"只解析 "{array(&item["flags"]).iter().map(crate::model::text).collect::<Vec<_>>().join(" / ")}}" · " if number(&stat["ok"],0.0)+number(&stat["fail"],0.0)==0.0{"还没测过"}else{"成功 "{s(stat,"ok")}" · 失败 "{s(stat,"fail")}if number(&stat["lastOk"],0.0)>0.0{" · 最近 "{s(stat,"lastMs")}"ms"}}}if !result.is_null(){div{class:"list-row-sub",if result["ok"]==true{"✓ "{s(&result,"ms")}"ms 换到 "{s(&result,"url")}}else{"✗ "{s(&result,"error")}}}}}
                div{class:"list-row-actions",button{class:"btn btn-sm",disabled:!busy.read().is_empty(),onclick:move |_|run_test(ui,vec![test.clone()],busy,results,list),if *busy.read()==url{span{class:"spin"}}else{Icon{name:"check",size:13}}"测一下"}if item["origin"]=="custom"{button{class:"btn btn-sm btn-danger",onclick:move |_|{let address=delete.clone();spawn(async move{if confirm("删除这条解析接口？").await&& let Some(value)=ui.report(ui.backend().media.remove_parse(&address)){list.set(value);}});},Icon{name:"trash",size:13}}}}
            }
        }}}}}else{div{class:"fs-13 text-3","还没解析接口。TVBox 订阅里如果有 parses 字段，同步后会自动出现在这里；也可以手动加一条。"}}
        div{class:"divider"}div{class:"form-row",div{class:"field",label{"名称"}input{class:"input",placeholder:"例如 某解析",value:"{name}",oninput:move |e|name.set(e.value())}}div{class:"field grow",label{"接口地址"}input{class:"input mono",placeholder:"https://…/?url=",value:"{address}",oninput:move |e|address.set(e.value())}}div{class:"field",label{"只解析哪些线路"}input{class:"input mono",placeholder:"留空 = 全部",value:"{flags}",oninput:move |e|flags.set(e.value())}}button{class:"btn btn-primary",disabled:address.read().trim().is_empty(),onclick:move |_|{let input=json!({"name":name.peek().trim(),"url":address.peek().trim(),"flags":flags.peek().split(|c:char|c.is_whitespace()||c==','||c=='|').filter(|v|!v.is_empty()).collect::<Vec<_>>()});if let Some(value)=ui.report(ui.backend().media.add_parse(&input)){list.set(value);name.set(String::new());address.set(String::new());flags.set(String::new());ui.notify("解析接口已添加","success");}},Icon{name:"plus",size:14}"添加"}}
        if count>0{div{class:"hstack",style:"margin-top:10px",button{class:"btn btn-sm",disabled:!busy.read().is_empty(),onclick:move |_|run_test(ui,test_all.clone(),busy,results,list),Icon{name:"check",size:13}"逐个测一遍"}button{class:"btn btn-sm",onclick:move |_|{if ui.report(ui.backend().store.write("parse-stats",&json!({}))).is_some(){results.set(json!({}));if let Some(value)=ui.report(ui.backend().media.parses()){list.set(value);}ui.notify("排行已清空","success");}},"清空排行"}}}
    }}}
}
