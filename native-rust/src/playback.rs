use crate::{
    model::{array, number, s},
    storage::Store,
};
use anyhow::Result;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

pub struct Playback {
    store: Arc<Store>,
    active: Mutex<Option<Session>>,
}

struct Session {
    id: String,
    entry: Value,
    position: f64,
    duration: f64,
    observed: bool,
    resume: Option<f64>,
    seeking: bool,
    capturing: bool,
    last_saved: Option<u64>,
    recoveries: u8,
}

#[derive(Default, Debug)]
pub struct Outcome {
    pub event: Option<Value>,
    pub action: Option<Value>,
    pub saved: bool,
}

impl Playback {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            store,
            active: Mutex::new(None),
        }
    }

    pub fn attach(&self, id: &str, entry: Value) -> Result<()> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| anyhow::anyhow!("播放状态锁异常"))?;
        if let Some(previous) = active.as_ref() {
            self.save(previous)?;
        }
        let previous = array(&self.store.read("history", json!([]))?)
            .into_iter()
            .find(|v| {
                v["siteKey"] == entry["siteKey"]
                    && v["vodId"] == entry["vodId"]
                    && v["lineIndex"] == entry["lineIndex"]
                    && v["epIndex"] == entry["epIndex"]
            });
        let position = previous
            .as_ref()
            .map(|v| positive(&v["position"]))
            .unwrap_or(0.0);
        let duration = previous
            .as_ref()
            .map(|v| positive(&v["duration"]))
            .unwrap_or(0.0);
        *active = Some(Session {
            id: id.into(),
            entry,
            position,
            duration,
            observed: false,
            resume: (position > 0.0 && position < duration).then_some(position),
            seeking: false,
            capturing: false,
            last_saved: None,
            recoveries: 0,
        });
        Ok(())
    }

    pub fn event(&self, event: &Value, at: u64) -> Result<Outcome> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| anyhow::anyhow!("播放状态锁异常"))?;
        let Some(state) = active.as_mut().filter(|v| v.id == s(event, "session")) else {
            return Ok(Outcome::default());
        };
        let kind = s(event, "event");
        let mut published = event.clone();
        published["at"] = json!(at);
        let mut out = Outcome {
            event: Some(published),
            ..Outcome::default()
        };
        if kind == "captureStart" {
            let duration = positive(&event["duration"]);
            if duration > 0.0 && !state.seeking {
                state.position = positive(&event["position"]).min(duration);
                state.duration = duration;
                state.observed = true;
            }
            state.capturing = true;
            return Ok(out);
        }
        if kind == "captureEnd" {
            state.capturing = false;
            return Ok(out);
        }
        if kind == "resumed" {
            state.seeking = false;
            state.resume = None;
        }
        if kind == "hlsError" {
            if event["fatal"] == true && state.recoveries < 2 {
                let action = match s(event, "errorType").as_str() {
                    "networkError" => Some("startLoad"),
                    "mediaError" => Some("recoverMediaError"),
                    _ => None,
                };
                if let Some(action) = action {
                    state.recoveries += 1;
                    out.action = Some(json!({"recover":action}));
                    out.event.as_mut().expect("event")["recovered"] = json!(true);
                }
            }
            return Ok(out);
        }
        let duration = positive(&event["duration"]);
        let position = positive(&event["position"]);
        if !state.capturing && event["capturing"] != true && duration > 0.0 {
            if ["loadedmetadata", "durationchange", "canplay", "timeupdate"]
                .contains(&kind.as_str())
                && let Some(resume) = state.resume.take().filter(|v| *v < duration)
            {
                state.seeking = true;
                out.action = Some(json!({"seek":resume}));
                return Ok(out);
            }
            if !state.seeking
                && [
                    "loadedmetadata",
                    "durationchange",
                    "canplay",
                    "timeupdate",
                    "pause",
                    "ended",
                    "destroy",
                    "resumed",
                ]
                .contains(&kind.as_str())
            {
                state.position = position.min(duration);
                state.duration = duration;
                state.observed = true;
            }
        }
        let ending = ["pause", "ended", "destroy"].contains(&kind.as_str());
        let periodic = kind == "timeupdate"
            && state
                .last_saved
                .is_none_or(|last| at.saturating_sub(last) >= 5000);
        if (ending || periodic) && (!state.capturing || kind == "destroy") {
            out.saved = self.save(state)?;
            if out.saved {
                state.last_saved = Some(at);
            }
        }
        if kind == "destroy" {
            *active = None;
        }
        Ok(out)
    }

    pub fn finish(&self) -> Result<()> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| anyhow::anyhow!("播放状态锁异常"))?;
        if let Some(state) = active.as_ref() {
            self.save(state)?;
        }
        *active = None;
        Ok(())
    }

    fn save(&self, state: &Session) -> Result<bool> {
        if !state.observed || s(&state.entry, "siteKey").is_empty() {
            return Ok(false);
        }
        let mut entry = state.entry.clone();
        entry["position"] = json!(state.position.floor());
        entry["duration"] = json!(state.duration.floor());
        self.store.record_history(entry)?;
        Ok(true)
    }
}

fn positive(value: &Value) -> f64 {
    let value = number(value, 0.0);
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}
