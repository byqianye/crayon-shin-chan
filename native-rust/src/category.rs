use crate::{
    catalogue::{Catalogue, fail, interleave, success},
    cms,
    model::{array, number, s},
};
use anyhow::{Context, Result, bail};
use futures_util::{StreamExt, stream};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, LazyLock},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

static SKIP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new("解说|演员|明星|资讯|新闻|预告|花絮|伦理|福利|写真|片花|榜单")
        .expect("constant regex")
});
const ORDER: &[&str] = &[
    "电影",
    "电视剧",
    "动漫",
    "综艺",
    "纪录片",
    "短剧",
    "体育",
    "少儿",
];
static BUCKETS: LazyLock<Vec<(&str, Regex)>> = LazyLock::new(|| {
    [
        (
            "电影",
            "电影|影院|^(动作|喜剧|爱情|科幻|恐怖|惊悚|剧情|战争|犯罪|悬疑|冒险|灾难)片?$",
        ),
        ("短剧", "短剧|微剧|竖屏"),
        ("纪录片", "纪录|记录|纪实"),
        ("动漫", "动漫|动画|番剧|漫画|漫剧"),
        ("综艺", "综艺|真人秀|脱口秀"),
        ("体育", "体育|赛事|足球|篮球|台球"),
        ("少儿", "少儿|儿童|亲子|幼教|益智"),
        (
            "电视剧",
            "电视剧|连续剧|电视|剧集|国产剧|美剧|韩剧|日剧|港剧|台剧|泰剧",
        ),
    ]
    .into_iter()
    .map(|(name, pattern)| (name, Regex::new(pattern).expect("constant regex")))
    .collect()
});
type SubRules = HashMap<&'static str, Vec<(&'static str, Regex)>>;
static SUBS: LazyLock<SubRules> = LazyLock::new(|| {
    let rules: &[(&str, &[(&str, &str)])] = &[
        (
            "电影",
            &[
                ("动作", "动作"),
                ("喜剧", "喜剧"),
                ("爱情", "爱情"),
                ("科幻", "科幻"),
                ("恐怖", "恐怖|惊悚"),
                ("剧情", "剧情"),
                ("战争", "战争"),
                ("动画", "动画|动漫"),
            ],
        ),
        (
            "电视剧",
            &[
                ("国产", "国产|大陆"),
                ("香港", "香港|港澳|港剧"),
                ("台湾", "台湾|台剧"),
                ("韩国", "韩国|韩剧"),
                ("日本", "日本|日剧"),
                ("欧美", "欧美|美剧"),
                ("泰国", "泰国|泰剧"),
                ("海外", "海外|其他"),
            ],
        ),
        (
            "动漫",
            &[
                ("国产", "国产|中国"),
                ("日本", "日本|日韩"),
                ("欧美", "欧美"),
                ("港台", "港台"),
                ("海外", "海外"),
                ("动画电影", "电影"),
                ("动画片", "动画片"),
            ],
        ),
        (
            "综艺",
            &[
                ("大陆", "大陆|国产"),
                ("港台", "港台"),
                ("日韩", "日韩"),
                ("欧美", "欧美"),
            ],
        ),
        (
            "体育",
            &[
                ("足球", "足球"),
                ("篮球", "篮球"),
                ("网球", "网球"),
                ("台球", "台球|斯诺克"),
                ("其他", "其他|综合"),
            ],
        ),
        (
            "短剧",
            &[
                ("擦边短剧", "擦边"),
                ("古装仙侠", "古装|仙侠"),
                ("现代都市", "现代|都市"),
                ("穿越年代", "穿越|年代"),
                ("言情总裁", "言情|总裁"),
                ("重生民国", "重生|民国"),
                ("反转爽剧", "反转|爽剧"),
                ("脑洞悬疑", "脑洞|悬疑"),
                ("短剧", "短剧|微剧|竖屏"),
            ],
        ),
        ("纪录片", &[("纪录片", "纪录|记录|纪实")]),
    ];
    rules
        .iter()
        .map(|(bucket, subs)| {
            (
                *bucket,
                subs.iter()
                    .map(|(name, pattern)| (*name, Regex::new(pattern).expect("constant regex")))
                    .collect(),
            )
        })
        .collect()
});

