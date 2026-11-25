use crate::{
    http::Net,
    model::{number, s},
    storage::Store,
};
use anyhow::{Context, Result, bail};
use base64::Engine;
use serde_json::{Value, json};
use std::time::Instant;
use tokio_util::sync::CancellationToken;

const TEXT: &str = "你是一个中文影视剧剧情助手。用户会给你一部剧（或番剧、综艺、动漫）的资料，以及他当前正在看的集数。\n请针对「这一集」输出内容，不要复述整部剧的简介。\n\n输出格式（纯文本，不要用 Markdown 标题符号）：\n第一段：用 2 到 4 句话说清这一集讲了什么。\n然后另起一行写「本集看点」，下面用「- 」列 3 到 5 条。\n\n硬性要求：\n1. 如果资料不足以确定这一集的具体情节，就在第一段里如实说明「仅凭现有资料无法确定这一集的具体情节」，然后只写你能推断的内容，绝对不要编造具体的人物、台词或事件。\n2. 不要写「好的」「以下是」这类客套话。\n3. 不要重复用户给你的原始资料。\n4. 全文控制在 400 字以内，用简体中文。";
const VISUAL: &str = "你是一个中文影视剧剧情解说。用户会给你一部剧集的资料，以及按时间顺序从这一集里抽出来的画面——每张图前面标着它在片中的时间点。\n你要根据这些画面写出这一集实际发生了什么，而不是根据剧名去猜。\n\n输出格式（纯文本，不要用 Markdown 标题符号）：\n第一段：用 3 到 6 句话说清这一集的剧情走向，按时间顺序。\n然后另起一行写「剧情节点」，下面用「- [时间] 内容」的形式列 4 到 8 条关键节点。把相邻的画面合并成一条，不要一帧一条。\n然后另起一行写「本集看点」，下面用「- 」列 2 到 4 条。\n\n硬性要求：\n1. 这些画面是每 20 秒抽一帧得到的，而且没有声音。画面上出现字幕或文字时可以照实引用；不要编造对白、人物姓名，或画面上根本看不到的情节；只描述确实从画面里看到的东西，拿不准的地方明说看不出来。\n2. 如果画面几乎全是片头、黑屏、静态图或者内容高度重复，就在第一段里直接说明「这一集抽到的画面信息量很少」，不要硬编。\n3. 不要写「好的」「以下是」这类客套话，不要重复用户给你的资料。\n4. 全文控制在 600 字以内，用简体中文。";

