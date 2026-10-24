//! The assistant's "app" tools: what's playing, and the playback controls
//! (resume, pause, skip, shuffle, repeat, volume) for whatever is playing —
//! a local file or Spotify. The app does them at once and answers with what
//! actually happened; nothing on screen moves.

use super::tools::{AppOp, Control, Effect};
use serde_json::{Value, json};

pub const NAMES: &[&str] = &["now_playing", "player"];

pub fn definitions() -> Value {
    let f = |name: &str, desc: &str, props: Value, required: &[&str]| {
        json!({ "type": "function", "name": name, "description": desc, "strict": true,
            "parameters": { "type": "object", "properties": props, "required": required, "additionalProperties": false } })
    };
    json!({
        "type": "namespace",
        "name": "app",
        "description": "Glass itself: what's playing, and playback controls.",
        "tools": [
            f("now_playing", "What's playing right now (song, position, paused or not, shuffle, repeat, volume, device), with IDs.", json!({}), &[]),
            f("player", "Control playback of whatever is playing.", json!({
                "action": { "type": "string", "enum": ["resume", "pause", "next", "previous", "shuffle_on", "shuffle_off", "repeat_off", "repeat_all", "repeat_one", "set_volume"] },
                "volume_percent": { "type": ["integer", "null"], "description": "0–100, for set_volume." }
            }), &["action", "volume_percent"])
        ]
    })
}

pub fn run(name: &str, arguments: &str) -> (Value, Effect) {
    let a: Value = match serde_json::from_str(arguments) {
        Ok(v @ Value::Object(_)) => v,
        _ => return (json!({ "error": "arguments must be a JSON object matching the tool's schema" }), Effect::None),
    };
    match name {
        "now_playing" => (json!({}), Effect::Act(AppOp::NowPlaying)),
        "player" => {
            let c = match a["action"].as_str() {
                Some("resume") => Control::Resume,
                Some("pause") => Control::Pause,
                Some("next") => Control::Next,
                Some("previous") => Control::Previous,
                Some("shuffle_on") => Control::Shuffle(true),
                Some("shuffle_off") => Control::Shuffle(false),
                Some("repeat_off") => Control::Repeat("off"),
                Some("repeat_all") => Control::Repeat("context"),
                Some("repeat_one") => Control::Repeat("track"),
                Some("set_volume") => match a["volume_percent"].as_i64() {
                    Some(v) if (0..=100).contains(&v) => Control::Volume(v as u8),
                    _ => return (json!({ "error": "set_volume needs volume_percent from 0 to 100" }), Effect::None),
                },
                _ => return (json!({ "error": "unknown action" }), Effect::None),
            };
            (json!({}), Effect::Act(AppOp::Control(c)))
        }
        other => (json!({ "error": format!("unknown tool {other:?}") }), Effect::None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_are_checked() {
        assert_eq!(run("player", r#"{"action":"next","volume_percent":null}"#).1, Effect::Act(AppOp::Control(Control::Next)));
        assert_eq!(run("player", r#"{"action":"set_volume","volume_percent":35}"#).1, Effect::Act(AppOp::Control(Control::Volume(35))));
        for bad in [r#"{"action":"set_volume","volume_percent":400}"#, r#"{"action":"format_disk","volume_percent":null}"#, "nope"] {
            let (v, e) = run("player", bad);
            assert!(v["error"].is_string() && e == Effect::None, "{bad}");
        }
        let d = definitions();
        for t in d["tools"].as_array().unwrap() {
            assert_eq!(t["parameters"]["properties"].as_object().unwrap().len(), t["parameters"]["required"].as_array().unwrap().len());
        }
    }
}
