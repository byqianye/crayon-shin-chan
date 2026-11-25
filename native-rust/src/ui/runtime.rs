use super::*;
use crate::{
    catalogue::Catalogue,
    live::Live,
    media::{Media, MediaProxy},
    playback::Playback,
    sources::Sources,
};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::{Duration, timeout};

pub(super) struct Services {
    pub store: Arc<Store>,
    pub sources: Arc<Sources>,
    pub catalogue: Arc<Catalogue>,
    pub media: Arc<Media>,
    pub live: Arc<Live>,
    proxy: tokio::sync::OnceCell<MediaProxy>,
    playback: Playback,
}
impl Services {
    pub fn new(store: Arc<Store>) -> Result<Self> {
        let sources = Arc::new(Sources::new(store.clone())?);
        let catalogue = Arc::new(Catalogue::new(sources.clone()));
        let media = Arc::new(Media::new(catalogue.clone())?);
        let live = Arc::new(Live::new(media.clone()));
        let playback = Playback::new(store.clone());
        Ok(Self {
            store,
            sources,
            catalogue,
            media,
            live,
            proxy: tokio::sync::OnceCell::new(),
            playback,
        })
    }
    pub async fn playable(&self, address: &str) -> Result<String> {
        if crate::http::is_http_url(address) {
            Ok(self
                .proxy
                .get_or_try_init(|| self.media.start_proxy())
                .await?
                .url(address))
        } else {
            anyhow::bail!(
                "这个协议需要使用外部播放器：{}",
                address.split(':').next().unwrap_or("未知")
            );
        }
    }
}

pub(super) fn install(mut ui: Ui) {
    use_future(move || async move {
        let libraries = format!(
            "if(!window.Artplayer){{const el=document.createElement('script');el.textContent={};document.head.append(el);}}if(!window.Hls){{const el=document.createElement('script');el.textContent={};document.head.append(el);}}",
            json!(ARTPLAYER),
            json!(HLS)
        );
        let mut events = document::eval(&format!(
            "window.__crayonSend=(v)=>dioxus.send(v);window.__crayonPlayers??=new Map();{libraries};await new Promise(()=>{{}});"
        ));
        while let Ok(event) = events.recv::<Value>().await {
            let svc = ui.backend();
            let Some(outcome) = ui.report(svc.playback.event(&event, now())) else {
                continue;
            };
            let Some(event) = outcome.event else {
                continue;
            };
            let session = s(&event, "session");
            if let Some(action) = outcome.action {
                let seek = action.get("seek").map(|v| format!("art.currentTime={v};art.__emit?.('resumed');art.notice.show='已继续播放至 {}';", clock(number(v, 0.0)))).unwrap_or_default();
                let recover = action
                    .get("recover")
                    .map(|v| format!("art.hls?.[{v}]();"))
                    .unwrap_or_default();
                let _ = document::eval(&format!(
                    "{{const art=window.__crayonPlayers?.get({});if(art){{{seek}{recover}}}}}",
                    json!(session)
                ));
            }
            if s(&event, "event") == "levels" {
                let levels = array(&event["levels"]);
                if let Some((index, _)) = levels.iter().enumerate().max_by(|(_, a), (_, b)| {
                    number(&a["height"], 0.0)
                        .total_cmp(&number(&b["height"], 0.0))
                        .then_with(|| {
                            number(&a["bitrate"], 0.0).total_cmp(&number(&b["bitrate"], 0.0))
                        })
                }) {
                    let _ = document::eval(&format!(
                        "{{const art=window.__crayonPlayers?.get({});if(art?.hls)art.hls.nextLevel={index};}}",
                        json!(session)
                    ));
                }
            }
            if outcome.saved {
                ui.refresh_library();
            }
            if s(&event, "event") != "timeupdate" {
                ui.player_event.set(event);
            }
        }
    });
    let desktop = dioxus::desktop::use_window();
    let close_svc = ui.backend();
    let close_window = desktop.clone();
    let closing = use_hook(|| Arc::new(AtomicBool::new(false)));
    use_hook(move || {
        desktop.set_close_behavior(dioxus::desktop::WindowCloseBehaviour::WindowHides);
    });
    dioxus::desktop::use_wry_event_handler(move |event, _| {
        use dioxus::desktop::tao::event::{Event, WindowEvent};
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            if closing.swap(true, Ordering::SeqCst) {
                return;
            }
            let svc = close_svc.clone();
            let window = close_window.clone();
            spawn(async move {
                let snapshot = document::eval(
                    "return Array.from(window.__crayonPlayers||[]).map(([session,art])=>({session,event:'destroy',position:art.currentTime,duration:art.duration,capturing:!!art.__capture}));",
                );
                match timeout(Duration::from_secs(2), snapshot.join::<Vec<Value>>()).await {
                    Ok(Ok(events)) => {
                        for event in events {
                            if let Err(error) = svc.playback.event(&event, now()) {
                                eprintln!("[history] 退出保存失败：{error:#}");
                            }
                        }
                    }
                    Ok(Err(error)) => eprintln!("[history] 读取最终进度失败：{error}"),
                    Err(_) => eprintln!("[history] 读取最终进度超时，保存最近进度"),
                }
                if let Err(error) = svc.playback.finish() {
                    eprintln!("[history] 保存最近进度失败：{error:#}");
                }
                let size = window.inner_size().to_logical::<f64>(window.scale_factor());
                if let Err(error) = svc
                    .store
                    .save_settings(&json!({"window":{"width":size.width,"height":size.height}}))
                {
                    eprintln!("[window] 保存失败：{error:#}");
                }
                if let Err(error) = svc.catalogue.spider.stop().await {
                    eprintln!("[spider] 退出失败：{error:#}");
                }
                window.set_close_behavior(dioxus::desktop::WindowCloseBehaviour::WindowCloses);
                window.close();
            });
        }
    });
}

