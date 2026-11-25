use super::*;

#[component]
pub(in crate::ui) fn Settings() -> Element {
    let ui = use_context::<Ui>();
    let mut name = use_signal(String::new);
    let mut address = use_signal(String::new);
    let mut adding = use_signal(|| false);
    let ignored = ui
        .backend()
        .sources
        .tombstones()
        .unwrap_or_else(|_| json!({}));
    let ignored_count = array(&ignored["sourceUrls"]).len() + array(&ignored["siteKeys"]).len();
    let sources = ui.sources.read().clone();
    let errors: Vec<_> = sources
        .iter()
        .filter(|v| !s(v, "error").is_empty())
        .map(|v| format!("{}：{}", s(v, "name"), s(v, "error")))
        .collect();
    let data_dir = ui.backend().store.root.display().to_string();
    rsx! {div{class:"page",div{class:"page-head",div{div{class:"page-title","设置"}div{class:"page-sub","配置源、播放与界面偏好"}}}div{class:"settings-grid",
        section{class:"section",div{class:"section-head",div{class:"section-title","配置源"}div{class:"section-more","支持 TVBox 标准 JSON 与本项目的 tvbox-desktop/1 简洁格式"}}
            div{class:"panel",div{class:"form-row",div{class:"field",style:"flex:0 0 180px",label{"名称（可选）"}input{class:"input",value:"{name}",placeholder:"留空则用配置里的 name",oninput:move |e|name.set(e.value())}}div{class:"field grow",label{"订阅地址 / 本地文件路径"}input{class:"input",value:"{address}",placeholder:"https://example.com/tvbox.json 或本地配置路径",oninput:move |e|address.set(e.value())}}
                button{class:"btn",onclick:move |_|{spawn(async move{if let Some(file)=rfd::AsyncFileDialog::new().add_filter("配置文件",&["json","txt"]).pick_file().await{let path=file.path();address.set(path.display().to_string());if name.peek().trim().is_empty(){name.set(path.file_stem().map(|v|v.to_string_lossy().into_owned()).unwrap_or_default());}}});},"选择文件…"}
                button{class:"btn btn-primary",disabled:*adding.read(),onclick:move |_|{let input=json!({"name":name.peek().trim(),"url":address.peek().trim()});if let Some(created)=ui.report(ui.backend().sources.add(&input)){name.set(String::new());address.set(String::new());adding.set(true);ui.refresh();spawn(async move{ui.report(ui.backend().sources.sync(&s(&created,"id")).await);ui.refresh();adding.set(false);});}},if *adding.read(){span{class:"spin"}}else{"+"}" 添加"}
            }}
            div{class:"panel",style:"margin-top:12px",div{class:"panel-body-pad",div{class:"field-hint",style:"margin-bottom:8px","内置推荐订阅（点「添加」写进配置源）"}PresetRows{}}}
            div{class:"panel",style:"margin-top:12px",div{class:"list",for source in sources{SourceRow{key:"{source}",source}}if ui.sources.read().is_empty(){Empty{title:"还没有配置源",desc:"用上面的表单添加一个吧。"}}}}
            if !errors.is_empty(){div{class:"banner banner-warn",style:"margin-top:12px",b{"同步失败详情："}{errors.join("　|　")}}}
        }
        section{class:"section",div{class:"section-head",div{class:"section-title","忽略列表"}div{class:"section-more","删除过的配置源、手动忽略的站点，同步时不会复活"}}div{class:"panel panel-body-pad",div{class:"form-row",span{class:"fs-13","配置源 "{array(&ignored["sourceUrls"]).len().to_string()}" 个 · 站点 "{array(&ignored["siteKeys"]).len().to_string()}" 个"}div{class:"grow"}button{class:"btn btn-sm",disabled:ignored_count==0,onclick:move |_|{spawn(async move{if confirm("清空后，之前删除/忽略的源可能在下次同步时重新出现。继续？").await&& ui.report(ui.backend().sources.clear_tombstones()).is_some(){ui.refresh();ui.notify("已清空忽略列表","success");}});},"清空忽略列表"}}div{class:"field-hint","手动源删除后不会进入站点墓碑；清空后重新同步订阅即可恢复被忽略的站点。"}}}
        super::parses::ParseSettings{}
        super::preferences::GeneralSettings{}
        super::preferences::AiSettings{}
        section{class:"section",div{class:"section-head",div{class:"section-title","配置格式"}div{class:"section-more","完整规范见项目根目录「功能框架.txt」第 5 章"}}div{class:"panel",div{class:"code-block",{JSON_SAMPLE}}div{class:"fs-12 text-3",style:"margin-top:10px;line-height:1.8",div{"· A 方言：与安卓版 TVBox 相同的 sites / lives / parses 结构，可直接导入现成订阅。"}div{"· B 方言：加上 format: tvbox-desktop/1，站点可以只写 type: cms 或 html，其余字段自动归一化。"}div{"· 桌面端内置 Spider 宿主，可下载并转换 spider jar。只有缺少 spider 文件或不兼容的站点会显示原因。"}}}}
        section{class:"section",div{class:"section-head",div{class:"section-title","关于"}button{class:"btn btn-ghost btn-sm",onclick:move |_|ui.go(Page::Sites),"查看影视源 →"}}div{class:"panel",
            div{class:"kv",div{class:"kv-key","版本"}div{class:"kv-value mono","v"{env!("CARGO_PKG_VERSION")}}}
            div{class:"kv",div{class:"kv-key","运行环境"}div{class:"kv-value mono","Rust · Dioxus · Microsoft Edge WebView2"}}
            div{class:"kv",div{class:"kv-key","数据目录"}div{class:"kv-value mono truncate",title:data_dir.clone(),"{data_dir}"}button{class:"btn btn-sm",onclick:move |_|{ui.report(crate::player::open_path(&ui.backend().store.root));},"打开目录"}}
            div{class:"kv",div{class:"kv-key","开源组件"}div{class:"kv-value","Dioxus · ArtPlayer · hls.js · reqwest · scraper · Java Spider"}}
        }div{class:"banner banner-info",style:"margin-top:12px","本软件是纯粹的空壳播放器，不内置、不提供、不分发任何影视内容。请自行添加合法的配置源。"}}
    }}}
}

