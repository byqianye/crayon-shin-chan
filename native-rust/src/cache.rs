use anyhow::Result;
use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use serde_json::Value;
use std::{
    collections::HashMap,
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

type Task = Shared<BoxFuture<'static, std::result::Result<Value, Arc<str>>>>;
struct Entry {
    at: Instant,
    ttl: Duration,
    value: Value,
}
#[derive(Default)]
pub struct Cache {
    entries: Mutex<HashMap<String, Entry>>,
    pending: Mutex<HashMap<String, (u64, Task)>>,
    next: AtomicU64,
}

impl Cache {
    pub fn read(&self, key: &str) -> Option<Value> {
        let mut entries = self.entries.lock().expect("cache lock");
        if entries.get(key).is_some_and(|v| v.at.elapsed() < v.ttl) {
            return entries.get(key).map(|v| v.value.clone());
        }
        entries.remove(key);
        None
    }
    pub fn insert(&self, key: String, value: Value, ttl: Duration, capacity: usize) {
        let mut entries = self.entries.lock().expect("cache lock");
        let now = Instant::now();
        entries.retain(|_, v| now.duration_since(v.at) < v.ttl);
        entries.insert(
            key,
            Entry {
                at: now,
                ttl,
                value,
            },
        );
        while entries.len() > capacity {
            if let Some(key) = entries
                .iter()
                .min_by_key(|(_, v)| v.at)
                .map(|(k, _)| k.clone())
            {
                entries.remove(&key);
            }
        }
    }
    pub fn clear(&self) {
        self.entries.lock().expect("cache lock").clear();
    }
    pub async fn coalesce(
        &self,
        key: String,
        fetch: impl Future<Output = Result<Value>> + Send + 'static,
    ) -> Result<Value> {
        let (id, task) = {
            let mut pending = self.pending.lock().expect("pending lock");
            pending
                .entry(key.clone())
                .or_insert_with(|| {
                    let id = self.next.fetch_add(1, Ordering::AcqRel);
                    let task =
                        async move { fetch.await.map_err(|e| Arc::<str>::from(format!("{e:#}"))) }
                            .boxed()
                            .shared();
                    (id, task)
                })
                .clone()
        };
        let result = task.await;
        let mut pending = self.pending.lock().expect("pending lock");
        if pending.get(&key).is_some_and(|v| v.0 == id) {
            pending.remove(&key);
        }
        result.map_err(|e| anyhow::anyhow!(e.to_string()))
    }
}
