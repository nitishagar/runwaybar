//! Golden renders: stable output for a frozen snapshot (CLI formatting contract).

use runwaybar::cli::{render_json, render_text, render_waybar};
use runwaybar::model::Snapshot;

fn golden() -> Snapshot {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/golden/status.json"
    ))
    .unwrap();
    serde_json::from_str(&raw).unwrap()
}

#[test]
fn json_render_roundtrips_the_golden_snapshot() {
    let snap = golden();
    let rendered = render_json(&snap);
    let back: Snapshot = serde_json::from_str(&rendered).unwrap();
    assert_eq!(back, snap);
}

#[test]
fn text_render_golden_lines() {
    let text = render_text(&golden());
    assert!(text.contains("Claude Code: ok"), "text was:\n{text}");
    assert!(text.contains("session  62%"), "text was:\n{text}");
    assert!(text.contains("weekly   31%"), "text was:\n{text}");
    assert!(text.contains("Codex: not installed"), "text was:\n{text}");
    // siloing: "pro" account appears exactly once, under claude-code
    assert_eq!(text.matches("pro").count(), 1, "text was:\n{text}");
}

#[test]
fn waybar_render_golden() {
    let out = render_waybar(&golden());
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["text"], "62%");
    assert_eq!(v["class"], "runway-warning");
    assert!(v["tooltip"].as_str().unwrap().contains("z.ai / ZCode"));
}