#[component]
pub(super) fn PlayerSurface(
    address: String,
    #[props(default)] live: bool,
    #[props(default)] entry: Value,
    #[props(default)] class: String,
    onsession: EventHandler<String>,
) -> Element {
    let mut ui = use_context::<Ui>();
    let session = use_hook(|| uuid::Uuid::new_v4().to_string());
    let id = format!("player-{session}");
    let cleanup = session.clone();
    use_drop(move || {
        let _ = document::eval(&format!(
            "{{const art=window.__crayonPlayers?.get({0});if(art){{art.__emit?.('destroy');art.destroy(false);window.__crayonPlayers.delete({0});}}}}",
            json!(cleanup)
        ));
    });
    let raw = address.clone();
    let session2 = session.clone();
    rsx! { div { id: "{id}", class: "{class}", onmounted: move |_| {
        let address = raw.clone(); let entry = entry.clone(); let session = session2.clone(); let id = id.clone();
        onsession.call(session.clone());
        spawn(async move {
            let svc = ui.backend();
            if ui.report(svc.playback.attach(&session, entry)).is_none() { return; }
            let proxy = match svc.playable(&address).await {
                Ok(value) => value,
                Err(error) => { ui.player_event.set(json!({"session":session,"event":"error","at":now(),"error":error.to_string()})); ui.notify(format!("{error:#}"), "danger"); return; }
            };
            let config = json!({"session":session,"id":id,"url":proxy,"hls":crate::media::is_hls(&address),"live":live,"accent":ui.settings.peek()["accent"]});
            let mut eval = document::eval(PLAYER_BRIDGE);
            if let Err(error) = eval.send(config) { ui.notify(format!("播放器初始化失败：{error}"), "danger"); return; }
            match eval.recv::<Value>().await {
                Ok(result) if result["ok"] != true => { ui.player_event.set(json!({"session":session,"event":"error","at":now(),"error":result["error"]})); ui.notify(s(&result, "error"), "danger"); },
                Err(error) => ui.notify(format!("播放器桥接失败：{error}"), "danger"), _ => {}
            }
        });
    } } }
}

// 只封装 WebView DOM 与第三方媒体组件；选线、续播、重试和持久化由 Rust 决定。
const PLAYER_BRIDGE: &str = r#"
const cfg=await dioxus.recv();
try {
  const el=document.getElementById(cfg.id);if(!el)throw new Error('找不到播放器容器');el.innerHTML='';
  let art;
  const emit=(event,extra={},instance=art)=>window.__crayonSend?.({session:cfg.session,event,position:instance?.currentTime||0,duration:Number.isFinite(instance?.duration)?instance.duration:0,capturing:!!instance?.__capture,...extra});
  const custom=(video,url,instance)=>{
    if(Hls.isSupported()){
      const hls=new Hls({maxBufferLength:30,manifestLoadingTimeOut:15000,abrEwmaDefaultEstimate:4000000,manifestLoadingMaxRetry:3,levelLoadingMaxRetry:4,fragLoadingMaxRetry:4});instance.hls=hls;
      hls.on(Hls.Events.MANIFEST_PARSED,(_,data)=>emit('levels',{levels:data.levels.map(v=>({height:v.height,bitrate:v.bitrate}))},instance));
      hls.on(Hls.Events.ERROR,(_,data)=>emit('hlsError',{fatal:data.fatal,errorType:data.type,error:data.details},instance));instance.on('destroy',()=>hls.destroy());hls.loadSource(url);hls.attachMedia(video);
    }else if(video.canPlayType('application/vnd.apple.mpegurl'))video.src=url;else emit('error',{error:'当前环境不支持 HLS 播放'},instance);
  };
  art=new Artplayer({container:el,url:cfg.url,isLive:cfg.live,...(cfg.hls?{type:'m3u8',customType:{m3u8:custom}}:{}),autoplay:true,volume:0.8,pip:true,fullscreen:true,fullscreenWeb:true,setting:true,hotkey:true,playbackRate:!cfg.live,aspectRatio:true,miniProgressBar:true,airplay:true,theme:cfg.accent||'#2563EB',lang:'zh-CN',moreVideoAttr:{crossOrigin:'anonymous'}});
  art.__emit=emit;window.__crayonPlayers.set(cfg.session,art);
  for(const event of ['loadedmetadata','durationchange','canplay','playing','timeupdate','pause','ended','error'])art.on('video:'+event,()=>emit(event,event==='error'?{error:art.video.error?.message||'视频加载失败'}:{}));
  dioxus.send({ok:true});
}catch(error){dioxus.send({ok:false,error:String(error.message||error)});}
"#;
