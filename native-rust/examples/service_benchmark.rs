use anyhow::{Context, Result, ensure};
use crayon_core::{
    cache::Cache,
    catalogue::Catalogue,
    dedupe, health,
    model::{array, compare_cn, s},
    sources::Sources,
    storage::Store,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn measure(mut operation: impl FnMut() -> Result<()>) -> Result<Vec<f64>> {
    (0..10)
        .map(|_| {
            let started = Instant::now();
            operation()?;
            Ok(started.elapsed().as_secs_f64() * 1000.0)
        })
        .collect()
}

fn components(input: &Value, directory: &Path) -> Result<Value> {
    let health_store = Store::open(directory.join("health-profile"))?;
    let health_ms = measure(|| {
        for index in 0..32 {
            health::record(&health_store, &format!("site-{index}"), true, 25)?;
        }
        Ok(())
    })?;
    let health_batched_ms = measure(|| {
        let batch = health::Batch::new(&health_store);
        for index in 0..32 {
            batch.record(&format!("site-{index}"), true, 25)?;
        }
        batch.flush()
    })?;
    let items = array(&input["merge"][0]["list"]);
    let cache = Cache::default();
    let cache_ms = measure(|| {
        for site in 0..32 {
            for item in items.iter().take(100) {
                cache.insert(
                    format!("detail:{site}:{}", item["vod_id"]),
                    item.clone(),
                    Duration::from_secs(120),
                    256,
                );
            }
        }
        Ok(())
    })?;
    let identity_ms = measure(|| {
        std::hint::black_box(items.iter().map(dedupe::key).collect::<Vec<_>>());
        Ok(())
    })?;
    let names: Vec<_> = items.iter().map(|item| s(item, "vod_name")).collect();
    let sort_ms = measure(|| {
        let mut sorted = names.clone();
        sorted.sort_by(|a, b| compare_cn(a, b));
        std::hint::black_box(sorted);
        Ok(())
    })?;
    Ok(
        json!({"healthRecords32":health_ms,"healthBatched32":health_batched_ms,"cacheInserts3200":cache_ms,"identities1200":identity_ms,"chineseSort1200":sort_ms}),
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        arguments.len() == 2,
        "用法：service_benchmark <scenario.json> <独立数据目录>"
    );
    let input: Value = serde_json::from_slice(&std::fs::read(&arguments[0])?)?;
    let store = Arc::new(Store::open(&arguments[1])?);
    ensure!(
        store.read("sources", json!([]))? == json!([]),
        "基准测试必须使用空的独立数据目录"
    );
    store.write("sources", &input["sources"])?;
    store.save_settings(&json!({"concurrency":8,"timeout":5}))?;
    let catalogue = Arc::new(Catalogue::new(Arc::new(Sources::new(store)?)));
    let mut samples = Vec::new();
    for round in 0..10 {
        let started = Instant::now();
        let first = Arc::new(Mutex::new(None));
        let result_first = first.clone();
        let response = catalogue
            .search(
                &format!("fixture-{round}"),
                json!({}),
                Some(Arc::new(move |_, _, _| {
                    result_first
                        .lock()
                        .expect("benchmark first result")
                        .get_or_insert_with(|| started.elapsed().as_secs_f64() * 1000.0);
                })),
            )
            .await?;
        let total = started.elapsed().as_secs_f64() * 1000.0;
        let results = array(&response["results"]);
        ensure!(
            results.len() == input["sites"].as_u64().context("sites 无效")? as usize
                && results.iter().all(|v| v["ok"] == true),
            "搜索结果不完整"
        );
        let mut identity: Vec<_> = results
            .iter()
            .flat_map(|v| array(&v["list"]))
            .map(|v| {
                vec![
                    v["siteKey"].clone(),
                    v["vod_id"].clone(),
                    v["vod_name"].clone(),
                ]
            })
            .collect();
        identity.sort_by_cached_key(|v| serde_json::to_string(v).expect("benchmark identity"));
        ensure!(
            identity.len() == input["items"].as_u64().context("items 无效")? as usize,
            "影片数量不一致"
        );
        let digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&identity)?));
        samples.push(json!({"first":*first.lock().expect("benchmark first result"),"total":total,"digest":digest}));
    }
    let mut merges = Vec::new();
    for known in [false, true] {
        let mut results = input["merge"].clone();
        if known {
            for vod in results[0]["list"].as_array_mut().context("merge 无效")? {
                vod["vod_year"] = json!("2026");
            }
        }
        let mut timings = Vec::new();
        let mut digest = String::new();
        for _ in 0..10 {
            let start = Instant::now();
            let merged = dedupe::merge_results(&array(&results), "", true, None);
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            ensure!(array(&merged["merged"]).len() == 1200, "去重改变了影片数量");
            let identity: Vec<_> = merged["merged"]
                .as_array()
                .context("merged 无效")?
                .iter()
                .map(|v| {
                    json!([
                        v["siteKey"],
                        v["vod_id"],
                        v["vod_name"],
                        v["allSiteKeys"],
                        v["alternatives"]
                    ])
                })
                .collect();
            let current = format!("{:x}", Sha256::digest(serde_json::to_vec(&identity)?));
            ensure!(digest.is_empty() || digest == current, "去重结果不稳定");
            digest = current;
            timings.push(ms);
        }
        merges.push(json!({"knownYear":known,"ms":timings,"digest":digest}));
    }
    let contracts = array(&input["contracts"]);
    for (index, contract) in contracts.iter().enumerate() {
        let actual = dedupe::merge_results(
            &array(&contract["results"]),
            contract["keyword"].as_str().unwrap_or_default(),
            true,
            None,
        );
        ensure!(
            actual == contract["expected"],
            "兼容样本 {index} 的完整去重结果不一致：{actual}"
        );
    }
    let profile = components(&input, Path::new(&arguments[1]))?;
    println!(
        "{}",
        json!({"search":samples,"merge":merges,"contracts":contracts.len(),"components":profile})
    );
    Ok(())
}
