use crate::{
    config::{
        host_name, load_payload, multi_refs, normalize_site, normalize_url, parse_config,
        short_hash,
    },
    http::{Net, is_http_url, match_headers},
    model::{array, flag, merge, now, number, s},
    storage::Store,
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio_util::sync::CancellationToken;

struct SyncSlot {
    gate: tokio::sync::Mutex<()>,
    completion: AtomicU64,
    result: Mutex<Option<std::result::Result<Value, String>>>,
}

pub struct Sources {
    pub store: Arc<Store>,
    pub net: Net,
    parsed: RwLock<HashMap<String, Value>>,
    syncing: Mutex<HashMap<String, Arc<SyncSlot>>>,
    revision: AtomicU64,
}

impl Sources {
    pub fn new(store: Arc<Store>) -> Result<Self> {
        let this = Self {
            store,
            net: Net::new()?,
            parsed: RwLock::new(HashMap::new()),
            syncing: Mutex::new(HashMap::new()),
            revision: AtomicU64::new(1),
        };
        let mut cache = this
            .parsed
            .write()
            .map_err(|_| anyhow::anyhow!("配置缓存锁异常"))?;
        for source in this.list()? {
            if source["raw"].is_null() {
                continue;
            }
            let parsed = if source["kind"] == "multirepo" {
                source["raw"].clone()
            } else {
                parse_config(
                    &source["raw"],
                    &s(&source, "id"),
                    &s(&source, "name"),
                    &s(&source, "url"),
                    number(&this.store.settings()?["timeout"], 20.0),
                )
            };
            cache.insert(s(&source, "id"), parsed);
        }
        drop(cache);
        Ok(this)
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }
    pub fn invalidate(&self) {
        self.revision.fetch_add(1, Ordering::AcqRel);
    }
    pub fn list(&self) -> Result<Vec<Value>> {
        Ok(self
            .store
            .read("sources", json!([]))?
            .as_array()
            .context("配置源数据不是数组")?
            .clone())
    }
    pub fn parsed(&self, id: &str) -> Result<Option<Value>> {
        Ok(self
            .parsed
            .read()
            .map_err(|_| anyhow::anyhow!("配置缓存锁异常"))?
            .get(id)
            .cloned())
    }
    pub fn configs(&self) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        for source in self
            .list()?
            .into_iter()
            .filter(|v| flag(&v["enabled"], true))
        {
            if let Some(parsed) = self.parsed(&s(&source, "id"))? {
                out.push(parsed);
            }
        }
        Ok(out)
    }

    pub fn add(&self, input: &Value) -> Result<Value> {
        let address = normalize_url(&s(input, "url"));
        if address.is_empty() {
            bail!("地址不能为空");
        }
        let name = s(input, "name");
        let kind = if is_http_url(&address) { "url" } else { "file" };
        let source = json!({"id":uuid::Uuid::new_v4(),"name":if name.is_empty(){host_name(&address)}else{name},"url":address,"kind":kind,"enabled":true,"lastSync":0,"siteCount":0,"liveCount":0,"parseCount":0});
        self.store.update("sources", json!([]), |list| {
            let list = list.as_array_mut().context("配置源数据不是数组")?;
            if list.iter().any(|v| normalize_url(&s(v, "url")) == address) {
                bail!("该配置源已存在");
            }
            list.push(source.clone());
            Ok(())
        })?;
        self.tombstone("sourceUrls", &address, false)?;
        self.invalidate();
        Ok(source)
    }

    pub fn update(&self, id: &str, patch: &Value) -> Result<Value> {
        let mut address_changed = false;
        let next = self.store.update("sources", json!([]), |list| {
            let item = list
                .as_array_mut()
                .context("配置源数据不是数组")?
                .iter_mut()
                .find(|v| s(v, "id") == id)
                .context("配置源不存在")?;
            let old = s(item, "url");
            for key in ["name", "enabled"] {
                if let Some(value) = patch.get(key) {
                    item[key] = value.clone();
                }
            }
            if let Some(value) = patch.get("url") {
                let address = normalize_url(&crate::model::text(value));
                if address.is_empty() {
                    bail!("地址不能为空");
                }
                if address != old {
                    address_changed = true;
                    for key in ["repos", "raw", "error", "notice"] {
                        item.as_object_mut().context("配置源格式无效")?.remove(key);
                    }
                    for key in [
                        "lastSync",
                        "siteCount",
                        "usableSiteCount",
                        "liveCount",
                        "parseCount",
                    ] {
                        item[key] = json!(0);
                    }
                    item["kind"] = json!(if is_http_url(&address) { "url" } else { "file" });
                    item["url"] = json!(address);
                }
            }
            Ok(item.clone())
        })?;
        if address_changed {
            self.parsed
                .write()
                .map_err(|_| anyhow::anyhow!("配置缓存锁异常"))?
                .remove(id);
        }
        self.invalidate();
        Ok(next)
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        let mut address = String::new();
        self.store.update("sources", json!([]), |list| {
            let list = list.as_array_mut().context("配置源数据不是数组")?;
            if let Some(item) = list.iter().find(|v| s(v, "id") == id) {
                address = s(item, "url");
            }
            list.retain(|v| s(v, "id") != id);
            Ok(())
        })?;
        self.tombstone("sourceUrls", &address, true)?;
        self.parsed
            .write()
            .map_err(|_| anyhow::anyhow!("配置缓存锁异常"))?
            .remove(id);
        self.invalidate();
        Ok(())
    }

    pub async fn sync(&self, id: &str) -> Result<Value> {
        let slot = {
            let mut slots = self
                .syncing
                .lock()
                .map_err(|_| anyhow::anyhow!("同步锁异常"))?;
            slots
                .entry(id.to_owned())
                .or_insert_with(|| {
                    Arc::new(SyncSlot {
                        gate: tokio::sync::Mutex::new(()),
                        completion: AtomicU64::new(0),
                        result: Mutex::new(None),
                    })
                })
                .clone()
        };
        let completion = slot.completion.load(Ordering::Acquire);
        let _guard = slot.gate.lock().await;
        if slot.completion.load(Ordering::Acquire) != completion
            && let Some(result) = slot
                .result
                .lock()
                .map_err(|_| anyhow::anyhow!("同步锁异常"))?
                .clone()
        {
            return result.map_err(anyhow::Error::msg);
        }
        let result = self.fetch(id).await.map_err(|error| format!("{error:#}"));
        *slot
            .result
            .lock()
            .map_err(|_| anyhow::anyhow!("同步锁异常"))? = Some(result.clone());
        slot.completion.fetch_add(1, Ordering::AcqRel);
        result.map_err(anyhow::Error::msg)
    }

    async fn fetch(&self, id: &str) -> Result<Value> {
        let source = self
            .list()?
            .into_iter()
            .find(|v| s(v, "id") == id)
            .context("配置源不存在")?;
        let address = s(&source, "url");
        let name = s(&source, "name");
        let timeout = number(&self.store.settings()?["timeout"], 20.0);
        let cancel = CancellationToken::new();
        let fetched=async{
            let raw=load_payload(&self.net,&address,timeout,&cancel).await?;
            let mut parsed=parse_config(&raw,id,&name,&address,timeout);
            let refs=multi_refs(&raw,&address);
            let mut update=json!({});
            if !refs.is_empty()&&array(&parsed["sites"]).is_empty(){
                let mut combined=json!({"name":name,"sites":[],"parses":[],"lives":[],"headers":[],"flags":[],"home":{"sections":[]}});
                let mut repos=Vec::new();let mut flags=HashSet::new();
                for reference in refs{
                    let address=s(&reference,"url");let child_name=s(&reference,"name");
                    match load_payload(&self.net,&address,timeout,&cancel).await{
                        Ok(raw)=>{
                            let child=parse_config(&raw,id,&child_name,&address,timeout);
                            for key in ["sites","parses","lives","headers"]{combined[key].as_array_mut().expect("combined array").extend(array(&child[key]));}
                            combined["home"]["sections"].as_array_mut().expect("sections array").extend(array(&child["home"]["sections"]));
                            for value in array(&child["flags"]){if flags.insert(crate::model::text(&value)){combined["flags"].as_array_mut().expect("flags array").push(value);}}
                            repos.push(json!({"name":child_name,"url":address,"ok":true,"siteCount":array(&child["sites"]).len()}));
                        }
                        Err(error)=>repos.push(json!({"name":child_name,"url":address,"ok":false,"error":format!("{error:#}")})),
                    }
                }
                if repos.iter().all(|v|v["ok"]==false){bail!("多仓的所有子仓库均同步失败");}
                update["kind"]=json!("multirepo");update["repos"]=json!(repos);update["raw"]=combined.clone();
                parsed=combined;
            }else{update["kind"]=json!(if s(&source,"kind")=="file"{"file"}else{"url"});update["repos"]=Value::Null;update["raw"]=raw;}
            update["siteCount"]=json!(array(&parsed["sites"]).len());
            update["usableSiteCount"]=json!(array(&parsed["sites"]).iter().filter(|v|s(v,"unsupported").is_empty()).count());
            update["liveCount"]=json!(array(&parsed["lives"]).len());update["parseCount"]=json!(array(&parsed["parses"]).len());
            update["lastSync"]=json!(now());update["error"]=Value::Null;update["notice"]=parsed["notice"].clone();
            Ok::<_,anyhow::Error>((parsed,update))
        }.await;
        let (mut parsed, update) = match fetched {
            Ok((parsed, update)) => (Some(parsed), update),
            Err(error) => (None, json!({"error":format!("{error:#}")})),
        };
        // Store 的一次事务只合并当前记录，避免网络响应复活已删除的源或覆盖启停状态。
        let next = self.store.update("sources", json!([]), |list| {
            let item = list
                .as_array_mut()
                .context("配置源数据不是数组")?
                .iter_mut()
                .find(|v| s(v, "id") == id)
                .context("配置源不存在")?;
            if s(item, "url") != address {
                bail!("配置源地址已修改，请重新同步");
            }
            if let Some(parsed) = parsed.as_mut() {
                if let Some(sites) = parsed["sites"].as_array_mut() {
                    for site in sites {
                        if s(site, "sourceName") == name {
                            site["sourceName"] = item["name"].clone();
                        }
                    }
                }
                self.parsed
                    .write()
                    .map_err(|_| anyhow::anyhow!("配置缓存锁异常"))?
                    .insert(id.to_owned(), parsed.clone());
            }
            merge(item, &update);
            Ok(item.clone())
        })?;
        self.invalidate();
        Ok(next)
    }

    pub async fn sync_all(&self) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        for source in self
            .list()?
            .into_iter()
            .filter(|v| flag(&v["enabled"], true))
        {
            out.push(self.sync(&s(&source, "id")).await?);
        }
        Ok(out)
    }

    pub fn tombstones(&self) -> Result<Value> {
        self.store
            .read("tombstones", json!({"sourceUrls":[],"siteKeys":[]}))
    }
    pub fn tombstone(&self, kind: &str, key: &str, add: bool) -> Result<()> {
        if key.is_empty() {
            return Ok(());
        }
        if !matches!(kind, "sourceUrls" | "siteKeys") {
            bail!("墓碑类型无效");
        }
        self.store.update(
            "tombstones",
            json!({"sourceUrls":[],"siteKeys":[]}),
            |value| {
                let list = value[kind].as_array_mut().context("墓碑数据格式无效")?;
                list.retain(|v| v.as_str() != Some(key));
                if add {
                    list.push(json!(key));
                }
                Ok(())
            },
        )?;
        self.invalidate();
        Ok(())
    }
    pub fn clear_tombstones(&self) -> Result<()> {
        self.store
            .write("tombstones", &json!({"sourceUrls":[],"siteKeys":[]}))?;
        self.invalidate();
        Ok(())
    }
    pub fn local_sites(&self) -> Result<Vec<Value>> {
        Ok(self
            .store
            .read("local-sites", json!([]))?
            .as_array()
            .context("手动源数据不是数组")?
            .clone())
    }
    pub async fn detect(&self, address: &str) -> Result<Value> {
        let address = address.trim();
        if address.is_empty() {
            bail!("请先填写接口地址");
        }
        if is_http_url(address) {
            let response = self
                .net
                .get(
                    address,
                    &json!({"User-Agent":self.store.settings()?["userAgent"]}),
                    number(&self.store.settings()?["timeout"], 20.0),
                    &CancellationToken::new(),
                )
                .await?;
            Ok(crate::config::detect(
                &response.bytes,
                response
                    .headers
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or(""),
            ))
        } else {
            let bytes = tokio::fs::read(address)
                .await
                .context("无法读取待识别的源")?;
            Ok(crate::config::detect(&bytes, ""))
        }
    }
    pub fn add_local(&self, input: &Value) -> Result<Value> {
        let address = normalize_url(&s(input, "api"));
        if address.is_empty() {
            bail!("接口地址不能为空");
        }
        let key = format!("manual_{}", short_hash(&address));
        let mut raw = input.clone();
        raw["key"] = json!(key);
        raw["api"] = json!(address);
        if s(&raw, "name").is_empty() {
            raw["name"] = json!(host_name(&address));
        }
        let mut site = normalize_site(&raw, "local", "手动添加", "", "", &json!({}))
            .context("影视源格式无效")?;
        site["createdAt"] = json!(now());
        site["updatedAt"] = json!(now());
        self.store.update("local-sites", json!([]), |list| {
            let list = list.as_array_mut().context("手动源数据不是数组")?;
            if list.iter().any(|v| normalize_url(&s(v, "api")) == address) {
                bail!("该影视源已存在");
            }
            list.push(site.clone());
            Ok(())
        })?;
        self.invalidate();
        Ok(site)
    }
    pub fn update_local(&self, key: &str, patch: &Value) -> Result<Value> {
        let next = self.store.update("local-sites", json!([]), |list| {
            let list = list.as_array_mut().context("手动源数据不是数组")?;
            let index = list
                .iter()
                .position(|v| s(v, "key") == key)
                .context("影视源不存在")?;
            let address = patch
                .get("api")
                .map(|v| normalize_url(&crate::model::text(v)))
                .unwrap_or_else(|| s(&list[index], "api"));
            if address.is_empty() {
                bail!("接口地址不能为空");
            }
            if list
                .iter()
                .enumerate()
                .any(|(i, v)| i != index && normalize_url(&s(v, "api")) == address)
            {
                bail!("该影视源已存在");
            }
            let mut raw = list[index].clone();
            merge(&mut raw, patch);
            raw["api"] = json!(address);
            raw["key"] = if address == s(&list[index], "api") {
                list[index]["key"].clone()
            } else {
                json!(format!("manual_{}", short_hash(&address)))
            };
            if !s(&list[index], "unsupported").is_empty() && patch.get("searchable").is_none() {
                raw["searchable"] = json!(true);
                raw["quickSearch"] = json!(true);
            }
            let mut next = normalize_site(&raw, "local", "手动添加", "", "", &json!({}))
                .context("影视源格式无效")?;
            next["createdAt"] = list[index]["createdAt"].clone();
            next["updatedAt"] = json!(now());
            list[index] = next.clone();
            Ok(next)
        })?;
        self.invalidate();
        Ok(next)
    }
    pub fn remove_local(&self, key: &str) -> Result<()> {
        self.store.update("local-sites", json!([]), |list| {
            list.as_array_mut()
                .context("手动源数据不是数组")?
                .retain(|v| s(v, "key") != key);
            Ok(())
        })?;
        self.invalidate();
        Ok(())
    }
    pub fn override_site(&self, keys: &[String], patch: &Value) -> Result<()> {
        self.store.update("sites", json!({}), |map| {
            if !map.is_object() {
                bail!("影视源覆盖数据不是对象");
            }
            for key in keys {
                if !map[key].is_object() {
                    map[key] = json!({});
                }
                merge(&mut map[key], patch);
            }
            Ok(())
        })?;
        self.invalidate();
        Ok(())
    }

    pub fn sites(&self) -> Result<Vec<Value>> {
        let overrides = self.store.read("sites", json!({}))?;
        let ignored: HashSet<String> = array(&self.tombstones()?["siteKeys"])
            .iter()
            .map(crate::model::text)
            .collect();
        let mut seen = HashSet::new();
        let mut manual_apis = HashSet::new();
        let mut out = Vec::new();
        let mut push = |mut site: Value| {
            let key = s(&site, "key");
            if ignored.contains(&key) || !seen.insert(key.clone()) {
                return;
            }
            let patch = &overrides[&key];
            for field in ["enabled", "name", "timeout", "health"] {
                if let Some(value) = patch.get(field) {
                    site[field] = value.clone();
                }
            }
            if !site["header"].is_object() {
                site["header"] = json!({});
            }
            merge(&mut site["header"], &patch["header"]);
            out.push(site);
        };
        for mut local in self
            .local_sites()?
            .into_iter()
            .filter(|v| flag(&v["enabled"], true))
        {
            manual_apis.insert(normalize_url(&s(&local, "api")));
            local["quickSearch"] = local["searchable"].clone();
            local["sourceId"] = json!("local");
            local["sourceName"] = json!("手动添加");
            local["origin"] = json!("manual");
            push(local);
        }
        for config in self.configs()? {
            for mut site in array(&config["sites"]) {
                if manual_apis.contains(&normalize_url(&s(&site, "api"))) {
                    continue;
                }
                site["origin"] = json!("config");
                push(site);
            }
        }
        out.sort_by(|a, b| {
            number(&overrides[s(a, "key")]["order"], f64::MAX)
                .total_cmp(&number(&overrides[s(b, "key")]["order"], f64::MAX))
                .then_with(|| crate::model::compare_cn(&s(a, "name"), &s(b, "name")))
        });
        Ok(out)
    }
    pub fn site(&self, key: &str) -> Result<Value> {
        self.sites()?
            .into_iter()
            .find(|v| s(v, "key") == key)
            .context("站点不存在")
    }
    pub fn headers(&self, site: &Value, address: &str) -> Result<Value> {
        let rules: Vec<Value> = self
            .configs()?
            .iter()
            .flat_map(|c| array(&c["headers"]))
            .collect();
        let mut header = json!({"User-Agent":self.store.settings()?["userAgent"]});
        merge(&mut header, &match_headers(address, &rules));
        merge(&mut header, &site["header"]);
        Ok(header)
    }
}
