use gpui::{hsla, App, Hsla};
use gpui_component::{Theme, ThemeMode};
use std::collections::HashMap;

fn parse_hex(hex: &str) -> Option<Hsla> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u32::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u32::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u32::from_str_radix(&hex[4..6], 16).ok()?;
    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < 0.001 {
        return Some(hsla(0.0, 0.0, l, 1.0));
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        (g - b) / d + (if g < b { 4.0 } else { 0.0 })
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    Some(hsla(h / 6.0, s, l, 1.0))
}

fn load_omarchy_colors() -> HashMap<String, Hsla> {
    let path = std::env::var("HOME")
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(".local/state/omarchy/current/theme/colors.toml"))
        .filter(|p| p.exists());

    let Some(path) = path else {
        return HashMap::new();
    };

    let Ok(content) = std::fs::read_to_string(&path) else {
        return HashMap::new();
    };

    let mut colors = HashMap::new();
    for line in content.lines() {
        if let Some((key, val)) = line.split_once('=') {
            let key = key.trim().to_string();
            let val = val.trim().trim_matches('"');
            if let Some(hsla) = parse_hex(val) {
                colors.insert(key, hsla);
            }
        }
    }
    colors
}

pub fn apply_omarchy_theme(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    let colors = load_omarchy_colors();
    if colors.is_empty() {
        return;
    }

    let theme = cx.global_mut::<Theme>();
    let c = &mut theme.colors;

    if let Some(&bg) = colors.get("background") {
        c.background = bg;
    }
    if let Some(&fg) = colors.get("foreground") {
        c.foreground = fg;
    }
    if let Some(&accent) = colors.get("accent") {
        c.accent = accent;
    }
    if let Some(&bright_fg) = colors.get("bright_foreground") {
        c.accent_foreground = bright_fg;
    }
    if let Some(&muted) = colors.get("muted") {
        c.muted = muted;
    }
    if let Some(&dark_fg) = colors.get("dark_foreground") {
        c.muted_foreground = dark_fg;
    }
    if let Some(&selection) = colors.get("selection") {
        c.selection = selection;
    }
    if let Some(&lighter_bg) = colors.get("lighter_background") {
        c.sidebar = lighter_bg;
        c.title_bar = lighter_bg;
    }
    if let Some(&dark_bg) = colors.get("dark_background") {
        c.sidebar_accent = dark_bg;
    }
    c.sidebar_accent_foreground = colors.get("bright_foreground").copied().unwrap_or(c.foreground);
    c.sidebar_foreground = colors.get("foreground").copied().unwrap_or(c.foreground);
    c.sidebar_border = colors.get("selection").copied().unwrap_or(c.border);
    c.title_bar_border = colors.get("selection").copied().unwrap_or(c.border);
    c.border = colors.get("selection").copied().unwrap_or(c.border);
}
