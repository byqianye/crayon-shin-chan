use crate::model::{compare_cn, number, s};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::LazyLock,
};

static ALIAS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[（(]([^（()）]*)[)）]\s*$").expect("constant regex"));
static SEASON_MARK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"第\s*[0-9一二三四五六七八九十百千]+\s*[季部集篇辑]|番外|特别篇|前传|后传|续集")
        .expect("constant regex")
});
static NOISE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(高清版?|超清|蓝光|完整版?|全集|全\d+集|国语版?|粤语版?|普通话版?|中字|双语|hd|bd|4k|1080p|720p|2160p)").expect("constant regex")
});
static COMMENTARY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new("解说|速看|盘点|混剪|拆解|影评|剧情介绍|一口气看完|分钟看完")
        .expect("constant regex")
});
static YEARS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"^((?:19|20)\d{2})(?:$|[-/.])",
        r"[（(]\s*((?:19|20)\d{2})\s*[)）]",
        r"(?:^|[\s._-])((?:19|20)\d{2})$",
    ]
    .map(|v| Regex::new(v).expect("constant regex"))
    .into()
});
static SEASONS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"第\s*([0-9一二两三四五六七八九十]+)\s*[季部]",
        r"(?i)(?:season|series)\s*(\d{1,2})\b",
        r"(?i)\bS(\d{1,2})\b",
    ]
    .map(|v| Regex::new(v).expect("constant regex"))
    .into()
});

pub fn normalize_name(input: &str) -> String {
    let mut name = input.trim();
    while let Some(capture) = ALIAS.captures(name) {
        if SEASON_MARK.is_match(&capture[1]) {
            break;
        }
        let head = name[..capture.get(0).expect("alias").start()].trim();
        if head.is_empty() {
            break;
        }
        name = head;
    }
    let name: String = name
        .chars()
        .map(|v| {
            if ('！'..='～').contains(&v) {
                char::from_u32(v as u32 - 0xfee0).expect("ASCII")
            } else {
                v
            }
        })
        .collect();
    NOISE
        .replace_all(&name, "")
        .chars()
        .filter(|v| {
            !v.is_whitespace()
                && !"·・:：;；-—_~～.,，。!！?？'\"“”‘’`^&*+=<>《》()（）[]【】{}｛｝/\\|@#$%"
                    .contains(*v)
        })
        .collect::<String>()
        .to_lowercase()
}
pub fn commentary(name: &str, kind: &str) -> bool {
    COMMENTARY.is_match(name) || COMMENTARY.is_match(kind)
}
pub fn year(values: &[&str]) -> String {
    for value in values {
        for pattern in YEARS.iter() {
            if let Some(capture) = pattern.captures(value.trim()) {
                return capture[1].to_owned();
            }
        }
    }
    String::new()
}
fn cn_number(input: &str) -> String {
    if let Ok(n) = input.parse::<u32>() {
        return n.to_string();
    }
    let digit = |v: char| -> Option<u32> {
        match v {
            '零' => Some(0),
            '一' => Some(1),
            '二' | '两' => Some(2),
            '三' => Some(3),
            '四' => Some(4),
            '五' => Some(5),
            '六' => Some(6),
            '七' => Some(7),
            '八' => Some(8),
            '九' => Some(9),
            _ => None,
        }
    };
    if let Some((high, low)) = input.split_once('十') {
        let high = if high.is_empty() {
            Some(1)
        } else {
            high.chars().next().and_then(digit)
        };
        let low = if low.is_empty() {
            Some(0)
        } else {
            low.chars().next().and_then(digit)
        };
        if let (Some(high), Some(low)) = (high, low) {
            return (high * 10 + low).to_string();
        }
    }
    if input.chars().count() == 1
        && let Some(value) = input.chars().next().and_then(digit)
    {
        return value.to_string();
    }
    input.to_owned()
}
pub fn season(input: &str) -> String {
    for pattern in SEASONS.iter() {
        if let Some(capture) = pattern.captures(input) {
            return cn_number(&capture[1]);
        }
    }
    String::new()
}
pub fn bucket(kind: &str) -> &'static str {
    static TYPES: LazyLock<Vec<(&str, Regex)>> = LazyLock::new(|| {
        [("anime","动漫|动画|番剧|卡通"),("variety","综艺|真人秀|脱口秀"),("tv","电视剧|连续剧|剧集|短剧|美剧|韩剧|日剧|港剧|台剧|国产剧|海外剧|香港剧|台湾剧|大陆剧|欧美剧"),("movie","电影|影片|片")].into_iter().map(|(name,pattern)|(name,Regex::new(pattern).expect("constant regex"))).collect()
    });
    TYPES
        .iter()
        .find(|(_, pattern)| pattern.is_match(kind))
        .map(|(name, _)| *name)
        .unwrap_or("other")
}
pub fn score(vod: &Value) -> u8 {
    let has_text = |value: &Value| match value {
        Value::String(text) => !text.trim().is_empty(),
        Value::Number(_) | Value::Bool(_) => true,
        _ => false,
    };
    u8::from(has_text(&vod["vod_pic"])) * 4
        + u8::from(has_text(&vod["vod_remarks"])) * 2
        + u8::from(number(&vod["vod_score"], 0.0) > 0.0)
}
#[derive(Clone)]
struct Identity {
    name: String,
    units: Vec<u16>,
    year: String,
    season: String,
    bucket: &'static str,
}
impl Identity {
    fn new(vod: &Value) -> Self {
        let raw = s(vod, "vod_name");
        let name = normalize_name(&raw);
        Self {
            units: name.encode_utf16().collect(),
            name,
            year: year(&[&s(vod, "vod_year"), &raw]),
            season: season(&raw),
            bucket: bucket(&s(vod, "type_name")),
        }
    }
    fn key(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.bucket, self.name, self.season, self.year
        )
    }
}
pub fn key(vod: &Value) -> String {
    Identity::new(vod).key()
}
fn can_merge(a: &Identity, b: &Identity) -> bool {
    if a.bucket != "other" && b.bucket != "other" && a.bucket != b.bucket {
        return false;
    }
    if a.season != b.season {
        return false;
    }
    if !a.year.is_empty() && !b.year.is_empty() && a.year != b.year {
        return false;
    }
    if a.year.is_empty() || b.year.is_empty() {
        return a.name == b.name;
    }
    if a.name == b.name {
        return true;
    }
    let longest = a.units.len().max(b.units.len());
    if longest == 0 {
        return false;
    }
    let allowed = ((longest as f64) * 0.08 + 1e-9).floor() as usize;
    if a.units.len().abs_diff(b.units.len()) > allowed {
        return false;
    }
    let mut previous: Vec<usize> = (0..=b.units.len()).collect();
    let mut current = vec![0; b.units.len() + 1];
    for (i, left) in a.units.iter().enumerate() {
        current[0] = i + 1;
        for (j, right) in b.units.iter().enumerate() {
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + usize::from(left != right));
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.units.len()] <= allowed
}
fn match_rank(name: &str, keyword: &str) -> u8 {
    if keyword.is_empty() {
        return 3;
    }
    let name = normalize_name(name);
    if name == keyword {
        0
    } else if name.starts_with(keyword) {
        1
    } else if name.contains(keyword) {
        2
    } else {
        3
    }
}

