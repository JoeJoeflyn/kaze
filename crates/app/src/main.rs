mod assets;

use std::path::PathBuf;

use gpui::{prelude::*, *};
use gpui_component::{h_flex, v_flex, ActiveTheme, Icon, IconName, Root, Sizable};
use kaze_file_list::{
    DeleteSelected, FileListView, NavigateUp, NewFolder, OpenSelected, OpenTabRequested, Refresh,
    SelectAll, ToggleHidden, ToggleReduceMotion, ToggleSidebarRequested, Undo,
};
use kaze_shared::theme::apply_omarchy_theme;
use kaze_sidebar::{FileTrashedEvent, SidebarPathSelectedEvent, SidebarView};

use assets::CombinedAssets;

fn main() {
    env_logger::init();

    let app = gpui_platform::application().with_assets(CombinedAssets);

    app.run(move |cx: &mut App| {
        gpui_component::init(cx);
        apply_omarchy_theme(cx);

        cx.bind_keys([
            KeyBinding::new("ctrl-h", ToggleHidden, None),
            KeyBinding::new("ctrl-b", ToggleSidebar, None),
            KeyBinding::new("delete", DeleteSelected, None),
            KeyBinding::new("alt-up", NavigateUp, None),
            KeyBinding::new("ctrl-r", Refresh, None),
            KeyBinding::new("ctrl-shift-n", NewFolder, None),
            KeyBinding::new("enter", OpenSelected, None),
            KeyBinding::new("ctrl-a", SelectAll, None),
            KeyBinding::new("ctrl-w", CloseTab, None),
            KeyBinding::new("ctrl-z", Undo, None),
            KeyBinding::new("ctrl-shift-m", ToggleReduceMotion, None),
        ]);

        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let workspace = cx.new(|cx| KazeWorkspace::new(window, cx));
                cx.new(|cx| Root::new(workspace, window, cx))
            })
            .expect("failed to open window");
        })
        .detach();
    });
}

struct TabState {
    label: SharedString,
    file_list: Entity<FileListView>,
}

pub struct KazeWorkspace {
    sidebar: Entity<SidebarView>,
    tabs: Vec<TabState>,
    active_tab: usize,
    sidebar_width: f32,
    sidebar_collapsed: bool,
    sidebar_width_before_collapse: f32,
    drag_start_x: Option<gpui::Pixels>,
    drag_start_width: f32,
    drag_velocity: f32,
    last_drag_sample: Option<(gpui::Pixels, std::time::Instant)>,
    pending_new_tab: Option<PathBuf>,
}

const SIDEBAR_MIN: f32 = 140.0;
const SIDEBAR_MAX: f32 = 500.0;
const SPRING_STIFFNESS: f32 = 600.0;
const SPRING_DAMPING: f32 = 42.0;

actions!(kaze, [ToggleSidebar, CloseTab]);

impl KazeWorkspace {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        let home_path = PathBuf::from(&home);

        let sidebar = cx.new(|_| SidebarView::new());

        let file_list = cx.new(|cx| FileListView::new(home_path.clone(), window, cx));

        cx.subscribe(&file_list, |this, _fl, _event: &ToggleSidebarRequested, cx| {
            this.toggle_sidebar(cx);
        })
        .detach();

        cx.subscribe(&file_list, |this, _fl, event: &OpenTabRequested, cx| {
            this.pending_new_tab = Some(event.path.clone());
            cx.notify();
        })
        .detach();

        let tabs = vec![TabState {
            label: "Home".into(),
            file_list,
        }];

        cx.subscribe(
            &sidebar,
            |this, _sidebar, event: &SidebarPathSelectedEvent, cx| {
                let path = event.path.clone();
                if let Some(tab) = this.tabs.get(this.active_tab) {
                    tab.file_list.update(cx, |view, cx| {
                        view.navigate(path, cx);
                    });
                }
                cx.notify();
            },
        )
        .detach();

        cx.subscribe(&sidebar, |this, _sidebar, event: &FileTrashedEvent, cx| {
            if let Some(tab) = this.tabs.get(this.active_tab) {
                tab.file_list.update(cx, |view, cx| {
                    view.animate_trash_drop_by_path(&event.path, cx);
                });
            }
            cx.notify();
        })
        .detach();

