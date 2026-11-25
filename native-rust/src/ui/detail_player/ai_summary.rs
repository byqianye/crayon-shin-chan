use super::*;
use crate::ai;

#[derive(Clone, Copy)]
struct SummaryState {
    phase: Signal<&'static str>,
    visual: Signal<bool>,
    result: Signal<Value>,
    error: Signal<String>,
    progress: Signal<Value>,
    frames: Signal<usize>,
    cancel: Signal<CancellationToken>,
}

fn run(mut state: SummaryState, ui: Ui, input: Value, visual: bool) {
    if *state.phase.peek() != "idle" {
        return;
    }
    let cancel = CancellationToken::new();
    state.cancel.set(cancel.clone());
    state.visual.set(visual);
    state.result.set(Value::Null);
    state.error.set(String::new());
    state.frames.set(0);
    state
        .phase
        .set(if visual { "capture" } else { "summarize" });
    spawn(async move {
        let outcome = async {
            let frames = if visual {
                capture(state, &cancel).await?
            } else {
                Vec::new()
            };
            if cancel.is_cancelled() {
                anyhow::bail!("已停止");
            }
            state.frames.set(frames.len());
            state.phase.set("summarize");
            let svc = ui.backend();
            ai::summarize(&svc.store, &svc.sources.net, &input, frames, &cancel).await
        }
        .await;
        match outcome {
            Ok(value) => state.result.set(value),
            Err(error) if !cancel.is_cancelled() => state.error.set(format!("{error:#}")),
            Err(_) => {}
        }
        state.progress.set(Value::Null);
        state.phase.set("idle");
    });
}

struct Restore(String);
impl Drop for Restore {
    fn drop(&mut self) {
        let _ = document::eval(&format!(
            "window.__crayonPlayers?.get({})?.__restoreCapture?.();",
            json!(self.0)
        ));
    }
}

async fn capture(mut state: SummaryState, cancel: &CancellationToken) -> Result<Vec<Value>> {
    let mut bridge = document::eval(CAPTURE);
    let metadata = bridge
        .recv::<Value>()
        .await
        .map_err(|e| anyhow::anyhow!("播放器还没准备好：{e}"))?;
    if metadata["ok"] != true {
        anyhow::bail!("{}", s(&metadata, "error"));
    }
    let _restore = Restore(s(&metadata, "session"));
    let targets = ai::plan_frames(number(&metadata["duration"], 0.0));
    if targets.is_empty() {
        anyhow::bail!("这个视频拿不到时长，没法扫。等它加载出来再试。");
    }
    let mut frames = Vec::new();
    let mut skipped = 0;
    for (i, at) in targets.iter().enumerate() {
        if cancel.is_cancelled() {
            break;
        }
        state
            .progress
            .set(json!({"done":i,"total":targets.len(),"at":at}));
        bridge
            .send(json!({"at":at}))
            .map_err(|e| anyhow::anyhow!("抽帧连接中断：{e}"))?;
        let result = tokio::select! {biased;_=cancel.cancelled()=>break,result=bridge.recv::<Value>()=>result.map_err(|e|anyhow::anyhow!("抽帧失败：{e}"))?};
        if result["ok"] == true {
            frames.push(json!({"t":at,"b64":result["b64"]}));
        } else {
            skipped += 1;
        }
    }
    let _ = bridge.send(json!({"restore":true}));
    if !cancel.is_cancelled() && frames.is_empty() {
        anyhow::bail!("一个画面都没抓到（{skipped} 个时间点都失败了），可能这个源不让跳转。");
    }
    Ok(frames)
}

