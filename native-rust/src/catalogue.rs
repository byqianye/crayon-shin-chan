use crate::{
    cache::Cache,
    cms, health, html_rules,
    model::{array, flag, number, s},
    sources::Sources,
    spider::Spider,
};
use anyhow::{Context, Result, bail};
use futures_util::{StreamExt, stream};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

pub struct Catalogue {
    pub sources: Arc<Sources>,
    pub spider: Arc<Spider>,
    pub cache: Cache,
    search_cancel: Mutex<CancellationToken>,
}
pub type SearchProgress = Arc<dyn Fn(Value, usize, usize) + Send + Sync>;
pub type LoadProgress = Arc<dyn Fn(Value) + Send + Sync>;

impl Catalogue {
    pub fn new(sources: Arc<Sources>) -> Self {
        let spider = Arc::new(Spider::new(sources.store.clone(), sources.net.clone()));
        Self {
            sources,
            spider,
            cache: Cache::default(),
            search_cancel: Mutex::new(CancellationToken::new()),
        }
    }
    pub fn concurrency(&self) -> usize {
        self.sources
            .store
            .settings()
            .map(|v| number(&v["concurrency"], 8.0).clamp(1.0, 32.0) as usize)
            .unwrap_or(8)
    }
    pub fn cancel_search(&self) {
        self.search_cancel.lock().expect("cancel lock").cancel();
    }
    pub fn available(&self) -> Result<Vec<Value>> {
        Ok(self
            .sources
            .sites()?
            .into_iter()
            .filter(content_site)
            .collect())
    }

