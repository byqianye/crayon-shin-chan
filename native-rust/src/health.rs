use crate::{
    model::{array, now, number},
    storage::Store,
};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

pub fn cooling(health: &Value) -> bool {
    number(&health["cooldownUntil"], 0.0) > now() as f64
}
pub fn summary(health: &Value) -> Value {
    let all = array(&health["samples"]);
    let samples = &all[all.len().saturating_sub(20)..];
    if samples.is_empty() {
        return json!({"score":0.5,"successRate":0.5,"avgMs":0,"cooling":cooling(health),"samples":0});
    }
    let ok: Vec<_> = samples.iter().filter(|v| v["ok"] == true).collect();
    let rate = ok.len() as f64 / samples.len() as f64;
    let latency = if ok.is_empty() {
        3000.0
    } else {
        ok.iter().map(|v| number(&v["ms"], 0.0)).sum::<f64>() / ok.len() as f64
    };
    let last = number(&samples.last().expect("samples")["at"], 0.0);
    let freshness = (1.0 - (now() as f64 - last) / 3_600_000.0).max(0.0);
    let score =
        (0.6 * rate + 0.3 * (1.0 - latency.min(3000.0) / 3000.0) + 0.1 * freshness).clamp(0.0, 1.0);
    json!({"score":score,"successRate":rate,"avgMs":latency.round(),"cooling":cooling(health),"samples":samples.len()})
}
pub fn record(store: &Store, key: &str, ok: bool, ms: u64) -> Result<Value> {
    store.update("health", json!({}), |all| apply(all, key, ok, ms, now()))
}

fn apply(all: &mut Value, key: &str, ok: bool, ms: u64, at: u64) -> Result<Value> {
    if !all[key].is_object() {
        all[key] = json!({"siteKey":key,"samples":[],"consecutiveFails":0});
    }
    let health = &mut all[key];
    let samples = health["samples"]
        .as_array_mut()
        .context("健康样本数据格式无效")?;
    samples.push(json!({"ok":ok,"ms":ms,"at":at}));
    if samples.len() > 50 {
        samples.drain(..samples.len() - 50);
    }
    if ok {
        health["consecutiveFails"] = json!(0);
        health["lastOkAt"] = json!(at);
        health
            .as_object_mut()
            .expect("health")
            .remove("cooldownUntil");
    } else {
        let failures = number(&health["consecutiveFails"], 0.0) + 1.0;
        health["consecutiveFails"] = json!(failures);
        health["lastFailAt"] = json!(at);
        if failures >= 3.0 {
            health["cooldownUntil"] = json!(at + 600_000);
        }
    }
    Ok(health.clone())
}

// 聚合搜索合并短时间内的写盘；完成、取消和 future 被丢弃时都保存已完成样本。
pub struct Batch<'a> {
    store: &'a Store,
    flushed_at: Mutex<Instant>,
}

impl<'a> Batch<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self {
            store,
            flushed_at: Mutex::new(Instant::now()),
        }
    }

    pub fn record(&self, key: &str, ok: bool, ms: u64) -> Result<()> {
        self.store.update_deferred("health", json!({}), |all| {
            apply(all, key, ok, ms, now())?;
            Ok(())
        })?;
        let mut flushed_at = self
            .flushed_at
            .lock()
            .map_err(|_| anyhow::anyhow!("健康样本锁异常"))?;
        if flushed_at.elapsed() >= Duration::from_millis(200) {
            self.store.flush("health")?;
            *flushed_at = Instant::now();
        }
        Ok(())
    }

    pub fn flush(&self) -> Result<()> {
        self.store.flush("health")
    }
}

impl Drop for Batch<'_> {
    fn drop(&mut self) {
        if let Err(error) = self.flush() {
            eprintln!("[health] 保存搜索样本失败：{error:#}");
        }
    }
}
