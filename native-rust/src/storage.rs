use crate::model::{merge, now, number, s};
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};
use unicode_normalization::UnicodeNormalization;

pub struct Store {
    pub root: PathBuf,
    cache: Mutex<HashMap<String, Cached>>,
}

struct Cached {
    value: Value,
    dirty: bool,
}

impl Store {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        fs::create_dir_all(root.as_ref()).context("无法创建用户数据目录")?;
        Ok(Self {
            root: root.as_ref().to_owned(),
            cache: Mutex::new(HashMap::new()),
        })
    }

    fn path(&self, name: &str) -> Result<PathBuf> {
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            bail!("非法数据文件名");
        }
        Ok(self.root.join(format!("{name}.json")))
    }

    fn load(
        &self,
        cache: &mut HashMap<String, Cached>,
        name: &str,
        fallback: Value,
    ) -> Result<Value> {
        let path = self.path(name)?;
        if let Some(value) = cache.get(name) {
            return Ok(value.value.clone());
        }
        let value = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(text.trim_start_matches('\u{feff}'))
                .with_context(|| format!("数据文件 {name}.json 损坏，已保留原文件"))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => fallback,
            Err(e) => return Err(e).with_context(|| format!("无法读取 {name}.json")),
        };
        cache.insert(
            name.to_owned(),
            Cached {
                value: value.clone(),
                dirty: false,
            },
        );
        Ok(value)
    }

    fn persist(
        &self,
        cache: &mut HashMap<String, Cached>,
        name: &str,
        value: &Value,
    ) -> Result<()> {
        let path = self.path(name)?;
        let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            use std::io::Write;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&serde_json::to_vec_pretty(value)?)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &path)?;
            Ok::<_, anyhow::Error>(())
        })();
        if let Err(error) = result {
            if temporary.exists() {
                fs::remove_file(&temporary).context("写入失败后清理临时文件失败")?;
            }
            return Err(error).with_context(|| format!("无法保存 {name}.json"));
        }
        cache.insert(
            name.to_owned(),
            Cached {
                value: value.clone(),
                dirty: false,
            },
        );
        Ok(())
    }

    pub fn read(&self, name: &str, fallback: Value) -> Result<Value> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| anyhow::anyhow!("数据锁异常"))?;
        self.load(&mut cache, name, fallback)
    }

    pub fn write(&self, name: &str, value: &Value) -> Result<()> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| anyhow::anyhow!("数据锁异常"))?;
        self.load(&mut cache, name, Value::Null)?;
        self.persist(&mut cache, name, value)
    }

    pub fn update<T>(
        &self,
        name: &str,
        fallback: Value,
        change: impl FnOnce(&mut Value) -> Result<T>,
    ) -> Result<T> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| anyhow::anyhow!("数据锁异常"))?;
        let mut next = self.load(&mut cache, name, fallback)?;
        let result = change(&mut next)?;
        self.persist(&mut cache, name, &next)?;
        Ok(result)
    }

    pub(crate) fn update_deferred<T>(
        &self,
        name: &str,
        fallback: Value,
        change: impl FnOnce(&mut Value) -> Result<T>,
    ) -> Result<T> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| anyhow::anyhow!("数据锁异常"))?;
        let mut next = self.load(&mut cache, name, fallback)?;
        let result = change(&mut next)?;
        cache.insert(
            name.to_owned(),
            Cached {
                value: next,
                dirty: true,
            },
        );
        Ok(result)
    }

    pub(crate) fn flush(&self, name: &str) -> Result<()> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| anyhow::anyhow!("数据锁异常"))?;
        if let Some(value) = cache
            .get(name)
            .filter(|entry| entry.dirty)
            .map(|entry| entry.value.clone())
        {
            self.persist(&mut cache, name, &value)?;
        }
        Ok(())
    }

    pub fn settings(&self) -> Result<Value> {
        let mut defaults = json!({"concurrency":8,"timeout":20,"userAgent":crate::http::DEFAULT_UA,"accent":"#2563EB","density":"compact","sidebarCollapsed":false,"filterCommentary":true,"externalPlayer":{"command":"","args":["{url}"],"prefer":["PotPlayer","mpv","VLC"]},"window":{"width":1440,"height":900},"ai":{"apiKey":"","model":"deepseek-flash","baseUrl":""}});
        merge(&mut defaults, &self.read("settings", json!({}))?);
        Ok(defaults)
    }

    pub fn save_settings(&self, patch: &Value) -> Result<Value> {
        if !patch.is_object() {
            bail!("设置必须为对象");
        }
        self.update("settings", json!({}), |next| {
            merge(next, patch);
            Ok(())
        })?;
        self.settings()
    }

    pub fn toggle_favorite(&self, mut item: Value) -> Result<bool> {
        validate_item(&item)?;
        self.update("favorites", json!([]), |value| {
            let items = value.as_array_mut().context("收藏数据不是数组")?;
            let key = item_key(&item);
            if let Some(index) = items.iter().position(|item| item_key(item) == key) {
                items.remove(index);
                Ok(false)
            } else {
                item["addedAt"] = json!(now());
                items.push(item);
                Ok(true)
            }
        })
    }

    pub fn record_history(&self, mut item: Value) -> Result<()> {
        validate_item(&item)?;
        if number(&item["position"], 0.0) < 0.0 || number(&item["duration"], 0.0) < 0.0 {
            bail!("播放进度不能为负数");
        }
        self.update("history", json!([]), |value| {
            let items = value.as_array_mut().context("历史数据不是数组")?;
            let key = item_key(&item);
            if let Some(index) = items.iter().position(|entry| item_key(entry) == key) {
                let previous = items.remove(index);
                if previous["lineIndex"] == item["lineIndex"]
                    && previous["epIndex"] == item["epIndex"]
                    && number(&item["duration"], 0.0) <= 0.0
                {
                    item["position"] = previous["position"].clone();
                    item["duration"] = previous["duration"].clone();
                }
            }
            item["updatedAt"] = json!(now());
            items.insert(0, item);
            items.sort_by_key(|entry| std::cmp::Reverse(entry["updatedAt"].as_u64().unwrap_or(0)));
            items.truncate(200);
            Ok(())
        })
    }

    pub fn remove_favorite(&self, key: &str) -> Result<()> {
        self.update("favorites", json!([]), |value| {
            value
                .as_array_mut()
                .context("收藏数据不是数组")?
                .retain(|item| item_key(item) != key);
            Ok(())
        })
    }

    pub fn remove_history(&self, key: &str) -> Result<()> {
        self.update("history", json!([]), |value| {
            value
                .as_array_mut()
                .context("历史数据不是数组")?
                .retain(|item| normalize_history_name(&s(item, "name")) != key);
            Ok(())
        })
    }

    pub fn library(&self) -> Result<Value> {
        let favorites = self.read("favorites", json!([]))?;
        let mut favorites = favorites.as_array().context("收藏数据不是数组")?.clone();
        favorites.sort_by_key(|item| std::cmp::Reverse(item["addedAt"].as_u64().unwrap_or(0)));
        let history = self.read("history", json!([]))?;
        let mut history = history.as_array().context("历史数据不是数组")?.clone();
        history.sort_by_key(|item| std::cmp::Reverse(item["updatedAt"].as_u64().unwrap_or(0)));
        let mut groups = Vec::<Value>::new();
        let mut indices = HashMap::<String, usize>::new();
        for item in history {
            let key = normalize_history_name(&s(&item, "name"));
            if let Some(index) = indices.get(&key) {
                groups[*index]["items"]
                    .as_array_mut()
                    .expect("group items")
                    .push(item);
            } else {
                indices.insert(key.clone(), groups.len());
                groups.push(json!({"key":key,"name":item["name"],"pic":item["pic"],"updatedAt":item["updatedAt"],"items":[item]}));
            }
        }
        Ok(json!({"favorites":favorites,"history":groups}))
    }
}

fn validate_item(item: &Value) -> Result<()> {
    if !item.is_object() || s(item, "siteKey").is_empty() || s(item, "vodId").is_empty() {
        bail!("影片缺少站点或影片标识");
    }
    Ok(())
}

pub fn item_key(item: &Value) -> String {
    format!("{}::{}", s(item, "siteKey"), s(item, "vodId"))
}

pub fn normalize_history_name(name: &str) -> String {
    static PUNCTUATION: OnceLock<Regex> = OnceLock::new();
    let normalized = name.nfkc().collect::<String>().to_lowercase();
    let key = PUNCTUATION
        .get_or_init(|| Regex::new(r"[\s\p{P}\p{S}]").expect("constant regex"))
        .replace_all(&normalized, "")
        .to_string();
    if key.is_empty() {
        name.trim().to_lowercase()
    } else {
        key
    }
}