    pub async fn request_cms(
        self: &Arc<Self>,
        site: &Value,
        query: Value,
        cancel: Option<&CancellationToken>,
    ) -> Result<Value> {
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            bail!("请求已取消");
        }
        let address = cms::build_url(site, &query)?;
        let headers = self.sources.headers(site, &address)?;
        let timeout = number(
            &site["timeout"],
            number(&self.sources.store.settings()?["timeout"], 20.0),
        );
        let revision = self.sources.revision();
        let identity = json!([
            revision,
            site["sourceId"],
            site["key"],
            site["api"],
            headers,
            timeout
        ])
        .to_string();
        let detail_key = format!("detail:{identity}:{}", crate::model::text(&query["ids"]));
        if query.get("ids").is_some()
            && let Some(raw) = self.cache.read(&detail_key)
        {
            return Ok(json!({"code":1,"list":[raw]}));
        }
        let token = cancel.cloned().unwrap_or_default();
        let this = self.clone();
        let pending_key = format!("cms:{identity}:{address}");
        let site = site.clone();
        let operation = async move {
            let payload = if crate::websites::supported(&site) {
                crate::websites::request(&this.sources, &site, &query, &token).await?
            } else {
                cms::parse_payload(
                    &this
                        .sources
                        .net
                        .text(&address, &headers, timeout, &token)
                        .await?,
                )?
            };
            if revision == this.sources.revision() && cms::is_ok(&payload) {
                for raw in array(&payload["list"]) {
                    let id = raw
                        .get("vod_id")
                        .or_else(|| raw.get("id"))
                        .map(crate::model::text)
                        .unwrap_or_default();
                    if id.is_empty()
                        || s(&raw, "vod_play_url").is_empty()
                        || raw.to_string().len() > 64 * 1024
                    {
                        continue;
                    }
                    this.cache.insert(
                        format!("detail:{identity}:{id}"),
                        raw,
                        Duration::from_secs(120),
                        256,
                    );
                }
            }
            Ok(payload)
        };
        if cancel.is_some() {
            operation.await
        } else {
            self.cache.coalesce(pending_key, operation).await
        }
    }

    pub async fn search_site(
        self: &Arc<Self>,
        site: &Value,
        keyword: &str,
        limit: usize,
        cancel: &CancellationToken,
    ) -> Value {
        self.search_site_recorded(site, keyword, limit, cancel, None)
            .await
    }

    async fn search_site_recorded(
        self: &Arc<Self>,
        site: &Value,
        keyword: &str,
        limit: usize,
        cancel: &CancellationToken,
        batch: Option<&health::Batch<'_>>,
    ) -> Value {
        let started = Instant::now();
        if !s(site, "unsupported").is_empty() {
            return fail(site, &s(site, "unsupported"), 0);
        }
        if !flag(&site["searchable"], true) {
            return fail(site, "该站点已关闭搜索", 0);
        }
        let result = async {
            let list = if !s(site, "spiderClass").is_empty() {
                let payload = self
                    .spider
                    .call(
                        site,
                        "search",
                        json!({"key":keyword,"quick":false}),
                        number(&site["timeout"], 35.0),
                        cancel,
                    )
                    .await?;
                cms::list(&payload, site, limit)
            } else if [3, 4].contains(&site["type"].as_i64().unwrap_or(2)) {
                let ac = if s(site, "api").to_lowercase().contains("{ac}") {
                    "videolist"
                } else {
                    "detail"
                };
                let payload = self
                    .request_cms(site, json!({"ac":ac,"wd":keyword,"pg":1}), Some(cancel))
                    .await?;
                if !cms::is_ok(&payload) {
                    bail!("接口返回异常：{}", s(&payload, "msg"));
                }
                cms::list(&payload, site, limit)
            } else if site["type"] == 1 {
                static SLOT: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
                    Regex::new(r"(?i)\{(wd|kw|key|keyword)\}").expect("constant regex")
                });
                let template = if s(&site["rules"], "search").is_empty() {
                    s(site, "api")
                } else {
                    s(&site["rules"], "search")
                };
                let address = SLOT
                    .replace_all(&template, crate::spider::encode(keyword))
                    .to_string();
                let headers = self.sources.headers(site, &address)?;
                let input = self
                    .sources
                    .net
                    .text(&address, &headers, number(&site["timeout"], 20.0), cancel)
                    .await?;
                html_rules::search(&input, site, &address, limit)?
            } else {
                bail!("该类型暂不支持搜索");
            };
            if cancel.is_cancelled() {
                bail!("搜索已取消");
            }
            Ok::<_, anyhow::Error>(list)
        }
        .await;
        let ms = started.elapsed().as_millis() as u64;
        if cancel.is_cancelled() {
            return fail(site, "搜索已取消", ms);
        }
        let ok = result.is_ok();
        let recorded = if let Some(batch) = batch {
            batch.record(&s(site, "key"), ok, ms)
        } else {
            health::record(&self.sources.store, &s(site, "key"), ok, ms).map(|_| ())
        };
        if let Err(error) = recorded {
            return fail(site, &format!("无法记录健康状态：{error:#}"), ms);
        }
        match result {
            Ok(list) => success(site, list, ms, "找到"),
            Err(error) => fail(site, &format!("{error:#}"), ms),
        }
    }

    pub async fn search(
        self: &Arc<Self>,
        keyword: &str,
        options: Value,
        on_result: Option<SearchProgress>,
    ) -> Result<Value> {
        let started = Instant::now();
        let keyword = keyword.trim();
        self.cancel_search();
        let cancel = CancellationToken::new();
        *self.search_cancel.lock().expect("cancel lock") = cancel.clone();
        if keyword.is_empty() {
            return Ok(json!({"wd":keyword,"ms":0,"total":0,"results":[]}));
        }
        let selected = array(&options["sites"]);
        let mut sites: Vec<_> = self
            .sources
            .sites()?
            .into_iter()
            .filter(|v| {
                flag(&v["enabled"], true)
                    && flag(&v["searchable"], true)
                    && s(v, "unsupported").is_empty()
                    && (selected.is_empty() || selected.contains(&v["key"]))
            })
            .collect();
        let health = self.sources.store.read("health", json!({}))?;
        sites.sort_by(|a, b| {
            let left = &health[s(a, "key")];
            let right = &health[s(b, "key")];
            health::cooling(left)
                .cmp(&health::cooling(right))
                .then_with(|| {
                    (!s(a, "spiderClass").is_empty()).cmp(&(!s(b, "spiderClass").is_empty()))
                })
                .then_with(|| {
                    number(&health::summary(right)["score"], 0.5)
                        .total_cmp(&number(&health::summary(left)["score"], 0.5))
                })
        });
        let total = sites.len();
        let concurrency =
            number(&options["concurrency"], self.concurrency() as f64).clamp(1.0, 32.0) as usize;
        let limit = number(&options["limitPerSite"], 30.0).clamp(1.0, 500.0) as usize;
        let batch = health::Batch::new(&self.sources.store);
        let mut tasks = stream::iter(sites.into_iter().map(|site| {
            let cancel = cancel.clone();
            let batch = &batch;
            let cooling = health::cooling(&health[s(&site, "key")]);
            async move {
                if cooling {
                    fail(&site, "该站点连续失败，冷却中", 0)
                } else {
                    self.search_site_recorded(&site, keyword, limit, &cancel, Some(batch))
                        .await
                }
            }
        }))
        .buffer_unordered(concurrency);
        let mut results = Vec::new();
        while let Some(result) = tasks.next().await {
            if cancel.is_cancelled() {
                break;
            }
            results.push(result.clone());
            if let Some(callback) = &on_result {
                callback(result, results.len(), total);
            }
        }
        batch.flush()?;
        Ok(
            json!({"wd":keyword,"ms":started.elapsed().as_millis() as u64,"total":total,"results":results}),
        )
    }

    pub async fn detail(self: &Arc<Self>, key: &str, id: &str) -> Result<Value> {
        let site = self.sources.site(key)?;
        if !s(&site, "unsupported").is_empty() {
            bail!("{}", s(&site, "unsupported"));
        }
        if id.is_empty() {
            bail!("缺少影片 id");
        }
        let started = Instant::now();
        let cancel = CancellationToken::new();
        let result = async {
            if !s(&site, "spiderClass").is_empty() {
                let payload = self
                    .spider
                    .call(&site, "detail", json!({"ids":[id]}), 60.0, &cancel)
                    .await?;
                let raw = array(&payload["list"])
                    .into_iter()
                    .next()
                    .context("Spider 没有返回该影片的详情")?;
                return Ok(cms::detail(&raw, &site));
            }
            if [3, 4].contains(&site["type"].as_i64().unwrap_or(2)) {
                let payload = self
                    .request_cms(&site, json!({"ac":"detail","ids":id}), None)
                    .await?;
                if !cms::is_ok(&payload) {
                    bail!("接口返回异常：{}", s(&payload, "msg"));
                }
                let raw = array(&payload["list"])
                    .into_iter()
                    .next()
                    .context("接口没有返回该影片的详情")?;
                return Ok(cms::detail(&raw, &site));
            }
            if site["type"] == 1 {
                if !id.starts_with("http") {
                    bail!("该网页规则站没有解析出可用的详情页地址");
                }
                let input = self
                    .sources
                    .net
                    .text(
                        id,
                        &self.sources.headers(&site, id)?,
                        number(&site["timeout"], 20.0),
                        &cancel,
                    )
                    .await?;
                return html_rules::detail(&input, &site, id);
            }
            bail!("该类型暂不支持获取详情")
        }
        .await;
        health::record(
            &self.sources.store,
            key,
            result.is_ok(),
            started.elapsed().as_millis() as u64,
        )?;
        result
    }

    pub async fn classes(self: &Arc<Self>, site: &Value) -> Result<Vec<Value>> {
        let key = format!("classes:{}:{}", self.sources.revision(), site);
        if let Some(value) = self.cache.read(&key) {
            return Ok(array(&value));
        }
        let this = self.clone();
        let site = site.clone();
        let cache_key = key.clone();
        let revision = self.sources.revision();
        Ok(array(&self.cache.coalesce(key,async move {
            let payload=if !s(&site,"spiderClass").is_empty(){this.spider.call(&site,"home",json!({"filter":true}),120.0,&CancellationToken::new()).await?}
                else{this.request_cms(&site,json!({"ac":"list"}),None).await?};
            if !cms::is_ok(&payload){bail!("接口返回异常：{}",s(&payload,"msg"));}
            let classes:Vec<_>=array(&payload["class"]).into_iter().filter_map(|v| {
                let id=v.get("type_id").or_else(||v.get("id")).or_else(||v.get("tid")).map(crate::model::text).unwrap_or_default();
                let name=v.get("type_name").or_else(||v.get("name")).or_else(||v.get("title")).map(crate::model::text).unwrap_or_default();
                let pid=v.get("type_pid").or_else(||v.get("pid")).map(crate::model::text).unwrap_or_else(||"0".into());
                if id.is_empty()||name.is_empty(){None}else{Some(json!({"id":id,"name":name,"pid":if pid.is_empty(){"0"}else{&pid}}))}
            }).collect();
            let value=json!(classes);if revision==this.sources.revision(){this.cache.insert(cache_key,value.clone(),Duration::from_secs(600),512);}Ok(value)
        }).await?))
    }

    pub async fn home(self: &Arc<Self>, force: bool) -> Result<Vec<Value>> {
        self.home_with_progress(force, None).await
    }

    pub async fn home_with_progress(
        self: &Arc<Self>,
        force: bool,
        on_progress: Option<LoadProgress>,
    ) -> Result<Vec<Value>> {
        let revision = self.sources.revision();
        let key = format!("home:{revision}");
        if !force && let Some(value) = self.cache.read(&key) {
            if let Some(callback) = &on_progress {
                callback(value.clone());
            }
            return Ok(array(&value));
        }
        let sites = self.sources.sites()?;
        let configs = self.sources.configs()?;
        let (mut plans, mut missing) = (Vec::new(), Vec::new());
        let mut seen = HashSet::new();
        for config in configs {
            for section in array(&config["home"]["sections"]) {
                let title = s(&section, "title");
                let reference = s(&section, "site");
                if title.is_empty() && reference.is_empty() {
                    continue;
                }
                let enabled: Vec<_> = sites.iter().filter(|v| flag(&v["enabled"], true)).collect();
                let found = enabled
                    .iter()
                    .copied()
                    .find(|v| s(v, "key") == reference)
                    .or_else(|| enabled.iter().copied().find(|v| s(v, "name") == reference))
                    .or_else(|| {
                        enabled.iter().copied().find(|v| {
                            s(v, "key").to_lowercase() == reference.to_lowercase()
                                || s(v, "name").to_lowercase() == reference.to_lowercase()
                        })
                    })
                    .or_else(|| {
                        enabled.iter().copied().find(|v| {
                            !reference.is_empty()
                                && (s(v, "name").contains(&reference)
                                    || reference.contains(&s(v, "name")))
                        })
                    });
                let Some(site) = found else {
                    missing.push(json!({"key":format!("missing:{reference}"),"title":if title.is_empty(){&reference}else{&title},"origin":"config","items":[],"error":format!("配置里写的影视源「{reference}」不在当前已启用的源里")}));
                    continue;
                };
                if !content_site(site) {
                    missing.push(json!({"key":format!("unsupported:{}",s(site,"key")),"title":title,"origin":"config","items":[],"error":format!("{} 不是可用的苹果CMS / Spider 源，首页内容位暂不支持",s(site,"name"))}));
                    continue;
                }
                let tid = s(&section, "typeId");
                let section_key = format!(
                    "{}::{}",
                    s(site, "key"),
                    if tid.is_empty() { "latest" } else { &tid }
                );
                if !seen.insert(section_key.clone()) {
                    continue;
                }
                plans.push(json!({"key":section_key,"title":if title.is_empty(){format!("{} · {}",s(site,"name"),if tid.is_empty(){"最近更新"}else{&tid})}else{title},"origin":"config","site":site,"typeId":tid}));
            }
        }
        if plans.is_empty() {
            let mut defaults: Vec<_> = sites
                .iter()
                .filter(|v| content_site(v) && s(v, "spiderClass").is_empty())
                .collect();
            if defaults.is_empty() {
                defaults = sites
                    .iter()
                    .filter(|v| content_site(v) && !s(v, "spiderClass").is_empty())
                    .take(8)
                    .collect();
            }
            for site in defaults {
                plans.push(json!({"key":format!("{}::latest",s(site,"key")),"title":format!("{} · 最近更新",s(site,"name")),"origin":"default","site":site,"typeId":""}));
            }
        }
        let mut sections: Vec<_> = plans.iter().map(|plan| {
            json!({"key":plan["key"],"title":plan["title"],"origin":plan["origin"],"siteKey":plan["site"]["key"],"siteName":plan["site"]["name"],"items":[],"loading":true})
        }).collect();
        sections.append(&mut missing);
        if let Some(callback) = &on_progress {
            callback(json!(sections));
        }
        let mut tasks = stream::iter(plans.into_iter().enumerate().map(
            |(index, plan)| async move {
                let key = format!("home-section:{revision}:{plan}");
                if !force && let Some(value) = self.cache.read(&key) {
                    return Ok((index, value));
                }
                let this = self.clone();
                let cache_key = key.clone();
                let value = self
                    .cache
                    .coalesce(key, async move {
                        let value = this.home_section(plan).await;
                        let ttl = if value.get("error").is_some() {
                            60
                        } else {
                            600
                        };
                        if revision == this.sources.revision() {
                            this.cache.insert(
                                cache_key,
                                value.clone(),
                                Duration::from_secs(ttl),
                                512,
                            );
                        }
                        Ok(value)
                    })
                    .await?;
                Ok::<_, anyhow::Error>((index, value))
            },
        ))
        .buffer_unordered(self.concurrency());
        while let Some(result) = tasks.next().await {
            let (index, section) = result?;
            sections[index] = section;
            if let Some(callback) = &on_progress {
                callback(json!(sections));
            }
        }
        let ttl = if sections.iter().any(|v| v.get("error").is_some()) {
            60
        } else {
            600
        };
        if revision == self.sources.revision() {
            self.cache
                .insert(key, json!(sections), Duration::from_secs(ttl), 512);
        }
        Ok(sections)
    }

    async fn home_section(self: &Arc<Self>, plan: Value) -> Value {
        let site = &plan["site"];
        let tid = s(&plan, "typeId");
        let mut out = json!({"key":plan["key"],"title":plan["title"],"origin":plan["origin"],"siteKey":site["key"],"siteName":site["name"],"items":[]});
        let result = async {
            let payload = if !s(site, "spiderClass").is_empty() {
                if tid.is_empty() {
                    self.spider
                        .call(
                            site,
                            "home",
                            json!({"filter":true}),
                            120.0,
                            &CancellationToken::new(),
                        )
                        .await?
                } else {
                    self.spider
                        .call(
                            site,
                            "category",
                            json!({"tid":tid,"pg":"1","filter":true,"extend":{}}),
                            120.0,
                            &CancellationToken::new(),
                        )
                        .await?
                }
            } else {
                let mut query = json!({"ac":"detail","pg":1});
                if !tid.is_empty() {
                    query["t"] = json!(tid);
                }
                self.request_cms(site, query, None).await?
            };
            if !cms::is_ok(&payload) {
                bail!("接口返回异常：{}", s(&payload, "msg"));
            }
            let items = cms::list(&payload, site, 24);
            if items.is_empty() {
                bail!("这一个内容位没有返回数据");
            }
            Ok::<_, anyhow::Error>(items)
        }
        .await;
        match result {
            Ok(items) => out["items"] = json!(items),
            Err(error) => out["error"] = json!(format!("{error:#}")),
        }
        out
    }
}

pub fn content_site(site: &Value) -> bool {
    flag(&site["enabled"], true)
        && s(site, "unsupported").is_empty()
        && (!s(site, "spiderClass").is_empty()
            || ([3, 4].contains(&site["type"].as_i64().unwrap_or(2)) && !s(site, "api").is_empty()))
}
pub fn fail(site: &Value, message: &str, ms: u64) -> Value {
    json!({"siteKey":site["key"],"siteName":site["name"],"ok":false,"ms":ms,"message":message,"list":[]})
}
pub fn success(site: &Value, list: Vec<Value>, ms: u64, verb: &str) -> Value {
    json!({"siteKey":site["key"],"siteName":site["name"],"ok":true,"ms":ms,"message":if list.is_empty(){"没有匹配结果".to_owned()}else{format!("{verb} {} 条",list.len())},"list":list})
}
pub fn interleave(lists: &[Vec<Value>]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for i in 0..lists.iter().map(Vec::len).max().unwrap_or(0) {
        for list in lists {
            if let Some(vod) = list.get(i)
                && seen.insert(s(vod, "vod_id"))
            {
                out.push(vod.clone());
            }
        }
    }
    out
}
