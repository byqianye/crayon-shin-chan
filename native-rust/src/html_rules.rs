use crate::{cms, model::s};
use anyhow::{Result, bail};
use scraper::{ElementRef, Html, Selector};
use serde_json::{Value, json};

pub fn absolutize(input: &str, base: &str) -> String {
    let value = input.trim();
    if value.is_empty()
        || value.starts_with("data:")
        || value.starts_with("http://")
        || value.starts_with("https://")
    {
        return value.to_owned();
    }
    if value.starts_with("//") {
        return format!("https:{value}");
    }
    crate::config::resolve(value, base)
}

pub fn select<'a>(scope: ElementRef<'a>, rule: &str) -> Vec<ElementRef<'a>> {
    let selector = rule
        .split("||")
        .next()
        .unwrap_or("")
        .split("&&")
        .next()
        .unwrap_or("")
        .trim();
    Selector::parse(selector)
        .map(|v| scope.select(&v).collect())
        .unwrap_or_default()
}

pub fn extract(scope: ElementRef<'_>, rule: &str, base: &str, fallback: &[&str]) -> String {
    let branches: Vec<_> = if rule.trim().is_empty() {
        vec![""]
    } else {
        rule.split("||")
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .collect()
    };
    for branch in branches {
        let segments: Vec<_> = branch
            .split("&&")
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .collect();
        let selector = segments.first().copied().unwrap_or("");
        let target = if selector.is_empty() {
            Some(scope)
        } else {
            Selector::parse(selector)
                .ok()
                .and_then(|v| scope.select(&v).next())
        };
        let Some(target) = target else {
            continue;
        };
        let arguments: Vec<_> = segments
            .iter()
            .skip(1)
            .flat_map(|v| v.split(','))
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .collect();
        let arguments = if arguments.is_empty() {
            fallback
        } else {
            &arguments
        };
        for argument in arguments {
            let lower = argument.to_lowercase();
            let value = match lower.as_str() {
                "text" | "textnodes" => target
                    .text()
                    .collect::<String>()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
                "html" | "innerhtml" => target.inner_html().trim().to_owned(),
                "outerhtml" => target.html().trim().to_owned(),
                _ => target
                    .value()
                    .attr(argument)
                    .or_else(|| target.value().attr(&lower))
                    .unwrap_or("")
                    .trim()
                    .to_owned(),
            };
            if value.is_empty() {
                continue;
            }
            return if matches!(lower.as_str(), "href" | "src")
                || lower.ends_with("src")
                || lower.ends_with("-url")
            {
                absolutize(&value, base)
            } else {
                value
            };
        }
    }
    String::new()
}

pub fn search(input: &str, site: &Value, address: &str, limit: usize) -> Result<Vec<Value>> {
    let html = Html::parse_document(input);
    let rules = &site["rules"];
    let nodes = select(html.root_element(), &s(rules, "list"));
    if nodes.is_empty() {
        bail!("列表选择器没有匹配到节点，规则可能已失效");
    }
    Ok(nodes.into_iter().take(limit).filter_map(|node| {
        let name = extract(node, &s(rules,"name"),address,&["Text"]);
        if name.is_empty() { return None; }
        let link = extract(node,&s(rules,"link"),address,&["href","Text"]);
        let raw = json!({"vod_id":if link.is_empty(){&name}else{&link},"vod_name":name,"vod_pic":extract(node,&s(rules,"pic"),address,&["data-src","data-original","src"]),"vod_remarks":extract(node,&s(rules,"remarks"),address,&["Text"])});
        Some(cms::item_to_vod(&raw,site))
    }).collect())
}

pub fn detail(input: &str, site: &Value, address: &str) -> Result<Value> {
    let html = Html::parse_document(input);
    let scope = select(html.root_element(), "body")
        .into_iter()
        .next()
        .unwrap_or(html.root_element());
    let rules = &site["rules"];
    let name = [
        extract(scope, &s(rules, "detailName"), address, &["Text"]),
        extract(scope, "h1", address, &["Text"]),
        extract(html.root_element(), "title", address, &["Text"]),
    ]
    .into_iter()
    .find(|v| !v.is_empty())
    .unwrap_or_default();
    let mut lines = Vec::new();
    let items = |node: ElementRef<'_>| -> Vec<Value> {
        let mut episodes = Vec::new();
        for anchor in select(node, &s(rules, "playItem")) {
            let url = anchor.value().attr("href").unwrap_or("").trim();
            if url.is_empty() {
                continue;
            }
            let label = anchor
                .text()
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            episodes.push(json!({"name":if label.is_empty(){format!("第{}集",episodes.len()+1)}else{label},"url":url,"index":episodes.len()}));
        }
        episodes
    };
    if !s(rules, "playItem").is_empty() {
        for (i, node) in select(scope, &s(rules, "playList")).into_iter().enumerate() {
            let episodes = items(node);
            if episodes.is_empty() {
                continue;
            }
            let title = node
                .value()
                .attr("data-name")
                .unwrap_or("")
                .trim()
                .to_owned();
            let title = if title.is_empty() {
                extract(node, "h3,h2,.title,.name", address, &["Text"])
            } else {
                title
            };
            lines.push(json!({"name":if title.is_empty(){format!("线路{}",i+1)}else{title},"episodes":episodes}));
        }
        if lines.is_empty() {
            let episodes = items(scope);
            if !episodes.is_empty() {
                lines.push(json!({"name":"默认线路","episodes":episodes}));
            }
        }
    }
    if name.is_empty() && lines.is_empty() {
        bail!("详情页没有解析出内容，规则可能已失效");
    }
    let mut out = cms::item_to_vod(
        &json!({"vod_id":address,"vod_name":if name.is_empty(){address}else{&name},"vod_pic":extract(scope,&s(rules,"pic"),address,&["data-src","data-original","src"])}),
        site,
    );
    out["vod_content"] = json!(
        extract(scope, &s(rules, "detailContent"), address, &["Text"])
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    );
    out["lines"] = json!(lines);
    Ok(out)
}
