use crate::model::{array, s, text};
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde_json::{Value, json};
use std::{collections::HashSet, sync::LazyLock};

static SLOTS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\{(ac|wd|kw|key|keyword|pg|page|t|type|typeid|class|ids|id|h)\}")
        .expect("constant regex")
});
pub fn build_url(site: &Value, query: &Value) -> Result<String> {
    let api = s(site, "api");
    let mut slots = HashSet::new();
    let expanded = SLOTS.replace_all(&api, |capture: &regex::Captures<'_>| {
        let field = match capture[1].to_lowercase().as_str() {
            "wd" | "kw" | "key" | "keyword" => "wd",
            "pg" | "page" => "pg",
            "t" | "type" | "typeid" | "class" => "t",
            "ids" | "id" => "ids",
            "h" => "h",
            _ => "ac",
        };
        slots.insert(field);
        crate::spider::encode(&text(&query[field]))
    });
    let mut address = url::Url::parse(&expanded).context("CMS 接口地址无效")?;
    let mut pairs: Vec<(String, String)> = address
        .query_pairs()
        .map(|(a, b)| (a.into_owned(), b.into_owned()))
        .collect();
    if let Some(query) = query.as_object() {
        for (field, value) in query
            .iter()
            .filter(|(field, value)| !value.is_null() && !slots.contains(field.as_str()))
        {
            let mut first = true;
            pairs.retain_mut(|(key, old)| {
                if key != field {
                    return true;
                }
                if first {
                    *old = text(value);
                    first = false;
                    true
                } else {
                    false
                }
            });
            if first {
                pairs.push((field.clone(), text(value)));
            }
        }
    }
    address.set_query(None);
    if !pairs.is_empty() {
        address.query_pairs_mut().extend_pairs(pairs);
    }
    Ok(address.to_string())
}

pub fn is_ok(payload: &Value) -> bool {
    if !payload.is_object() {
        return false;
    }
    let code = s(payload, "code").to_lowercase();
    matches!(code.as_str(), "" | "1" | "true" | "ok" | "success" | "200")
}

pub fn parse_payload(input: &str) -> Result<Value> {
    let input = input.trim_start_matches('﻿').trim();
    if !input.starts_with('<') {
        let payload = match serde_json::from_str::<Value>(input) {
            Ok(payload) => payload,
            Err(_) => {
                let start = input.find('{').context("返回内容不是合法 JSON")?;
                let end = input.rfind('}').context("返回内容不是合法 JSON")?;
                serde_json::from_str(&input[start..=end]).context("返回内容不是合法 JSON")?
            }
        };
        if !payload.is_object()
            || !["list", "class", "code"]
                .iter()
                .any(|key| payload.get(key).is_some())
        {
            bail!("接口未返回 CMS 数据");
        }
        return Ok(payload);
    }
    let document = roxmltree::Document::parse(input).context("CMS XML 格式无效")?;
    let rss = document
        .descendants()
        .find(|v| v.has_tag_name("rss"))
        .context("接口未返回 CMS XML 数据")?;
    let list = rss.children().find(|v| v.has_tag_name("list"));
    let classes = rss.children().find(|v| v.has_tag_name("class"));
    if list.is_none() && classes.is_none() {
        bail!("接口未返回 CMS XML 数据");
    }
    let classes: Vec<Value> = classes.into_iter().flat_map(|v| v.children()).filter(|v| v.has_tag_name("ty")).map(|v| json!({"type_id":v.attribute("id").unwrap_or(""),"type_name":xml_text(v),"type_pid":v.attribute("pid").unwrap_or("0")})).collect();
    let items: Vec<Value> = list
        .into_iter()
        .flat_map(|v| v.children())
        .filter(|v| v.has_tag_name("video"))
        .map(|node| {
            let mut item = json!({});
            for (tag, field) in [
                ("id", "vod_id"),
                ("name", "vod_name"),
                ("pic", "vod_pic"),
                ("note", "vod_remarks"),
                ("year", "vod_year"),
                ("area", "vod_area"),
                ("actor", "vod_actor"),
                ("director", "vod_director"),
                ("type", "type_name"),
                ("score", "vod_score"),
                ("des", "vod_content"),
            ] {
                item[field] = json!(
                    node.children()
                        .filter(|v| v.has_tag_name(tag))
                        .map(xml_text)
                        .collect::<String>()
                );
            }
            let lines: Vec<_> = node
                .descendants()
                .filter(|v| {
                    v.has_tag_name("dd") && v.parent().is_some_and(|p| p.has_tag_name("dl"))
                })
                .collect();
            item["vod_play_from"] = json!(
                lines
                    .iter()
                    .map(|v| v.attribute("flag").unwrap_or(""))
                    .collect::<Vec<_>>()
                    .join("$$$")
            );
            item["vod_play_url"] = json!(
                lines
                    .into_iter()
                    .map(xml_text)
                    .collect::<Vec<_>>()
                    .join("$$$")
            );
            item
        })
        .collect();
    Ok(
        json!({"code":1,"page":list.and_then(|v|v.attribute("page")),"pagecount":list.and_then(|v|v.attribute("pagecount")),"total":list.and_then(|v|v.attribute("recordcount")),"class":classes,"list":items}),
    )
}