        Self {
            sidebar,
            tabs,
            active_tab: 0,
            sidebar_width: 200.0,
            sidebar_collapsed: false,
            sidebar_width_before_collapse: 200.0,
            drag_start_x: None,
            drag_start_width: 200.0,
            drag_velocity: 0.0,
            last_drag_sample: None,
            pending_new_tab: None,
        }
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        let target = if self.sidebar_collapsed {
            self.sidebar_collapsed = false;
            self.sidebar_width_before_collapse.clamp(SIDEBAR_MIN, SIDEBAR_MAX)
        } else {
            self.sidebar_width_before_collapse = self.sidebar_width;
            self.sidebar_collapsed = true;
            0.0
        };
        for tab in &self.tabs {
            tab.file_list.update(cx, |view, cx| {
                view.set_sidebar_collapsed(self.sidebar_collapsed, cx);
            });
        }
        self.animate_sidebar_to(target, 0.0, cx);
    }

    /// Spring-animate `sidebar_width` toward `target`, carrying `initial_velocity`.
    fn animate_sidebar_to(&mut self, target: f32, initial_velocity: f32, cx: &mut Context<Self>) {
        let mut width = self.sidebar_width;
        let mut velocity = initial_velocity;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(16))
                    .await;
                let dt = 0.016;
                let accel = (target - width) * SPRING_STIFFNESS - velocity * SPRING_DAMPING;
                velocity += accel * dt;
                width += velocity * dt;
                let done = (width - target).abs() < 0.5 && velocity.abs() < 2.0;
                let w = width;
                this.update(cx, |this, cx| {
                    this.sidebar_width = w;
                    cx.notify();
                })
                .ok();
                if done {
                    break;
                }
            }
        })
        .detach();
    }

    /// Rubber-band: damped resistance past the clamp boundaries.
    fn rubber_band(raw: f32) -> f32 {
        if raw < SIDEBAR_MIN {
            SIDEBAR_MIN - (SIDEBAR_MIN - raw) * 0.45
        } else if raw > SIDEBAR_MAX {
            SIDEBAR_MAX + (raw - SIDEBAR_MAX) * 0.45
        } else {
            raw
        }
    }

    fn add_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let home = std::env::var("HOME").unwrap_or_default();
        let path = PathBuf::from(&home);
        self.add_tab_with_path(path, window, cx);
    }

    fn add_tab_with_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let file_list = cx.new(|cx| FileListView::new(path.clone(), window, cx));
        file_list.update(cx, |view, cx| {
            view.set_sidebar_collapsed(self.sidebar_collapsed, cx);
        });

        cx.subscribe(&file_list, |this, _fl, _event: &ToggleSidebarRequested, cx| {
            this.toggle_sidebar(cx);
        })
        .detach();

        cx.subscribe(&file_list, |this, _fl, event: &OpenTabRequested, cx| {
            this.pending_new_tab = Some(event.path.clone());
            cx.notify();
        })
        .detach();

        let label: SharedString = path
            .file_name()
            .map(|n| n.to_string_lossy().into())
            .unwrap_or_else(|| "Folder".into());
        self.tabs.push(TabState {
            label,
            file_list,
        });
        self.active_tab = self.tabs.len() - 1;
    }

    fn close_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.tabs.len() <= 1 {
            return;
        }
        self.tabs.remove(ix);
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        }
        cx.notify();
    }

    fn select_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.tabs.len() {
            self.active_tab = ix;
        }
        cx.notify();
    }
}