pub fn merge_results(
    results: &[Value],
    keyword: &str,
    filter: bool,
    previous_order: Option<&HashMap<String, usize>>,
) -> Value {
    let mut exact: Vec<(Identity, Vec<&Value>)> = Vec::new();
    let mut exact_indices = HashMap::new();
    let mut hidden = 0;
    for result in results {
        let items = match &result["list"] {
            Value::Array(items) => items.as_slice(),
            item @ Value::Object(_) => std::slice::from_ref(item),
            _ => &[],
        };
        for vod in items {
            let name = s(vod, "vod_name");
            if name.is_empty() {
                continue;
            }
            if filter && commentary(&name, &s(vod, "type_name")) {
                hidden += 1;
                continue;
            }
            let identity = Identity::new(vod);
            let key = identity.key();
            let index = if let Some(index) = exact_indices.get(&key) {
                *index
            } else {
                let index = exact.len();
                exact_indices.insert(key, index);
                exact.push((identity, Vec::new()));
                index
            };
            exact[index].1.push(vod);
        }
    }
    let mut groups: Vec<(Identity, Vec<&Value>)> = Vec::new();
    let mut by_name: HashMap<(String, String), Vec<usize>> = HashMap::new();
    let mut by_year: HashMap<(String, String, usize), Vec<usize>> = HashMap::new();
    for (identity, items) in exact {
        let name_key = (identity.season.clone(), identity.name.clone());
        let mut candidates = by_name.get(&name_key).cloned().unwrap_or_default();
        let length = identity.units.len();
        if !identity.year.is_empty() {
            for size in ((length as f64 * 0.92 - 1e-9).ceil() as usize)
                ..=((length as f64 / 0.92 + 1e-9).floor() as usize)
            {
                if size.max(length) < 13 {
                    continue;
                }
                if let Some(related) =
                    by_year.get(&(identity.season.clone(), identity.year.clone(), size))
                {
                    for index in related {
                        if groups[*index].0.name != identity.name {
                            candidates.push(*index);
                        }
                    }
                }
            }
        }
        let target = candidates
            .into_iter()
            .filter(|index| can_merge(&groups[*index].0, &identity))
            .min();
        if let Some(index) = target {
            groups[index].1.extend(items);
        } else {
            let index = groups.len();
            by_name.entry(name_key).or_default().push(index);
            by_year
                .entry((identity.season.clone(), identity.year.clone(), length))
                .or_default()
                .push(index);
            groups.push((identity, items));
        }
    }
    let keyword = normalize_name(keyword);
    let mut merged = Vec::new();
    for (_, mut items) in groups {
        if items.len() > 1 {
            items.sort_by(|a, b| {
                score(b).cmp(&score(a)).then_with(|| {
                    s(a, "vod_name")
                        .encode_utf16()
                        .count()
                        .cmp(&s(b, "vod_name").encode_utf16().count())
                })
            });
            let mut seen = HashSet::new();
            items.retain(|v| seen.insert(s(v, "siteKey")));
        }
        if items.is_empty() {
            continue;
        }
        let mut best = (*items[0]).clone();
        best["allSiteKeys"] = json!(
            items
                .iter()
                .map(|v| v["siteKey"].clone())
                .collect::<Vec<_>>()
        );
        best["alternatives"] = json!(&items[1..]);
        let name = s(&best, "vod_name");
        let rank = match_rank(&name, &keyword);
        let quality = score(&best);
        merged.push((best, rank, items.len(), quality, name));
    }
    merged.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then_with(|| b.2.cmp(&a.2))
            .then_with(|| b.3.cmp(&a.3))
            .then_with(|| compare_cn(&a.4, &b.4))
    });
    if let Some(order) = previous_order.filter(|v| !v.is_empty()) {
        merged.sort_by_cached_key(|v| order.get(&key(&v.0)).copied().unwrap_or(usize::MAX));
    }
    let merged: Vec<_> = merged.into_iter().map(|v| v.0).collect();
    let mut output = json!({"hidden":hidden});
    output["merged"] = Value::Array(merged);
    output
}