#[component]
fn SourceRow(source: Value) -> Element {
    let ui = use_context::<Ui>();
    let mut editing = use_signal(|| false);
    let initial = source.clone();
    let mut draft = use_signal(|| initial);
    let id = s(&source, "id");
    let toggle = id.clone();
    let sync = id.clone();
    let edit = id.clone();
    let delete = id.clone();
    let source_name = s(&source, "name");
    rsx! {div{class:"list-row",Toggle{checked:flag(&source["enabled"],true),title:"启用 / 停用该配置源",onchange:move |value|{ui.report(ui.backend().sources.update(&toggle,&json!({"enabled":value})));ui.refresh();}}
        if *editing.read(){div{class:"field grow",input{class:"input",value:s(&draft.read(),"name"),placeholder:"名称",oninput:move |e|draft.write()["name"]=json!(e.value())}}div{class:"field grow",input{class:"input",value:s(&draft.read(),"url"),oninput:move |e|draft.write()["url"]=json!(e.value())}}button{class:"btn btn-sm btn-primary",onclick:move |_|{if ui.report(ui.backend().sources.update(&edit,&draft.peek())).is_some(){editing.set(false);ui.refresh();ui.notify("已保存修改","success");}},"保存"}button{class:"btn btn-sm",onclick:move |_|editing.set(false),"取消"}}
        else{div{class:"list-row-main",div{class:"list-row-title",span{class:"truncate",{s(&source,"name")}}span{class:"tag tag-outline",match s(&source,"kind").as_str(){"url"=>"订阅","multirepo"=>"多仓",_=>"本地"}}if !s(&source,"error").is_empty(){span{class:"tag tag-danger","异常"}}}div{class:"list-row-sub truncate",title:s(&source,"url"),{s(&source,"url")}if source["kind"]=="multirepo"{" · "{array(&source["repos"]).len().to_string()}" 个仓库（"{array(&source["repos"]).iter().filter(|v|v["ok"]==true).count().to_string()}" 成功）"}}}
            span{class:"fs-12 text-3 nowrap","站点 "{s(&source,"siteCount")}if source["usableSiteCount"].is_number(){"（可用 "{s(&source,"usableSiteCount")}"）"}" · 直播 "{s(&source,"liveCount")}" · 解析 "{s(&source,"parseCount")}}
            span{class:"fs-12 text-3 nowrap",{relative(&source["lastSync"])}}
            div{class:"list-row-actions",button{class:"btn btn-sm",disabled:*ui.busy.read(),onclick:move |_|ui.sync(Some(sync.clone())),Icon{name:"refresh",size:13}"同步"}button{class:"btn btn-sm",onclick:move |_|{draft.set(source.clone());editing.set(true);},"编辑"}button{class:"btn btn-sm btn-danger",onclick:move |_|{let id=delete.clone();let name=source_name.clone();spawn(async move{if confirm(&format!("确定要删除配置源「{name}」吗？该操作不影响磁盘上的原始文件。")).await{ui.report(ui.backend().sources.remove(&id));ui.refresh();}});},"删除"}}
        }
    }}
}

const JSON_SAMPLE: &str = r#"{
  "format": "tvbox-desktop/1",
  "name": "我的配置",
  "sites": [
    { "key": "demo", "name": "示例站", "type": "cms",
      "api": "https://demo.example.com/api.php/provide/vod" },
    { "key": "html", "name": "网页站", "type": "html",
      "api": "https://demo.example.com/search?wd={wd}",
      "rules": { "list": ".result li", "name": "a", "link": "a" } }
  ],
  "lives": [ { "name": "默认直播", "type": 0, "url": "https://example.com/live.txt" } ],
  "parses": [ { "name": "解析线路", "type": 1, "url": "https://example.com/jx?url=" } ]
}"#;