impl Render for KazeWorkspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(path) = self.pending_new_tab.take() {
            self.add_tab_with_path(path, window, cx);
        }

        let theme = cx.theme();
        let active_tab = self.active_tab;
        let sidebar_width = self.sidebar_width;
        let sidebar_collapsed = self.sidebar_collapsed;
        let entity = cx.entity();

        let sidebar_panel = v_flex()
            .h_full()
            .w(gpui::px(sidebar_width.max(0.0)))
            .flex_shrink_0()
            .overflow_hidden()
            .bg(theme.sidebar.alpha(0.88))
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(
                // Sidebar content scrolls
                div()
                    .id("sidebar-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .child(self.sidebar.clone()),
            )
            .child(
                div()
                    .border_t_1()
                    .border_color(theme.sidebar_border)
                    .child(
                        v_flex()
                            .id("tabs-scroll")
                            .gap_0()
                            .px_2()
                            .py_2()
                            .max_h(gpui::px(200.0))
                            .overflow_y_scroll()
                            .restrict_scroll_to_axis()
                            .child(
                                h_flex()
                                    .items_center()
                                    .justify_between()
                                    .px_2()
                                    .py_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.muted_foreground)
                                            .child("Tabs"),
                                    )
                                    .child(
                                        div()
                                            .id("new-tab")
                                            .child(Icon::new(IconName::Plus).small())
                                            .hover(|this| this.bg(theme.muted).rounded_md())
                                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                                this.add_tab(window, cx);
                                            })),
                                    ),
                            )
                            .children(self.tabs.iter().enumerate().map(|(ix, tab)| {
                                let is_active = ix == active_tab;
                                let entity = cx.entity();
                                h_flex()
                                    .id(("tab", ix))
                                    .items_center()
                                    .gap_2()
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .text_sm()
                                    .when(is_active, |this| {
                                        this.bg(theme.sidebar_accent)
                                            .text_color(theme.sidebar_accent_foreground)
                                    })
                                    .when(!is_active, |this| {
                                        this.text_color(theme.sidebar_foreground)
                                            .hover(|this| this.bg(theme.muted_foreground.alpha(0.08)))
                                    })
                                    .child(Icon::new(IconName::Folder).small())
                                    .child(div().flex_1().child(tab.label.clone()))
                                    .when(self.tabs.len() > 1, |this| {
                                        this.child(
                                            div()
                                                .id(("close-tab", ix))
                                                .child(Icon::new(IconName::Close).small())
                                                .hover(|this| this.bg(theme.muted).rounded_sm())
                                                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                                    cx.stop_propagation();
                                                    entity.update(cx, |this, cx| {
                                                        this.close_tab(ix, cx);
                                                    });
                                                }),
                                        )
                                    })
                                    .on_click({
                                        let entity = cx.entity();
                                        move |_, _, cx| {
                                            entity.update(cx, |this, cx| {
                                                this.select_tab(ix, cx);
                                            });
                                        }
                                    })
                            })),
                    ),
            );

        // Drag handle between sidebar and file list
        let drag_handle = div()
            .id("sidebar-resize")
            .h_full()
            .w(gpui::px(4.0))
            .flex_shrink_0()
            .cursor_col_resize()
            .bg(theme.sidebar_border)
            .hover(|this| this.bg(theme.accent.alpha(0.5)))
            .on_drag(
                SidebarResize,
                |_, _, _, cx| cx.new(|_| gpui::Empty),
            );

        let active_file_list = self.tabs.get(active_tab).map(|t| t.file_list.clone());

        h_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .on_action(cx.listener(|this, _: &ToggleSidebar, _window, cx| {
                this.toggle_sidebar(cx);
            }))
            .on_action(cx.listener(|this, _: &CloseTab, _window, cx| {
                this.close_tab(this.active_tab, cx);
            }))
            .when(!sidebar_collapsed, |this| this.child(drag_handle))
            .child(sidebar_panel)
            .on_drag_move::<SidebarResize>({
                let entity = entity.clone();
                move |event, _window, cx| {
                    let current_x = event.event.position.x;
                    entity.update(cx, |this, cx| {
                        if this.sidebar_collapsed {
                            return;
                        }
                        // Record drag start on first move
                        if this.drag_start_x.is_none() {
                            this.drag_start_x = Some(current_x);
                            this.drag_start_width = this.sidebar_width;
                        }
                        let start_x = this.drag_start_x.unwrap();
                        let raw = this.drag_start_width + (current_x - start_x).as_f32();
                        this.sidebar_width = Self::rubber_band(raw);

                        // Velocity sampling for release hand-off
                        let now = std::time::Instant::now();
                        if let Some((px, pt)) = this.last_drag_sample {
                            let dt = pt.elapsed().as_secs_f64().max(0.001);
                            this.drag_velocity = (current_x - px).as_f32() / dt as f32;
                        }
                        this.last_drag_sample = Some((current_x, now));
                        cx.notify();
                    });
                }
            })
            .on_drop::<SidebarResize>({
                let entity = entity.clone();
                move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        let velocity = this.drag_velocity;
                        let target = this.sidebar_width.clamp(SIDEBAR_MIN, SIDEBAR_MAX);
                        this.drag_start_x = None;
                        this.last_drag_sample = None;
                        this.drag_velocity = 0.0;
                        // Spring settle with velocity hand-off (snaps rubber-band back)
                        this.animate_sidebar_to(target, velocity, cx);
                    });
                }
            })
            .children(active_file_list)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct SidebarResize;
