use std::path::PathBuf;

use gpui::{
    div, prelude::*, Context, EventEmitter, InteractiveElement, IntoElement, Render, SharedString,
    Window,
};
use gpui_component::{h_flex, v_flex, ActiveTheme, Icon, Sizable};
use kaze_file_list::FileDrag;

#[derive(Clone, Debug)]
pub struct FileTrashedEvent {
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct SidebarPathSelectedEvent {
    pub path: PathBuf,
}

#[derive(Clone)]
pub struct SidebarItem {
    pub label: SharedString,
    pub icon_path: SharedString,
    pub path: PathBuf,
}

pub struct SidebarView {
    items: Vec<SidebarItem>,
    selected: usize,
}

impl SidebarView {
    pub fn new() -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        let home_path = PathBuf::from(&home);

        let items = vec![
            SidebarItem {
                label: "Home".into(),
                icon_path: "icons/house.svg".into(),
                path: home_path.clone(),
            },
            SidebarItem {
                label: "Downloads".into(),
                icon_path: "icons/download.svg".into(),
                path: home_path.join("Downloads"),
            },
            SidebarItem {
                label: "Documents".into(),
                icon_path: "icons/file-text.svg".into(),
                path: home_path.join("Documents"),
            },
            SidebarItem {
                label: "Pictures".into(),
                icon_path: "icons/image.svg".into(),
                path: home_path.join("Pictures"),
            },
            SidebarItem {
                label: "Videos".into(),
                icon_path: "icons/film.svg".into(),
                path: home_path.join("Videos"),
            },
            SidebarItem {
                label: "Projects".into(),
                icon_path: "icons/layout-dashboard.svg".into(),
                path: if home_path.join("Projects").exists() {
                    home_path.join("Projects")
                } else {
                    home_path.join("Project")
                },
            },
            SidebarItem {
                label: "Trash".into(),
                icon_path: "icons/trash-2.svg".into(),
                path: home_path.join(".local/share/Trash/files"),
            },
        ];

        Self { items, selected: 0 }
    }

    pub fn selected_path(&self) -> &PathBuf {
        &self.items[self.selected].path
    }

    pub fn on_select(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.selected = ix;
        cx.emit(SidebarPathSelectedEvent {
            path: self.selected_path().clone(),
        });
        cx.notify();
    }
}

impl EventEmitter<FileTrashedEvent> for SidebarView {}
impl EventEmitter<SidebarPathSelectedEvent> for SidebarView {}

impl Default for SidebarView {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for SidebarView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        v_flex()
            .id("sidebar")
            .gap_0()
            .w_full()
            .py_2()
            .px_2()
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Favorites"),
            )
            .children(self.items.iter().enumerate().map(|(ix, item)| {
                let is_selected = ix == self.selected;
                let is_trash = item.label.as_ref() == "Trash";
                h_flex()
                    .id(("sidebar-item", ix))
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_sm()
                    .when(is_selected, |this| {
                        this.bg(theme.sidebar_accent)
                            .text_color(theme.sidebar_accent_foreground)
                    })
                    .when(!is_selected, |this| {
                        this.text_color(theme.sidebar_foreground)
                            .hover(|this| this.bg(theme.muted_foreground.alpha(0.08)))
                    })
                    .child(Icon::empty().path(item.icon_path.clone()).small())
                    .child(div().child(item.label.clone()))
                    .when(is_trash, |this| {
                        this.on_drop::<FileDrag>(cx.listener(
                            move |_this, drag: &FileDrag, _window, cx| {
                                match kaze_shared::move_to_trash(&drag.path) {
                                    Ok(()) => {
                                        cx.emit(FileTrashedEvent {
                                            path: drag.path.clone(),
                                        });
                                    }
                                    Err(err) => {
                                        eprintln!(
                                            "Kaze: failed to move {} to Trash: {}",
                                            drag.path.display(),
                                            err
                                        );
                                    }
                                }
                            },
                        ))
                        .drag_over::<FileDrag>(|this, _drag, _window, _cx| {
                            this.bg(gpui::hsla(0.0, 0.85, 0.55, 0.40))
                                .border_1()
                                .border_color(gpui::hsla(0.0, 0.9, 0.6, 0.9))
                                .rounded_md()
                        })
                    })
                    .when(!is_trash, |this| {
                        let target_dir = item.path.clone();
                        this.on_drop::<FileDrag>(cx.listener(
                            move |_this, drag: &FileDrag, _window, _cx| {
                                let target = target_dir.join(
                                    drag.path.file_name().unwrap_or_default()
                                );
                                let _ = std::fs::rename(&drag.path, &target);
                            }
                        ))
                        .drag_over::<FileDrag>(|this, _drag, _window, cx| {
                            this.bg(cx.theme().accent.alpha(0.35))
                                .border_1()
                                .border_color(cx.theme().accent)
                                .rounded_md()
                        })
                    })
                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                        this.on_select(ix, cx);
                    }))
            }))
    }
}
