use regex::Regex;
use serde_json::{Value, json};
use std::sync::LazyLock;

pub const SAMPLE_LIMIT: usize = 4096;

pub fn kind_from_url(address: &str) -> &'static str {
    let Ok(url) = url::Url::parse(address) else {
        return "unknown";
    };
    let path = url.path().to_ascii_lowercase();
    match path.rsplit('.').next().unwrap_or("") {
        "m3u8" => "hls",
        "mpd" => "dash",
        "flv" => "flv",
        "ts" | "m2ts" | "mts" => "ts",
        "mp4" | "m4v" | "mov" => "mp4",
        "webm" => "webm",
        "mkv" => "mkv",
        "mp3" | "m4a" | "aac" | "flac" | "ogg" | "opus" | "wav" => "audio",
        "html" | "htm" => "html",
        _ => "unknown",
    }
}

pub fn normalized_mime(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .chars()
        .take(128)
        .collect::<String>()
        .to_ascii_lowercase()
}

fn kind_from_mime(mime: &str) -> &'static str {
    match mime {
        "application/vnd.apple.mpegurl"
        | "application/x-mpegurl"
        | "audio/mpegurl"
        | "audio/x-mpegurl" => "hls",
        "application/dash+xml" => "dash",
        "video/x-flv" | "video/flv" => "flv",
        "video/mp2t" => "ts",
        "video/mp4" | "application/mp4" | "video/quicktime" => "mp4",
        "video/webm" => "webm",
        "video/x-matroska" | "application/x-matroska" => "mkv",
        "text/html" | "application/xhtml+xml" => "html",
        _ if mime.starts_with("audio/") => "audio",
        _ => "unknown",
    }
}

fn signature(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_start_matches(['\u{feff}', ' ', '\r', '\n', '\t']);
    let lower = text.to_ascii_lowercase();
    let mut html = lower.as_str();
    if html.starts_with("<?xml")
        && let Some(end) = html.find("?>")
    {
        html = html[end + 2..].trim_start();
    }
    while html.starts_with("<!--") {
        let Some(end) = html.find("-->") else { break };
        html = html[end + 3..].trim_start();
    }
    if [
        "<!doctype html",
        "<html",
        "<head",
        "<body",
        "<script",
        "<title",
    ]
    .iter()
    .any(|v| html.starts_with(v))
    {
        return Some(("html", "HTML markup"));
    }
    if text.lines().next() == Some("#EXTM3U") {
        return Some(("hls", "#EXTM3U"));
    }
    static MPD: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?s)^(?:<\?xml[^>]*>\s*)?(?:<!--.*?-->\s*)*<(?:[A-Za-z_][\w.-]*:)?MPD(?:\s|/?>)",
        )
        .expect("constant regex")
    });
    if MPD.is_match(text) {
        return Some(("dash", "MPD root element"));
    }
    if bytes.len() >= 9
        && &bytes[..3] == b"FLV"
        && bytes[3] == 1
        && u32::from_be_bytes(bytes[5..9].try_into().expect("four bytes")) >= 9
    {
        return Some(("flv", "FLV header"));
    }
    if bytes.len() >= 12
        && &bytes[4..8] == b"ftyp"
        && u32::from_be_bytes(bytes[..4].try_into().expect("four bytes")) >= 12
        && !matches!(
            &bytes[8..12],
            b"avif" | b"avis" | b"heic" | b"heix" | b"mif1" | b"msf1"
        )
    {
        return Some(if &bytes[8..12] == b"M4A " {
            ("audio", "ISO BMFF M4A brand")
        } else {
            ("mp4", "ISO BMFF ftyp box")
        });
    }
    if bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        for offset in 4..bytes.len().saturating_sub(3) {
            if bytes.get(offset..offset + 2) != Some(&[0x42, 0x82]) {
                continue;
            }
            let length = bytes[offset + 2];
            if length & 0x80 == 0 {
                continue;
            }
            match bytes.get(offset + 3..offset + 3 + usize::from(length & 0x7f)) {
                Some(b"webm") => return Some(("webm", "EBML WebM DocType")),
                Some(b"matroska") => return Some(("mkv", "EBML Matroska DocType")),
                _ => {}
            }
        }
    }
    for stride in [188, 192, 204] {
        if (0..stride.min(bytes.len().saturating_sub(stride * 2)))
            .any(|i| bytes[i] == 0x47 && bytes[i + stride] == 0x47 && bytes[i + stride * 2] == 0x47)
        {
            return Some(("ts", "MPEG-TS repeated sync bytes"));
        }
    }
    if bytes.starts_with(b"ID3")
        || bytes.starts_with(b"fLaC")
        || bytes.starts_with(b"OggS")
        || (bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WAVE")
        || (bytes.len() >= 7 && bytes[0] == 0xff && bytes[1] & 0xf6 == 0xf0)
        || (bytes.len() >= 4
            && bytes[0] == 0xff
            && bytes[1] & 0xe0 == 0xe0
            && bytes[1] & 0x18 != 0x08
            && bytes[1] & 0x06 != 0
            && bytes[2] & 0xf0 != 0
            && bytes[2] & 0xf0 != 0xf0
            && bytes[2] & 0x0c != 0x0c)
    {
        return Some(("audio", "Audio container header"));
    }
    None
}

pub fn identify(address: &str, content_type: &str, sample: &[u8]) -> Value {
    let sample = &sample[..sample.len().min(SAMPLE_LIMIT)];
    let url_kind = kind_from_url(address);
    let mime = normalized_mime(content_type);
    let mime_kind = kind_from_mime(&mime);
    let signature = signature(sample);
    let mut evidence = Vec::new();
    if url_kind != "unknown" {
        evidence.push(json!({"source":"url","value":url_kind}));
    }
    if mime_kind != "unknown" {
        evidence.push(json!({"source":"mime","value":mime}));
    }
    if let Some((_, value)) = signature {
        evidence.push(json!({"source":"signature","value":value}));
    }
    let (kind, confidence) = if let Some((kind, _)) = signature {
        (kind, "signature")
    } else if mime_kind != "unknown" {
        (mime_kind, "mime")
    } else if url_kind != "unknown" {
        (url_kind, "url")
    } else {
        ("unknown", "none")
    };
    let container = match kind {
        "hls" => "HLS",
        "dash" => "MPEG-DASH",
        "flv" => "FLV",
        "ts" => "MPEG-TS",
        "mp4" => "MP4 / ISO BMFF",
        "webm" => "WebM",
        "mkv" => "Matroska",
        "audio" => "音频",
        "html" => "HTML",
        _ => "未知",
    };
    let mut report = json!({"kind":kind,"container":container,"evidence":evidence,"confidence":confidence,"browserPlayable":null,"decodeVerified":false});
    if kind == "hls" && confidence == "signature" {
        for (field, pattern) in [
            ("height", r"(?i)RESOLUTION=\d+x(\d+)"),
            ("bandwidth", r"(?i)BANDWIDTH=(\d+)"),
        ] {
            if let Some(max) = Regex::new(pattern)
                .expect("constant regex")
                .captures_iter(&String::from_utf8_lossy(sample))
                .filter_map(|v| v[1].parse::<u64>().ok())
                .max()
            {
                report[field] = json!(max);
            }
        }
    }
    report
}