pub fn leaves(classes: &[Value]) -> Vec<Value> {
    let by_id: HashMap<_, _> = classes.iter().map(|v| (s(v, "id"), v)).collect();
    let parents: HashSet<_> = classes
        .iter()
        .filter(|v| v["pid"] != "0")
        .map(|v| s(v, "pid"))
        .collect();
    let mut out = Vec::new();
    for leaf in classes.iter().filter(|v| !parents.contains(&s(v, "id"))) {
        let mut chain = vec![leaf];
        for _ in 0..7 {
            let parent = s(chain.last().expect("chain"), "pid");
            if parent == "0" {
                break;
            }
            if let Some(next) = by_id.get(&parent) {
                chain.push(next);
            } else {
                break;
            }
        }
        if chain.iter().any(|v| SKIP.is_match(&s(v, "name"))) {
            continue;
        }
        let bucket = chain.iter().find_map(|v| {
            BUCKETS
                .iter()
                .find(|(_, pattern)| pattern.is_match(&s(v, "name")))
                .map(|(name, _)| *name)
        });
        let bucket = if s(leaf, "name") == "动画片"
            && chain.iter().any(|v| s(v, "name").contains("电影"))
        {
            Some("电影")
        } else {
            bucket
        };
        let Some(bucket) = bucket else {
            continue;
        };
        let name = s(leaf, "name");
        let rules = SUBS.get(bucket).map(Vec::as_slice).unwrap_or(&[]);
        let found = rules
            .iter()
            .enumerate()
            .find(|(_, (_, pattern))| pattern.is_match(&name));
        let (order, sub) = found
            .map(|(i, (name, _))| (i, *name))
            .unwrap_or((rules.len(), name.as_str()));
        out.push(json!({"id":leaf["id"],"bucket":bucket,"sub":sub,"order":order}));
    }
    out
}

impl Catalogue {
    pub async fn categories(self: &Arc<Self>) -> Result<Vec<Value>> {
        let trees: Vec<_> = stream::iter(self.available()?.into_iter().map(|site| async move {
            let classes = self.classes(&site).await;
            (site, classes)
        }))
        .buffered(self.concurrency())
        .collect()
        .await;
        let mut buckets: Vec<Value> = Vec::new();
        for (site, classes) in trees {
            let classes = match classes {
                Ok(v) => v,
                Err(error) => {
                    eprintln!("[category] {}：{error:#}", s(&site, "name"));
                    continue;
                }
            };
            for leaf in leaves(&classes) {
                let bucket = s(&leaf, "bucket");
                let sub = s(&leaf, "sub");
                let bi = if let Some(i) = buckets.iter().position(|v| s(v, "name") == bucket) {
                    i
                } else {
                    buckets.push(json!({"name":bucket,"subs":[]}));
                    buckets.len() - 1
                };
                let subs = buckets[bi]["subs"].as_array_mut().expect("subs");
                let si = if let Some(i) = subs.iter().position(|v| s(v, "name") == sub) {
                    i
                } else {
                    subs.push(json!({"name":sub,"order":leaf["order"],"sites":[]}));
                    subs.len() - 1
                };
                let sites = subs[si]["sites"].as_array_mut().expect("sites");
                let ei = if let Some(i) = sites.iter().position(|v| v["siteKey"] == site["key"]) {
                    i
                } else {
                    sites.push(json!({"siteKey":site["key"],"siteName":site["name"],"typeIds":[]}));
                    sites.len() - 1
                };
                let ids = sites[ei]["typeIds"].as_array_mut().expect("ids");
                if !ids.contains(&leaf["id"]) {
                    ids.push(leaf["id"].clone());
                }
            }
        }
        for bucket in &mut buckets {
            let subs = bucket["subs"].as_array_mut().expect("subs");
            subs.sort_by(|a, b| {
                number(&a["order"], 0.0)
                    .total_cmp(&number(&b["order"], 0.0))
                    .then_with(|| crate::model::compare_cn(&s(a, "name"), &s(b, "name")))
            });
            for sub in subs {
                sub.as_object_mut().expect("sub").remove("order");
            }
        }
        buckets.sort_by_key(|v| {
            ORDER
                .iter()
                .position(|name| *name == s(v, "name"))
                .unwrap_or(ORDER.len())
        });
        Ok(buckets)
    }