fn xml_text(node: roxmltree::Node<'_, '_>) -> String {
    node.descendants()
        .filter(|v| v.is_text())
        .filter_map(|v| v.text())
        .collect::<String>()
        .trim()
        .to_owned()
}

pub fn item_to_vod(raw: &Value, site: &Value) -> Value {
    let mut out =
        json!({"siteKey":site["key"],"siteName":site["name"],"sourceId":site["sourceId"]});
    for (key, alias) in [
        ("vod_id", "id"),
        ("vod_name", "name"),
        ("vod_pic", "pic"),
        ("vod_remarks", "remarks"),
        ("vod_year", "year"),
        ("vod_area", "area"),
        ("vod_actor", "actor"),
        ("vod_director", "director"),
        ("type_name", "class"),
        ("vod_score", "score"),
    ] {
        out[key] = json!(
            raw.get(key)
                .or_else(|| raw.get(alias))
                .map(text)
                .unwrap_or_default()
        );
    }
    if s(site, "spiderClass").is_empty() {
        out["vod_pic"] = json!(crate::html_rules::absolutize(
            &s(&out, "vod_pic"),
            &s(site, "api")
        ));
    }
    out
}

pub fn parse_lines(from: &str, address: &str, note: &str) -> Vec<Value> {
    let from: Vec<_> = from.split("$$$").collect();
    let notes: Vec<_> = note.split("$$$").collect();
    address.split("$$$").enumerate().filter_map(|(line, group)| {
        let name = from.get(line).filter(|v|!v.is_empty()).or_else(||notes.get(line).filter(|v|!v.is_empty())).map(|v|v.trim().to_owned()).unwrap_or_else(||format!("线路{}", line+1));
        let mut episodes = Vec::new();
        for segment in group.split('#').map(str::trim).filter(|v|!v.is_empty()) {
            let (label, address) = segment.split_once('$').unwrap_or(("", segment));
            if address.trim().is_empty() { continue; }
            episodes.push(json!({"name":if label.trim().is_empty(){format!("第{}集",episodes.len()+1)}else{label.trim().to_owned()},"url":address.trim(),"index":episodes.len()}));
        }
        if episodes.is_empty() { None } else { Some(json!({"name":name,"episodes":episodes})) }
    }).collect()
}

pub fn detail(raw: &Value, site: &Value) -> Value {
    let mut out = item_to_vod(raw, site);
    let content = raw
        .get("vod_content")
        .or_else(|| raw.get("vod_blurb"))
        .map(text)
        .unwrap_or_default();
    static TAGS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"<[^>]+>").expect("constant regex"));
    out["vod_content"] = json!(TAGS.replace_all(&content, "").trim());
    let mut lines = parse_lines(
        &s(raw, "vod_play_from"),
        &s(raw, "vod_play_url"),
        &s(raw, "vod_play_note"),
    );
    if !s(site, "spiderClass").is_empty() {
        for line in &mut lines {
            let flag = s(line, "name");
            for episode in line["episodes"].as_array_mut().expect("episodes") {
                episode["url"] = json!(crate::spider::make_url(
                    &s(site, "key"),
                    &flag,
                    &s(episode, "url")
                ));
            }
        }
    }
    out["lines"] = json!(lines);
    out
}

pub fn list(payload: &Value, site: &Value, limit: usize) -> Vec<Value> {
    array(&payload["list"])
        .iter()
        .take(limit)
        .map(|v| item_to_vod(v, site))
        .filter(|v| !s(v, "vod_id").is_empty() && !s(v, "vod_name").is_empty())
        .collect()
}