pub fn build_user(input: &Value) -> String {
    let mut meta = Vec::new();
    for (key, label) in [
        ("vodName", "片名"),
        ("typeName", "类型"),
        ("year", "年份"),
        ("area", "地区"),
        ("director", "导演"),
        ("actor", "演员"),
        ("score", "评分"),
        ("remarks", "备注"),
    ] {
        let value = s(input, key);
        if !value.is_empty() {
            meta.push(format!("{label}：{value}"));
        }
    }
    let line = s(input, "lineName");
    let line = if line.is_empty() {
        String::new()
    } else {
        format!("，线路「{line}」")
    };
    let total = number(&input["epTotal"], 0.0) as usize;
    let total = if total > 0 {
        total.to_string()
    } else {
        "?".into()
    };
    let intro = s(input, "content").chars().take(1200).collect::<String>();
    format!(
        "【影片资料】\n{}\n当前正在看：{}（第 {} 集 / 共 {total} 集{line}）{}",
        meta.join("\n"),
        s(input, "epName"),
        number(&input["epIndex"], 0.0) as usize + 1,
        if intro.is_empty() {
            String::new()
        } else {
            format!("\n【整部剧的简介】\n{intro}")
        }
    )
}
pub fn plan_frames(duration: f64) -> Vec<f64> {
    if !duration.is_finite() || duration <= 1.0 {
        return Vec::new();
    }
    let span = duration - 3.0;
    if span <= 0.0 {
        return vec![2.0_f64.min(duration)];
    }
    let step = 20.0_f64.max(span / 120.0);
    let mut targets = Vec::new();
    let mut at = 2.0;
    while at < duration - 1.0 && targets.len() < 120 {
        targets.push(at);
        at += step;
    }
    targets
}
pub fn visual_blocks(input: &Value, frames: &[Value]) -> Result<Vec<Value>> {
    let mut blocks = vec![
        json!({"type":"text","text":build_user(input)}),
        json!({"type":"text","text":format!("【按时间顺序抽取的画面】共 {} 帧，每帧前标着它在片中的时间点。",frames.len())}),
    ];
    for frame in frames.iter().take(140) {
        let data = s(frame, "b64");
        if data.len() > 2_000_000 {
            bail!("单帧图片超过 2MB 上限");
        }
        base64::engine::general_purpose::STANDARD
            .decode(&data)
            .context("抽帧数据不是有效 Base64")?;
        let seconds = number(&frame["t"], 0.0).max(0.0).round() as usize;
        blocks.push(json!({"type":"text","text":format!("[{:02}:{:02}]",seconds/60,seconds%60)}));
        let address = ["data:", "image/jpeg;base64,", &data].concat();
        blocks.push(json!({"type":"image_url","image_url":{"url":address,"detail":"low"}}));
    }
    Ok(blocks)
}
pub async fn summarize(
    store: &Store,
    net: &Net,
    input: &Value,
    frames: Vec<Value>,
    cancel: &CancellationToken,
) -> Result<Value> {
    let settings = store.settings()?;
    let key = s(&settings["ai"], "apiKey");
    if key.is_empty() {
        bail!("还没有配置 DeepSeek API Key。去「设置 → AI 总结」里填一下就能用了。");
    }
    let model = s(&settings["ai"], "model");
    let model = if model.is_empty() {
        "deepseek-flash".into()
    } else {
        model
    };
    let base = s(&settings["ai"], "baseUrl");
    let base = if base.is_empty() {
        "https://api.deepseek.com"
    } else {
        &base
    }
    .trim_end_matches('/');
    let frames: Vec<_> = frames
        .into_iter()
        .filter(|v| s(v, "b64").len() > 64)
        .take(140)
        .collect();
    let visual = !frames.is_empty();
    let user = if visual {
        json!(visual_blocks(input, &frames)?)
    } else {
        json!(build_user(input))
    };
    let body = json!({"model":model,"messages":[{"role":"system","content":if visual{VISUAL}else{TEXT}},{"role":"user","content":user}],"thinking":{"type":"disabled"},"max_tokens":if visual{1600}else{900},"stream":false});
    let started = Instant::now();
    let response = net
        .request(crate::http::Request {
            url: &format!("{base}/chat/completions"),
            method: reqwest::Method::POST,
            headers: &json!({"Content-Type":"application/json","Authorization":format!("Bearer {key}")}),
            body: Some(serde_json::to_vec(&body)?),
            timeout: if visual {
                (120.0 + frames.len() as f64).min(300.0)
            } else {
                90.0
            },
            allow_http_error: true,
        }, cancel)
        .await
        .context("连接 AI 服务失败，请检查网络或代理")?;
    let payload: Value = serde_json::from_str(&response.text()).unwrap_or(Value::Null);
    if !(200..300).contains(&response.status) {
        let detail = s(&payload["error"], "message").replace(&key, "[隐藏]");
        let reason = match response.status {
            401 => "API Key 不正确，请到设置检查（401）".into(),
            402 => "AI 账户余额不足（402）".into(),
            413 => "这次传的画面太多了，接口拒收（413）".into(),
            429 => "请求太频繁被限流了（429），等一会儿再试".into(),
            status => format!("AI 返回 HTTP {status}：{detail}"),
        };
        bail!("{reason}");
    }
    let text = s(&payload["choices"][0]["message"], "content");
    if text.is_empty() {
        bail!("AI 这次没返回任何内容，再点一次试试。");
    }
    let usage = &payload["usage"];
    let usage = if usage.is_object() {
        json!({"prompt":number(&usage["prompt_tokens"],0.0),"completion":number(&usage["completion_tokens"],0.0),"total":number(&usage["total_tokens"],0.0)})
    } else {
        Value::Null
    };
    Ok(json!({"text":text,"model":model,"ms":started.elapsed().as_millis() as u64,"usage":usage}))
}
