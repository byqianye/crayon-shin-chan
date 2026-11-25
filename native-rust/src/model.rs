use serde_json::Value;

pub fn text(value: &Value) -> String {
    match value {
        Value::String(s) => s.trim().to_owned(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

pub fn s(value: &Value, key: &str) -> String {
    text(&value[key])
}

pub fn array(value: &Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items.clone(),
        Value::Object(_) => vec![value.clone()],
        _ => Vec::new(),
    }
}

pub fn number(value: &Value, fallback: f64) -> f64 {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|n| n.is_finite())
        .unwrap_or(fallback)
}

pub fn flag(value: &Value, fallback: bool) -> bool {
    if value.is_null() {
        return fallback;
    }
    if let Some(v) = value.as_bool() {
        return v;
    }
    if let Some(v) = value.as_f64().or_else(|| value.as_str()?.parse().ok()) {
        return v != 0.0;
    }
    matches!(text(value).to_lowercase().as_str(), "true" | "yes")
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn merge(target: &mut Value, patch: &Value) {
    if let (Some(dst), Some(src)) = (target.as_object_mut(), patch.as_object()) {
        for (key, value) in src {
            if value.is_object() && dst.get(key).is_some_and(Value::is_object) {
                merge(dst.get_mut(key).expect("existing key"), value);
            } else {
                dst.insert(key.clone(), value.clone());
            }
        }
    }
}

pub fn compare_cn(left: &str, right: &str) -> std::cmp::Ordering {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn CompareStringEx(
                locale: *const u16,
                flags: u32,
                left: *const u16,
                left_len: i32,
                right: *const u16,
                right_len: i32,
                version: *const std::ffi::c_void,
                reserved: *mut std::ffi::c_void,
                param: isize,
            ) -> i32;
        }
        let locale = [122, 104, 45, 67, 78, 0];
        let left_utf16: Vec<_> = left.encode_utf16().collect();
        let right_utf16: Vec<_> = right.encode_utf16().collect();
        if let (Ok(left_len), Ok(right_len)) = (
            i32::try_from(left_utf16.len()),
            i32::try_from(right_utf16.len()),
        ) {
            // 两个 UTF-16 缓冲区在调用期间有效；长度明确，保留内嵌空字符。
            let result = unsafe {
                CompareStringEx(
                    locale.as_ptr(),
                    0,
                    left_utf16.as_ptr(),
                    left_len,
                    right_utf16.as_ptr(),
                    right_len,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    0,
                )
            };
            if result != 0 {
                return result.cmp(&2);
            }
        }
    }
    left.cmp(right)
}
