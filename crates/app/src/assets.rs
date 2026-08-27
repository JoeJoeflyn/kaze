use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

#[derive(rust_embed::RustEmbed)]
#[folder = "assets/icons"]
struct KazeAssets;

pub struct CombinedAssets;

impl AssetSource for CombinedAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }
        let stripped = path
            .strip_prefix("icons/")
            .or_else(|| path.strip_prefix("sidebar-icons/"))
            .unwrap_or(path);

        if let Some(file) = KazeAssets::get(stripped) {
            return Ok(Some(file.data));
        }
        if let Some(file) = KazeAssets::get(path) {
            return Ok(Some(file.data));
        }
        gpui_component_assets::Assets
            .load(path)
            .or_else(|_| gpui_component_assets::Assets.load(stripped))
            .or(Ok(None))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let stripped = path
            .strip_prefix("icons/")
            .or_else(|| path.strip_prefix("sidebar-icons/"))
            .unwrap_or(path);

        let mut entries: Vec<SharedString> = KazeAssets::iter()
            .filter_map(|p| {
                if p.starts_with(path) {
                    Some(p.into())
                } else if p.starts_with(stripped) {
                    Some(format!("icons/{}", p).into())
                } else {
                    None
                }
            })
            .collect();
        if let Ok(gpui_entries) = gpui_component_assets::Assets.list(path) {
            entries.extend(gpui_entries);
        }
        Ok(entries)
    }
}
