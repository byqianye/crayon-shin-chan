use crate::{
    model::{array, s, text},
    storage::Store,
};
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

pub const PREFER: &[&str] = &[
    "PotPlayer",
    "mpv",
    "VLC",
    "MPC-HC",
    "MPC-BE",
    "KMPlayer",
    "SMPlayer",
];
pub fn arguments(templates: &[Value], address: &str, title: &str) -> Result<Vec<String>> {
    let url = url::Url::parse(address.trim()).context("播放地址不是受支持的协议")?;
    if !["http", "https", "rtmp", "rtsp", "file"].contains(&url.scheme()) {
        bail!("播放地址不是受支持的协议");
    }
    let url_slot = Regex::new(r"(?i)\{url\}")?;
    let title_slot = Regex::new(r"(?i)\{title\}")?;
    let templates = if templates.is_empty() {
        vec![json!("{url}")]
    } else {
        templates.to_vec()
    };
    Ok(templates
        .iter()
        .map(|v| {
            title_slot
                .replace_all(
                    &url_slot.replace_all(&text(v), regex::NoExpand(address)),
                    regex::NoExpand(title),
                )
                .to_string()
        })
        .collect())
}
fn hidden(command: &mut tokio::process::Command) {
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    #[cfg(not(windows))]
    let _ = command;
}
async fn reg(args: &[&str]) -> String {
    let mut command = tokio::process::Command::new("reg.exe");
    command.args(args).kill_on_drop(true);
    hidden(&mut command);
    match tokio::time::timeout(Duration::from_secs(4), command.output()).await {
        Ok(Ok(output)) if output.status.success() => {
            let text = crate::http::decode(&output.stdout, "text/plain;charset=gbk");
            text.lines()
                .find_map(|line| {
                    line.split_once("REG_SZ")
                        .or_else(|| line.split_once("REG_EXPAND_SZ"))
                        .map(|v| v.1.trim().to_owned())
                })
                .unwrap_or_default()
        }
        _ => String::new(),
    }
}
fn executable(command: &str) -> String {
    let command = command.trim();
    let path = if let Some(command) = command.strip_prefix('"') {
        command.split('"').next().unwrap_or("")
    } else {
        command.split_whitespace().next().unwrap_or("")
    };
    if path.to_lowercase().ends_with(".exe") {
        path.into()
    } else {
        String::new()
    }
}
pub async fn detect(store: &Store) -> Result<Vec<Value>> {
    let settings = store.settings()?;
    let mut roots: Vec<_> = [
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
        "LOCALAPPDATA",
    ]
    .iter()
    .filter_map(std::env::var_os)
    .map(PathBuf::from)
    .collect();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join("Programs"));
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let on_path: Vec<_> = std::env::split_paths(&path).collect();
    let definitions: &[(&str, &[&str], &str)] = &[
        (
            "PotPlayer",
            &[
                "DAUM/PotPlayer/PotPlayerMini64.exe",
                "DAUM/PotPlayer/PotPlayerMini.exe",
                "PotPlayer/PotPlayerMini64.exe",
                "PotPlayer/PotPlayerMini.exe",
            ],
            "PotPlayerMini64.exe",
        ),
        (
            "mpv",
            &["mpv/mpv.exe", "mpv-player/mpv.exe", "mpv.net/mpvnet.exe"],
            "mpv.exe",
        ),
        ("VLC", &["VideoLAN/VLC/vlc.exe"], "vlc.exe"),
        (
            "MPC-HC",
            &[
                "MPC-HC/mpc-hc64.exe",
                "MPC-HC64/mpc-hc64.exe",
                "K-Lite Codec Pack/MPC-HC64/mpc-hc64.exe",
            ],
            "mpc-hc64.exe",
        ),
        ("MPC-BE", &["MPC-BE x64/mpc-be64.exe"], "mpc-be64.exe"),
        (
            "KMPlayer",
            &["The KMPlayer/KMPlayer64.exe", "KMPlayer/KMPlayer.exe"],
            "KMPlayer64.exe",
        ),
        ("SMPlayer", &["SMPlayer/smplayer.exe"], "smplayer.exe"),
    ];
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    for (name, relatives, exe) in definitions {
        let candidates: Vec<_> = roots
            .iter()
            .flat_map(|root| relatives.iter().map(move |relative| root.join(relative)))
            .chain(on_path.iter().map(|root| root.join(exe)))
            .collect();
        if let Some(path) = candidates.iter().find(|p| p.is_file()) {
            let path = path.to_string_lossy().into_owned();
            if seen.insert(path.to_lowercase()) {
                found.push(json!({"name":name,"path":path,"source":"scan"}));
            }
        }
    }
    let prefer = if array(&settings["externalPlayer"]["prefer"]).is_empty() {
        PREFER.iter().map(|v| json!(v)).collect()
    } else {
        array(&settings["externalPlayer"]["prefer"])
    };
    found.sort_by_key(|v| {
        prefer
            .iter()
            .position(|p| text(p) == s(v, "name"))
            .unwrap_or(prefer.len())
    });
    #[cfg(windows)]
    let ignored_program = Regex::new(
        r"(?i)^(mse?edge|chrome|chromium|firefox|brave|opera|quark|vivaldi|360|sogou|qqbrowser|notepad|code|explorer|photos|mspaint|acdsee|wmp11\.assocfile)",
    )?;
    #[cfg(windows)]
    for ext in [".mkv", ".mp4", ".ts", ".flv"] {
        let mut id = reg(&[
            "query",
            &format!(
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\{ext}\UserChoice"
            ),
            "/v",
            "ProgId",
        ])
        .await;
        if id.is_empty() {
            for hive in ["HKCU", "HKLM"] {
                id = reg(&["query", &format!(r"{hive}\Software\Classes\{ext}"), "/ve"]).await;
                if !id.is_empty() {
                    break;
                }
            }
        }
        if id.is_empty() || ignored_program.is_match(&id) {
            continue;
        }
        let mut command = String::new();
        for hive in ["HKCU", "HKLM"] {
            command = reg(&[
                "query",
                &format!(r"{hive}\Software\Classes\{id}\shell\open\command"),
                "/ve",
            ])
            .await;
            if !command.is_empty() {
                break;
            }
        }
        let path = executable(&command);
        if !path.is_empty() && Path::new(&path).is_file() && seen.insert(path.to_lowercase()) {
            found.push(json!({"name":id.rsplit_once('.').map(|v|v.0).unwrap_or(&id),"path":path,"source":"registry"}));
        }
    }
    let manual = s(&settings["externalPlayer"], "command");
    if !manual.is_empty() && Path::new(&manual).is_file() && seen.insert(manual.to_lowercase()) {
        found.insert(
            0,
            json!({"name":"设置里指定的","path":manual,"source":"manual"}),
        );
    }
    Ok(found)
}
pub async fn open(store: &Store, address: &str, title: &str) -> Result<Value> {
    let settings = store.settings()?;
    let manual = s(&settings["externalPlayer"], "command");
    let command = if manual.is_empty() {
        detect(store)
            .await?
            .first()
            .map(|v| s(v, "path"))
            .context("没有找到可用的外部播放器，请在「设置」里手动指定播放器路径")?
    } else {
        if !Path::new(&manual).is_file() {
            bail!("设置里指定的播放器不存在：{manual}");
        }
        manual
    };
    let args = arguments(&array(&settings["externalPlayer"]["args"]), address, title)?;
    let mut child = tokio::process::Command::new(&command);
    child
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    child.creation_flags(0x00000008);
    child.spawn().context("无法启动外部播放器")?;
    Ok(json!({"player":command,"args":args}))
}
pub fn open_path(path: &Path) -> Result<()> {
    opener::open(path).context("无法打开目录")
}