    pub async fn browse(self: &Arc<Self>, bucket: &str, sub: &str, page: usize) -> Result<Value> {
        let target = self
            .categories()
            .await?
            .into_iter()
            .find(|v| s(v, "name") == bucket)
            .with_context(|| format!("没有找到「{bucket}」这个分类"))?;
        let subs: Vec<_> = array(&target["subs"])
            .into_iter()
            .filter(|v| sub.is_empty() || s(v, "name") == sub)
            .collect();
        if subs.is_empty() {
            bail!("「{bucket}」下没有「{sub}」这个子分类");
        }
        let sites: HashMap<_, _> = self
            .available()?
            .into_iter()
            .map(|v| (s(&v, "key"), v))
            .collect();
        let mut jobs = Vec::new();
        let mut seen = HashSet::new();
        for sub in subs {
            for entry in array(&sub["sites"]) {
                let key = s(&entry, "siteKey");
                if let Some(site) = sites.get(&key) {
                    for id in array(&entry["typeIds"]) {
                        let id = crate::model::text(&id);
                        if seen.insert((key.clone(), id.clone())) {
                            jobs.push((site.clone(), id));
                        }
                    }
                }
            }
        }
        let pages: Vec<_> = stream::iter(jobs.into_iter().map(|(site, id)| async move {
            let (result, depth) = self.browse_site(&site, &id, page, 30).await;
            (site, result, depth)
        }))
        .buffered(self.concurrency())
        .collect()
        .await;
        let mut per_site: Vec<(Value, Vec<Value>, usize)> = Vec::new();
        for (site, result, depth) in pages {
            let index = if let Some(i) = per_site
                .iter()
                .position(|(v, _, _)| v["key"] == site["key"])
            {
                i
            } else {
                per_site.push((site, Vec::new(), 0));
                per_site.len() - 1
            };
            per_site[index].1.push(result);
            per_site[index].2 = per_site[index].2.max(depth);
        }
        let mut results = Vec::new();
        let mut page_count = page;
        for (site, parts, depth) in per_site {
            let lists: Vec<_> = parts
                .iter()
                .filter(|v| v["ok"] == true)
                .map(|v| array(&v["list"]))
                .collect();
            let list: Vec<_> = interleave(&lists).into_iter().take(30).collect();
            if list.is_empty() {
                results.push(
                    parts
                        .iter()
                        .find(|v| v["ok"] == false)
                        .cloned()
                        .unwrap_or_else(|| fail(&site, "这个分类下没有返回内容", 0)),
                );
            } else {
                page_count = page_count.max(depth);
                results.push(success(
                    &site,
                    list,
                    parts
                        .iter()
                        .map(|v| number(&v["ms"], 0.0) as u64)
                        .max()
                        .unwrap_or(0),
                    "返回",
                ));
            }
        }
        Ok(json!({"page":page,"pageCount":page_count,"bucket":bucket,"sub":sub,"results":results}))
    }

    async fn browse_site(
        self: &Arc<Self>,
        site: &Value,
        tid: &str,
        page: usize,
        limit: usize,
    ) -> (Value, usize) {
        let started = Instant::now();
        let spider = !s(site, "spiderClass").is_empty();
        let payload = if spider {
            self.spider
                .call(
                    site,
                    "category",
                    json!({"tid":tid,"pg":page.to_string(),"filter":true,"extend":{}}),
                    120.0,
                    &CancellationToken::new(),
                )
                .await
        } else {
            self.request_cms(site, json!({"ac":"detail","t":tid,"pg":page}), None)
                .await
        };
        let ms = started.elapsed().as_millis() as u64;
        match payload {
            Ok(payload) if cms::is_ok(&payload) => {
                let list = cms::list(&payload, site, limit);
                let reported = number(
                    payload
                        .get("pagecount")
                        .or_else(|| payload.get("pageCount"))
                        .or_else(|| if spider { payload.get("total") } else { None })
                        .unwrap_or(&Value::Null),
                    0.0,
                );
                let depth = if reported > 0.0 && (!spider || reported <= 1000.0) {
                    reported as usize
                } else if list.is_empty() {
                    page
                } else {
                    page + 1
                };
                if list.is_empty() {
                    (fail(site, "这个分类下没有返回内容", ms), depth)
                } else {
                    (success(site, list, ms, "返回"), depth)
                }
            }
            Ok(payload) => (
                fail(site, &format!("接口返回异常：{}", s(&payload, "msg")), ms),
                0,
            ),
            Err(error) => (fail(site, &format!("{error:#}"), ms), 0),
        }
    }
}