#[component]
pub(in crate::ui) fn AiSummary(input: Value) -> Element {
    let ui = use_context::<Ui>();
    let mut state = SummaryState {
        phase: use_signal(|| "idle"),
        visual: use_signal(|| true),
        result: use_signal(|| Value::Null),
        error: use_signal(String::new),
        progress: use_signal(|| Value::Null),
        frames: use_signal(|| 0),
        cancel: use_signal(CancellationToken::new),
    };
    use_drop(move || state.cancel.peek().cancel());
    let text_input = input.clone();
    let visual_input = input.clone();
    let again = input.clone();
    let busy = *state.phase.read() != "idle";
    let result = state.result.read().clone();
    let has_key = !s(&ui.settings.read()["ai"], "apiKey").is_empty();
    let progress = state.progress.read().clone();
    let pct = if number(&progress["total"], 0.0) > 0.0 {
        (number(&progress["done"], 0.0) / number(&progress["total"], 1.0) * 100.0).round()
    } else {
        0.0
    };
    let copied = s(&result, "text");
    rsx! {div{class:"ai-block",
        div{class:"ai-head",div{class:"ai-head-text",div{class:"ai-title",Icon{name:"sparkle",size:14}"AI 剧情总结"}div{class:"ai-sub text-3 fs-12",{s(&input,"epName")}" · 让 AI 看一遍这一集，点了才开始"}}
            div{class:"hstack",
                if !result.is_null(){
                    button{class:"btn btn-sm",onclick:move |_|{let value=copied.clone();spawn(async move{copy(ui,&value).await;});},Icon{name:"link",size:12}"复制"}
                    button{class:"btn btn-sm",disabled:busy,onclick:move |_|run(state,ui,again.clone(),true),Icon{name:"refresh",size:12}"重新看一遍"}
                    button{class:"btn btn-sm",onclick:move |_|{state.cancel.peek().cancel();state.result.set(Value::Null);state.error.set(String::new());},Icon{name:"close",size:12}"收起"}
                }else{
                    button{class:"btn btn-sm",disabled:busy||!has_key,onclick:move |_|run(state,ui,text_input.clone(),false),"只看资料"}
                    button{class:"btn btn-primary btn-sm",disabled:busy||!has_key,onclick:move |_|run(state,ui,visual_input.clone(),true),if busy{span{class:"spin"}}else{Icon{name:"sparkle",size:13}}match *state.phase.read(){"capture"=>"正在抽帧…","summarize"=>"正在看片…",_=>"看一遍再总结"}}
                }
            }
        }
        if !has_key{div{class:"ai-body ai-hint","还没有配置 DeepSeek API Key，" a{href:"#",onclick:move |e|{e.prevent_default();ui.go(Page::Settings);},"去「设置 → AI 总结」"}"填一下就能用了。"}}
        if result.is_null()&&!busy&&has_key{div{class:"ai-body ai-hint","点「看一遍再总结」会先把这一集快速过一遍，每 20 秒抓一张画面（最多 120 张），再交给配置的 AI 服务。" br{}"抓画面会把整集缓冲一遍；画面没有声音，带硬字幕的片可以读字幕。"}}
        if *state.phase.read()=="capture"{div{class:"ai-body",div{class:"ai-hint",span{class:"spin"}"正在抽帧 "{s(&progress,"done")}" / "{s(&progress,"total")}" · "{clock(number(&progress["at"],0.0))}" · {pct}%"}div{class:"ai-bar",div{class:"ai-bar-fill",style:"width:{pct}%"}}div{class:"ai-foot text-3 fs-12","抽帧是把整集拉一遍" button{class:"btn btn-sm",onclick:move |_|state.cancel.peek().cancel(),"停止"}}}}
        if *state.phase.read()=="summarize"&&result.is_null(){div{class:"ai-body ai-hint",span{class:"spin"}if *state.visual.read(){"正在让 AI 看这 {state.frames} 张画面并写剧情…"}else{"正在让 AI 写这一集的剧情…"}button{class:"btn btn-sm",onclick:move |_|state.cancel.peek().cancel(),"停止"}}}
        if !state.error.read().is_empty(){div{class:"ai-body",div{class:"banner banner-danger","{state.error}"}}}
        if !result.is_null(){div{class:"ai-body ai-text",{s(&result,"text")}}div{class:"ai-foot text-3 fs-12",{s(&result,"model")}" · 耗时 "{format!("{:.1}s",number(&result["ms"],0.0)/1000.0)}if result["usage"].is_object(){" · "{s(&result["usage"],"total")}"tokens"}if *state.visual.read(){" · 看了 {state.frames} 张画面"}else{" · 只看资料"}" · " span{class:"ai-warn","AI 生成，可能不准，仅供参考"}}}
    }}
}

// 只操作视频、canvas 和 seek 事件；采样时间、进度、取消和 AI 请求由 Rust 控制。
const CAPTURE: &str = r#"
const pair=Array.from(window.__crayonPlayers||[]).find(([,art])=>art.video&&!art.option.isLive);
if(!pair){dioxus.send({ok:false,error:'播放器还没准备好。等画面出来再点，或者先点「只看资料」。'});return;}
const [session,art]=pair,video=art.video;
art.__capture={time:video.currentTime,paused:video.paused};art.__emit?.('captureStart');video.pause();
const canvas=document.createElement('canvas');
let restoring=false,restoreTask;
const restore=()=>restoreTask??=(async()=>{
  if(!art.__capture)return;restoring=true;const saved=art.__capture;
  try{await new Promise(resolve=>{const done=()=>{clearTimeout(timer);video.removeEventListener('seeked',done);resolve();};const timer=setTimeout(done,1500);video.addEventListener('seeked',done);try{video.currentTime=saved.time;}catch{done();}});}
  finally{delete art.__capture;delete art.__restoreCapture;art.__emit?.('captureEnd');if(!saved.paused)video.play().catch(()=>{});}
})();
art.__restoreCapture=restore;
dioxus.send({ok:true,session,duration:Number.isFinite(video.duration)?video.duration:0});
try{
  while(true){
    const cmd=await dioxus.recv();if(cmd.restore||restoring)break;
    try{
      await new Promise((resolve,reject)=>{
        if(Math.abs(video.currentTime-cmd.at)<0.06&&video.readyState>=2){resolve();return;}
        const cleanup=()=>{clearTimeout(timer);video.removeEventListener('seeked',seeked);};
        const seeked=()=>{cleanup();resolve();};const timer=setTimeout(()=>{cleanup();reject(new Error('跳转超时'));},15000);
        video.addEventListener('seeked',seeked);try{video.currentTime=cmd.at;}catch(error){cleanup();reject(error);}
      });
      await new Promise(resolve=>{if(video.requestVideoFrameCallback){const timer=setTimeout(resolve,1000);video.requestVideoFrameCallback(()=>{clearTimeout(timer);resolve();});}else requestAnimationFrame(()=>requestAnimationFrame(resolve));});
      if(restoring)break;
      if(!video.videoWidth||!video.videoHeight)throw new Error('还没拿到画面尺寸');
      canvas.width=Math.max(64,Math.min(512,video.videoWidth));canvas.height=Math.max(1,Math.round(canvas.width*video.videoHeight/video.videoWidth));
      const ctx=canvas.getContext('2d');if(!ctx)throw new Error('拿不到 canvas 上下文');ctx.drawImage(video,0,0,canvas.width,canvas.height);
      const image=canvas.toDataURL('image/jpeg',0.7);dioxus.send({ok:true,b64:image.slice(image.indexOf(',')+1)});
    }catch(error){dioxus.send({ok:false,error:String(error.message||error)});}
  }
}finally{await restore();}
"#;
