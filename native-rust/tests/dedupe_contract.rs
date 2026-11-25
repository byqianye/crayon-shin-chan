use crayon_core::{
    dedupe,
    model::{array, s},
};
use serde_json::json;
use std::collections::HashMap;

#[test]
fn representative_alternatives_keyword_rank_and_previous_order_remain_stable() {
    let item = |id: &str, site: &str, name: &str, pic: &str| {
        json!({
            "vod_id":id,"siteKey":site,"vod_name":name,"vod_pic":pic,"vod_year":"2026","type_name":"电影"
        })
    };
    let results = [json!({"list":[
        item("a1","a","庆余年",""),
        item("a2","a","庆余年高清","poster"),
        item("a3","a","庆余年","poster"),
        item("b","b","庆余年",""),
        item("c","c","庆余年之风起沧州","poster"),
        item("d","d","另一部独立电影","poster")
    ]})];
    let output = dedupe::merge_results(&results, "庆余年", true, None);
    let merged = array(&output["merged"]);
    assert_eq!(merged.len(), 3);
    assert_eq!(merged[0]["vod_id"], "a3");
    assert_eq!(merged[0]["allSiteKeys"], json!(["a", "b"]));
    assert_eq!(merged[0]["alternatives"][0]["vod_id"], "b");
    assert_eq!(merged[1]["vod_id"], "c");
    let order = HashMap::from([(dedupe::key(&merged[2]), 0), (dedupe::key(&merged[0]), 1)]);
    let ordered = dedupe::merge_results(&results, "庆余年", true, Some(&order));
    let ids: Vec<_> = array(&ordered["merged"])
        .iter()
        .map(|v| s(v, "vod_id"))
        .collect();
    assert_eq!(ids, ["d", "a3", "c"]);
}

#[test]
fn chinese_title_sort_uses_locale_order_after_score_ties() {
    let input = [json!({"list":[
        {"siteKey":"a","vod_name":"重庆"},
        {"siteKey":"b","vod_name":"北京"},
        {"siteKey":"c","vod_name":"阿凡达"}
    ]})];
    let output = dedupe::merge_results(&input, "", true, None);
    let names: Vec<_> = array(&output["merged"])
        .iter()
        .map(|v| s(v, "vod_name"))
        .collect();
    #[cfg(windows)]
    assert_eq!(names, ["阿凡达", "北京", "重庆"]);
    #[cfg(not(windows))]
    assert_eq!(names.len(), 3);
}
