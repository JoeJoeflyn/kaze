use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use gpui::{
    actions, div, prelude::*, uniform_list, AnyElement, Context, Entity, EventEmitter,
    Focusable, InteractiveElement, IntoElement, MouseButton, Pixels, Render,
    SharedString, UniformListScrollHandle, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    menu::{ContextMenuExt, PopupMenu, PopupMenuItem},
    scroll::{Scrollbar, ScrollbarMode},
    tooltip::Tooltip,
    v_flex, ActiveTheme, Icon, IconName, Sizable,
};
use kaze_shared::{FileEntry, FileKind};

actions!(
    kaze,
    [
        ToggleHidden,
        DeleteSelected,
        NavigateUp,
        NewFolder,
        Refresh,
        OpenSelected,
        SelectAll,
        CloseModal,
        ConfirmModal,
        ToggleReduceMotion,
        Undo
    ]
);

const ROW_HEIGHT: f32 = 28.0;
const LIST_ROW_HEIGHT: f32 = 28.0;
const LIST_COL_DATE: f32 = 170.0;
const LIST_COL_SIZE: f32 = 100.0;
const LIST_COL_KIND: f32 = 130.0;
const MIN_COL_WIDTH: f32 = 160.0;
const MAX_COL_WIDTH: f32 = 500.0;
const CHAR_WIDTH: f32 = 7.0;
const COL_PADDING: f32 = 68.0;

fn auto_col_width(entries: &[FileEntry]) -> f32 {
    let max_name = entries
        .iter()
        .map(|e| e.name.chars().count())
        .max()
        .unwrap_or(0);
    let computed = max_name as f32 * CHAR_WIDTH + COL_PADDING;
    computed.clamp(MIN_COL_WIDTH, MAX_COL_WIDTH)
}

// Apple macOS Finder: 130ms crisp micro-fade and upward row collapse
const DELETE_ANIMATION_DURATION: Duration = Duration::from_millis(130);
// Apple macOS Finder: 350ms flying icon badge arc into sidebar Trash
const TRASH_FLIGHT_DURATION: Duration = Duration::from_millis(350);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ViewMode {
    Columns,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortColumn {
    Name,
    DateModified,
    Size,
    Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

pub fn is_in_trash(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains("/.local/share/Trash") || s.contains("/Trash/") || s.ends_with("/Trash") || s.ends_with("/Trash/files")
}

#[derive(Clone)]
struct PendingDeletion {
    path: PathBuf,
    started_at: Instant,
}

#[derive(Clone)]
struct TrashFlight {
    is_dir: bool,
    start_x: f32,
    start_y: f32,
    target_x: f32,
    target_y: f32,
    started_at: Instant,
}

pub enum ModalState {
    None,
    Rename {
        path: PathBuf,
        input: Entity<InputState>,
    },
    NewFolder {
        parent: PathBuf,
        input: Entity<InputState>,
    },
    NewFile {
        parent: PathBuf,
        input: Entity<InputState>,
    },
    GetInfo {
        entry: FileEntry,
    },
    ConfirmDelete {
        path: PathBuf,
        is_dir: bool,
    },
}

#[derive(Clone)]
struct UndoEntry {
    original_path: PathBuf,
    trash_path: PathBuf,
}

struct RowRenderContext {
    col_ix: usize,
    entry_ix: usize,
    is_selected: bool,
    entity: Entity<FileListView>,
    deletion_progress: Option<f32>,
    muted_foreground: gpui::Hsla,
    accent: gpui::Hsla,
    muted: gpui::Hsla,
}

pub fn horizontal_wheel_delta(delta_x: f32, delta_y: f32, shift_held: bool) -> Option<f32> {
    if shift_held {
        return Some(delta_y);
    }

    if delta_x.abs() < 1.0 || delta_x.abs() <= delta_y.abs() {
        return None;
    }

    Some(delta_x)
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct ColumnResize(usize);

#[derive(Clone, PartialEq, Eq)]
pub struct FileDrag {
    pub path: PathBuf,
    pub name: String,
    pub col_ix: usize,
    pub entry_ix: usize,
    pub is_dir: bool,
}

pub struct FileDragGhost {
    pub name: String,
    pub is_dir: bool,
}

impl gpui::Render for FileDragGhost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let icon = if self.is_dir {
            IconName::Folder
        } else {
            IconName::File
        };
        h_flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_1p5()
            .rounded_lg()
            .bg(theme.popover.alpha(0.78))
            .border_1()
            .border_color(theme.accent)
            .shadow_xl()
            .text_sm()
            .child(
                Icon::new(icon)
                    .small()
                    .text_color(if self.is_dir { theme.accent } else { theme.muted_foreground }),
            )
            .child(
                div()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.foreground)
                    .child(self.name.clone()),
            )
    }
}

impl gpui::Render for FileDrag {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let icon = if self.is_dir {
            IconName::Folder
        } else {
            IconName::File
        };
        h_flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_1p5()
            .rounded_lg()
            .bg(theme.popover.alpha(0.78))
            .border_1()
            .border_color(theme.accent)
            .shadow_xl()
            .text_sm()
            .child(
                Icon::new(icon)
                    .small()
                    .text_color(if self.is_dir { theme.accent } else { theme.muted_foreground }),
            )
            .child(
                div()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.foreground)
                    .child(self.name.clone()),
            )
    }
}

#[derive(Clone)]
struct Column {
    path: PathBuf,
    entries: Vec<FileEntry>,
    selected: Option<usize>,
    scroll: UniformListScrollHandle,
    width: f32,
}

pub fn sort_entries(entries: &mut [FileEntry], column: SortColumn, direction: SortDirection) {
    entries.sort_by(|a, b| {
        match (a.is_dir(), b.is_dir()) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => {
                let ordering = match column {
                    SortColumn::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                    SortColumn::DateModified => {
                        let time_a = a.modified.unwrap_or(std::time::UNIX_EPOCH);
                        let time_b = b.modified.unwrap_or(std::time::UNIX_EPOCH);
                        time_a.cmp(&time_b)
                    }
                    SortColumn::Size => a
                        .size
                        .cmp(&b.size)
                        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
                    SortColumn::Kind => a
                        .kind_label()
                        .cmp(b.kind_label())
                        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
                };
                match direction {
                    SortDirection::Ascending => ordering,
                    SortDirection::Descending => ordering.reverse(),
                }
            }
        }
    });
}

fn scan_dir(
    path: &PathBuf,
    show_hidden: bool,
    sort_col: SortColumn,
    sort_dir: SortDirection,
) -> Vec<FileEntry> {
    let mut entries: Vec<FileEntry> = std::fs::read_dir(path)
        .ok()
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| FileEntry::from_path(e.path()))
                .filter(|e| show_hidden || !e.name.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();

    sort_entries(&mut entries, sort_col, sort_dir);
    entries
}

pub fn unique_child_path(parent: &Path, name: &str) -> PathBuf {
    let initial = parent.join(name);
    if !initial.exists() {
        return initial;
    }

    for suffix in 2.. {
        let candidate = parent.join(format!("{} {}", name, suffix));
        if !candidate.exists() {
            return candidate;
        }
    }

    unreachable!("an unused folder name must eventually be available")
}

pub fn unique_file_path(parent: &Path, base_name: &str, ext: &str) -> PathBuf {
    let initial = parent.join(format!("{}.{}", base_name, ext));
    if !initial.exists() {
        return initial;
    }

    for suffix in 2.. {
        let candidate = parent.join(format!("{} {}.{}", base_name, suffix, ext));
        if !candidate.exists() {
            return candidate;
        }
    }

    unreachable!("an unused file name must eventually be available")
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

pub fn duplicate_path(src: &Path) -> std::io::Result<PathBuf> {
    let parent = src.parent().unwrap_or_else(|| Path::new("/"));
    let file_name = src.file_name().and_then(|n| n.to_str()).unwrap_or("file");

    let (stem, ext) = if src.is_dir() {
        (file_name, None)
    } else {
        let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or(file_name);
        let ext = src.extension().and_then(|e| e.to_str());
        (stem, ext)
    };

    let mut count = 1;
    loop {
        let new_name = if count == 1 {
            match ext {
                Some(ext) => format!("{} copy.{}", stem, ext),
                None => format!("{} copy", stem),
            }
        } else {
            match ext {
                Some(ext) => format!("{} copy {}.{}", stem, count, ext),
                None => format!("{} copy {}", stem, count),
            }
        };

        let target = parent.join(&new_name);
        if !target.exists() {
            if src.is_dir() {
                copy_dir_all(src, &target)?;
            } else {
                std::fs::copy(src, &target)?;
            }
            return Ok(target);
        }
        count += 1;
    }
}

pub fn open_in_terminal(path: &Path) {
    let dir = if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };

    let mut candidates = Vec::new();
    if let Ok(term) = std::env::var("TERMINAL") {
        candidates.push(term);
    }
    candidates.extend(vec![
        "ghostty".to_string(),
        "alacritty".to_string(),
        "kitty".to_string(),
        "wezterm".to_string(),
        "gnome-terminal".to_string(),
        "x-terminal-emulator".to_string(),
        "konsole".to_string(),
        "xfce4-terminal".to_string(),
        "foot".to_string(),
        "xterm".to_string(),
    ]);

    for term in candidates {
        if Command::new(&term).current_dir(dir).spawn().is_ok() {
            return;
        }
        if Command::new(&term)
            .arg("--working-directory")
            .arg(dir)
            .spawn()
            .is_ok()
        {
            return;
        }
    }
}

fn format_exact_bytes(bytes: u64) -> String {
    let s = bytes.to_string();
    let mut result = String::new();
    let chars: Vec<char> = s.chars().rev().collect();
    for (i, ch) in chars.iter().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(*ch);
    }
    result.chars().rev().collect()
}

pub struct FileListView {
    columns: Vec<Column>,
    search: Entity<InputState>,
    search_results: Vec<FileEntry>,
    searching: bool,
    last_click: Option<(usize, usize, Instant)>,
    col_drag_start_x: Option<Pixels>,
    col_drag_start_width: f32,
    show_hidden: bool,
    search_generation: u64,
    list_scroll: UniformListScrollHandle,
    search_scroll: UniformListScrollHandle,
    view_mode: ViewMode,
    sort_column: SortColumn,
    sort_direction: SortDirection,
    deleting: Vec<PendingDeletion>,
    trash_flights: Vec<TrashFlight>,
    delete_animation_active: bool,
    dragged_path: Option<PathBuf>,
    sidebar_collapsed: bool,
    modal_state: ModalState,
    last_error: Option<(String, Instant)>,
    reduce_motion: bool,
    undo_stack: Vec<UndoEntry>,
}

#[derive(Clone, Debug)]
pub struct ToggleSidebarRequested;

impl EventEmitter<ToggleSidebarRequested> for FileListView {}

#[derive(Clone, Debug)]
pub struct OpenTabRequested {
    pub path: PathBuf,
}

impl EventEmitter<OpenTabRequested> for FileListView {}

impl FileListView {
    pub fn new(cwd: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));

        cx.observe(&search, |this, search, cx| {
            let text = search.read(cx).value().trim().to_string();
            this.search_generation = this.search_generation.wrapping_add(1);
            let generation = this.search_generation;
            if text.is_empty() {
                this.search_results.clear();
                this.searching = false;
                cx.notify();
            } else {
                this.searching = true;
                this.run_plocate(&text, generation, cx);
            }
        })
        .detach();

        let sort_column = SortColumn::Name;
        let sort_direction = SortDirection::Ascending;
        let entries = scan_dir(&cwd, false, sort_column, sort_direction);
        let width = auto_col_width(&entries);
        Self {
            columns: vec![Column {
                path: cwd,
                entries,
                selected: None,
                scroll: UniformListScrollHandle::new(),
                width,
            }],
            search,
            search_results: vec![],
            searching: false,
            last_click: None,
            col_drag_start_x: None,
            col_drag_start_width: 0.0,
            show_hidden: false,
            search_generation: 0,
            list_scroll: UniformListScrollHandle::new(),
            search_scroll: UniformListScrollHandle::new(),
            view_mode: ViewMode::Columns,
            sort_column,
            sort_direction,
            deleting: vec![],
            trash_flights: vec![],
            delete_animation_active: false,
            dragged_path: None,
            sidebar_collapsed: false,
            modal_state: ModalState::None,
            last_error: None,
            reduce_motion: false,
            undo_stack: vec![],
        }
    }

    fn run_plocate(&mut self, query: &str, generation: u64, cx: &mut Context<Self>) {
        let query = query.to_string();
        let current_dir = self.cwd().clone();
        let home = std::env::var("HOME").unwrap_or_default();

        cx.spawn(async move |this, cx| {
            let results = cx
                .background_executor()
                .spawn(async move {
                    let mut found_paths: Vec<PathBuf> = Vec::new();
                    let mut seen = std::collections::HashSet::new();

                    let q_lower = query.to_lowercase();

                    // 1. Direct local scan in current dir, ~/Project, ~/Projects, and ~
                    let mut search_roots = vec![current_dir];
                    let home_p = PathBuf::from(&home);
                    search_roots.push(home_p.join("Project"));
                    search_roots.push(home_p.join("Projects"));
                    search_roots.push(home_p.join("Documents"));
                    search_roots.push(home_p.join("Downloads"));
                    search_roots.push(home_p.clone());

                    for root in search_roots {
                        if !root.exists() {
                            continue;
                        }
                        if let Ok(entries) = std::fs::read_dir(&root) {
                            for entry in entries.flatten() {
                                let path = entry.path();
                                let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                                if file_name.to_lowercase().contains(&q_lower) {
                                    if seen.insert(path.clone()) {
                                        found_paths.push(path);
                                    }
                                }
                            }
                        }
                    }

                    // 2. Also run plocate for broader system search
                    if let Ok(o) = Command::new("plocate")
                        .arg("-i")
                        .arg("-l")
                        .arg("100")
                        .arg(&query)
                        .output()
                    {
                        if o.status.success() {
                            for line in String::from_utf8_lossy(&o.stdout).lines() {
                                let p = PathBuf::from(line);
                                if seen.insert(p.clone()) {
                                    found_paths.push(p);
                                }
                            }
                        }
                    }

                    // 3. Convert to FileEntry and Rank intelligently
                    let mut entries: Vec<FileEntry> = found_paths
                        .into_iter()
                        .filter_map(|p| FileEntry::from_path(p))
                        .collect();

                    entries.sort_by(|a, b| {
                        let a_str = a.path.to_string_lossy();
                        let b_str = b.path.to_string_lossy();

                        let a_hidden = a_str.contains("/.") || a.name.starts_with('.');
                        let b_hidden = b_str.contains("/.") || b.name.starts_with('.');

                        let a_exact = a.name.eq_ignore_ascii_case(&q_lower);
                        let b_exact = b.name.eq_ignore_ascii_case(&q_lower);

                        let a_starts = a.name.to_lowercase().starts_with(&q_lower);
                        let b_starts = b.name.to_lowercase().starts_with(&q_lower);

                        let rank_a = if a_exact && !a_hidden {
                            0
                        } else if a_starts && !a_hidden {
                            1
                        } else if !a_hidden {
                            2
                        } else if a_exact {
                            3
                        } else {
                            4
                        };

                        let rank_b = if b_exact && !b_hidden {
                            0
                        } else if b_starts && !b_hidden {
                            1
                        } else if !b_hidden {
                            2
                        } else if b_exact {
                            3
                        } else {
                            4
                        };

                        rank_a
                            .cmp(&rank_b)
                            .then_with(|| match (a.is_dir(), b.is_dir()) {
                                (true, false) => std::cmp::Ordering::Less,
                                (false, true) => std::cmp::Ordering::Greater,
                                _ => a.name.cmp(&b.name),
                            })
                    });

                    entries.truncate(100);
                    entries
                })
                .await;

            this.update(cx, |this, cx| {
                if this.searching && this.search_generation == generation {
                    this.search_results = results;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn navigate(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let entries = scan_dir(&path, self.show_hidden, self.sort_column, self.sort_direction);
        let width = auto_col_width(&entries);
        self.columns = vec![Column {
            path,
            entries,
            selected: None,
            scroll: UniformListScrollHandle::new(),
            width,
        }];
        cx.notify();
    }

    pub fn cwd(&self) -> &PathBuf {
        &self.columns.first().unwrap().path
    }

    fn current_path(&self) -> Option<&PathBuf> {
        self.columns.last().map(|column| &column.path)
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        for column in &mut self.columns {
            column.entries = scan_dir(
                &column.path,
                self.show_hidden,
                self.sort_column,
                self.sort_direction,
            );
            column.selected = None;
            column.width = auto_col_width(&column.entries);
        }
        cx.notify();
    }

    fn navigate_up(&mut self, cx: &mut Context<Self>) {
        let parent = self
            .current_path()
            .and_then(|path| path.parent())
            .map(Path::to_path_buf);

        if let Some(parent) = parent {
            self.navigate(parent, cx);
        }
    }

    fn show_column(&mut self, col_ix: usize, cx: &mut Context<Self>) {
        if col_ix < self.columns.len() {
            self.columns.truncate(col_ix + 1);
            if let Some(column) = self.columns.last_mut() {
                column.selected = None;
                cx.notify();
            }
        }
    }

    fn set_view_mode(&mut self, view_mode: ViewMode, cx: &mut Context<Self>) {
        if self.view_mode != view_mode {
            self.view_mode = view_mode;
            cx.notify();
        }
    }

    pub fn toggle_sort(&mut self, column: SortColumn, cx: &mut Context<Self>) {
        if self.sort_column == column {
            self.sort_direction = match self.sort_direction {
                SortDirection::Ascending => SortDirection::Descending,
                SortDirection::Descending => SortDirection::Ascending,
            };
        } else {
            self.sort_column = column;
            self.sort_direction = match column {
                SortColumn::DateModified | SortColumn::Size => SortDirection::Descending,
                _ => SortDirection::Ascending,
            };
        }
        self.apply_sorting();
        cx.notify();
    }

    fn apply_sorting(&mut self) {
        for col in &mut self.columns {
            sort_entries(&mut col.entries, self.sort_column, self.sort_direction);
        }
    }

    fn create_folder(&mut self, cx: &mut Context<Self>) {
        let Some(parent) = self.current_path().cloned() else {
            return;
        };

        let path = unique_child_path(&parent, "Untitled Folder");
        if let Err(err) = std::fs::create_dir(&path) {
            self.set_error(format!("Failed to create folder: {}", err), cx);
            return;
        }

        if let Some(column) = self.columns.last_mut() {
            column.entries = scan_dir(
                &column.path,
                self.show_hidden,
                self.sort_column,
                self.sort_direction,
            );
            column.selected = column.entries.iter().position(|entry| entry.path == path);
        }
        cx.notify();
    }

    pub fn duplicate_item(&mut self, path: &Path, cx: &mut Context<Self>) {
        if let Ok(new_path) = duplicate_path(path) {
            self.refresh(cx);
            if let Some(col) = self.columns.last_mut() {
                col.selected = col.entries.iter().position(|e| e.path == new_path);
            }
            cx.notify();
        }
    }

    pub fn open_in_new_tab(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        cx.emit(OpenTabRequested { path });
    }

    pub fn start_rename(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let current_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("File name");
            state.set_value(current_name, window, cx);
            state
        });
        input.update(cx, |state, cx| {
            state.focus_handle(cx).focus(window, cx);
        });
        self.modal_state = ModalState::Rename { path, input };
        cx.notify();
    }

    pub fn prompt_new_folder(
        &mut self,
        parent: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let default_name = unique_child_path(&parent, "Untitled Folder")
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled Folder".to_string());
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("Folder name");
            state.set_value(default_name, window, cx);
            state
        });
        input.update(cx, |state, cx| {
            state.focus_handle(cx).focus(window, cx);
        });
        self.modal_state = ModalState::NewFolder { parent, input };
        cx.notify();
    }

    pub fn prompt_new_file(
        &mut self,
        parent: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let default_name = unique_file_path(&parent, "Untitled", "txt")
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled.txt".to_string());
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("File name");
            state.set_value(default_name, window, cx);
            state
        });
        input.update(cx, |state, cx| {
            state.focus_handle(cx).focus(window, cx);
        });
        self.modal_state = ModalState::NewFile { parent, input };
        cx.notify();
    }

    pub fn show_get_info(&mut self, entry: FileEntry, cx: &mut Context<Self>) {
        self.modal_state = ModalState::GetInfo { entry };
        cx.notify();
    }

    pub fn close_modal(&mut self, cx: &mut Context<Self>) {
        self.modal_state = ModalState::None;
        cx.notify();
    }

    pub fn confirm_modal(&mut self, cx: &mut Context<Self>) {
        match &self.modal_state {
            ModalState::None => {}
            ModalState::Rename { path, input } => {
                let new_name = input.read(cx).value().trim().to_string();
                if !new_name.is_empty() {
                    if let Some(parent) = path.parent() {
                        let target = parent.join(&new_name);
                        if target != *path {
                            if let Err(err) = std::fs::rename(path, &target) {
                                self.set_error(format!("Failed to rename: {}", err), cx);
                            } else {
                                self.refresh(cx);
                            }
                        }
                    }
                }
                self.modal_state = ModalState::None;
                cx.notify();
            }
            ModalState::NewFolder { parent, input } => {
                let name = input.read(cx).value().trim().to_string();
                if !name.is_empty() {
                    let target = parent.join(&name);
                    if let Err(err) = std::fs::create_dir(&target) {
                        self.set_error(format!("Failed to create folder: {}", err), cx);
                    } else {
                        self.refresh(cx);
                    }
                }
                self.modal_state = ModalState::None;
                cx.notify();
            }
            ModalState::NewFile { parent, input } => {
                let name = input.read(cx).value().trim().to_string();
                if !name.is_empty() {
                    let target = parent.join(&name);
                    if let Err(err) = std::fs::File::create(&target) {
                        self.set_error(format!("Failed to create file: {}", err), cx);
                    } else {
                        self.refresh(cx);
                    }
                }
                self.modal_state = ModalState::None;
                cx.notify();
            }
            ModalState::GetInfo { .. } => {
                self.modal_state = ModalState::None;
                cx.notify();
            }
            ModalState::ConfirmDelete { path, is_dir } => {
                let path = path.clone();
                let is_dir = *is_dir;
                let path_for_fs = path.clone();
                cx.background_spawn(async move {
                    if is_dir {
                        let _ = std::fs::remove_dir_all(&path_for_fs);
                    } else {
                        let _ = std::fs::remove_file(&path_for_fs);
                    }
                })
                .detach();
                self.delete_file_by_path(&path, cx);
                self.modal_state = ModalState::None;
                cx.notify();
            }
        }
    }

    fn open_search_result(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.searching = false;
        self.search_results.clear();
        self.search.update(cx, |search, cx| {
            search.set_value("", window, cx);
        });

        if path.is_dir() {
            self.navigate(path, cx);
        } else {
            if let Some(parent) = path.parent() {
                self.navigate(parent.to_path_buf(), cx);
                if let Some(col) = self.columns.last_mut() {
                    col.selected = col.entries.iter().position(|e| e.path == path);
                }
            }
            let _ = Command::new("xdg-open").arg(&path).spawn();
        }
        cx.notify();
    }

    fn toggle_hidden(&mut self, cx: &mut Context<Self>) {
        self.show_hidden = !self.show_hidden;
        for col in &mut self.columns {
            col.entries = scan_dir(
                &col.path,
                self.show_hidden,
                self.sort_column,
                self.sort_direction,
            );
            col.selected = None;
            col.width = auto_col_width(&col.entries);
        }
        cx.notify();
    }

    pub fn set_sidebar_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        self.sidebar_collapsed = collapsed;
        cx.notify();
    }

    fn open_selected(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let entry = self
            .columns
            .last()
            .and_then(|col| col.selected)
            .and_then(|ix| self.columns.last().and_then(|col| col.entries.get(ix)))
            .cloned();

        if let Some(entry) = entry {
            if entry.is_dir() {
                let path = entry.path.clone();
                self.navigate(path, cx);
            } else {
                Command::new("xdg-open").arg(&entry.path).spawn().ok();
            }
        }
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        if let Some(col) = self.columns.last_mut() {
            if !col.entries.is_empty() {
                col.selected = Some(0);
                // ponytail: real multi-select needs a HashSet; for now select first as anchor
            }
        }
        cx.notify();
    }

    fn set_error(&mut self, msg: String, cx: &mut Context<Self>) {
        self.last_error = Some((msg, Instant::now()));
        cx.notify();
        // Auto-clear after 4s
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(4))
                .await;
            this.update(cx, |this, cx| {
                this.last_error = None;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn toggle_reduce_motion(&mut self, cx: &mut Context<Self>) {
        self.reduce_motion = !self.reduce_motion;
        cx.notify();
    }

    fn undo_last_delete(&mut self, cx: &mut Context<Self>) {
        // ponytail: real undo needs trash-info parsing to find the trashed path;
        // this is a stub that restores from the most recent trash entry if found.
        if let Some(entry) = self.undo_stack.pop() {
            let original = entry.original_path.clone();
            let trash = entry.trash_path.clone();
            cx.background_spawn(async move {
                let _ = std::fs::rename(&trash, &original);
            })
            .detach();
            self.refresh(cx);
        } else {
            self.set_error("Nothing to undo".to_string(), cx);
        }
    }

    fn confirm_permanent_delete(&mut self, path: PathBuf, is_dir: bool, cx: &mut Context<Self>) {
        self.modal_state = ModalState::ConfirmDelete { path, is_dir };
        cx.notify();
    }

    fn select_in_column(&mut self, col_ix: usize, entry_ix: usize, cx: &mut Context<Self>) {
        if self.dragged_path.take().is_some() {
            return;
        }
        let now = Instant::now();
        let is_double = self
            .last_click
            .map(|(c, e, t)| {
                c == col_ix && e == entry_ix && now.duration_since(t) < Duration::from_millis(270)
            })
            .unwrap_or(false);

        if is_double {
            self.last_click = None;
            let entry = self
                .columns
                .get(col_ix)
                .and_then(|c| c.entries.get(entry_ix))
                .cloned();
            if let Some(entry) = entry {
                if entry.is_dir() {
                    let path = entry.path.clone();
                    if self.view_mode == ViewMode::List {
                        self.navigate(path, cx);
                    } else {
                        self.open_column(col_ix, path);
                    }
                } else {
                    Command::new("xdg-open").arg(&entry.path).spawn().ok();
                }
                cx.notify();
            }
        } else {
            self.last_click = Some((col_ix, entry_ix, now));
            if let Some(col) = self.columns.get_mut(col_ix) {
                col.selected = Some(entry_ix);
                let entry = col.entries.get(entry_ix).cloned();
                if let Some(entry) = entry {
                    // Always open directory column when clicking a folder (macOS Finder standard)
                    if entry.is_dir() && self.view_mode == ViewMode::Columns {
                        self.open_column(col_ix, entry.path);
                    }
                }
                cx.notify();
            }
        }
    }

    fn open_column(&mut self, after: usize, path: PathBuf) {
        if !path.is_dir() {
            return;
        }
        self.columns.truncate(after + 1);
        let entries = scan_dir(&path, self.show_hidden, self.sort_column, self.sort_direction);
        let width = auto_col_width(&entries);
        self.columns.push(Column {
            path,
            entries,
            selected: None,
            scroll: UniformListScrollHandle::new(),
            width,
        });
    }

    // Apple macOS Finder: Flying badge arc into sidebar Trash + micro-fade collapse
    pub fn animate_trash_drop_by_path(&mut self, path: &PathBuf, cx: &mut Context<Self>) {
        self.dragged_path = None;

        if self.reduce_motion {
            // Skip the flight animation; just queue the deletion collapse
            let path_clone = path.clone();
            cx.background_spawn(async move {
                if is_in_trash(&path_clone) {
                    if path_clone.is_dir() {
                        let _ = std::fs::remove_dir_all(&path_clone);
                    } else {
                        let _ = std::fs::remove_file(&path_clone);
                    }
                } else {
                    let _ = kaze_shared::move_to_trash(&path_clone);
                }
            })
            .detach();
            self.queue_deletion(path.clone(), cx);
            return;
        }

        let mut flight_info: Option<(bool, f32, f32)> = None;
        for (col_ix, col) in self.columns.iter().enumerate() {
            for (entry_ix, entry) in col.entries.iter().enumerate() {
                if entry.path == *path {
                    // Approximate position: column offset + row offset.
                    // ponytail: real fix needs measured screen bounds from the sidebar Trash item;
                    // target the left edge where the sidebar Trash icon lives.
                    let start_x = (col_ix as f32) * 220.0 + 80.0;
                    let start_y = (entry_ix as f32) * 28.0 + 60.0;
                    flight_info = Some((entry.is_dir(), start_x, start_y));
                    break;
                }
            }
            if flight_info.is_some() {
                break;
            }
        }

        if let Some((is_dir, start_x, start_y)) = flight_info {
            // Target: fly toward the sidebar Trash icon (left edge, ~bottom area)
            self.trash_flights.push(TrashFlight {
                is_dir,
                start_x,
                start_y,
                target_x: 24.0,
                target_y: 260.0,
                started_at: Instant::now(),
            });
        }

        let path_clone = path.clone();
        cx.background_spawn(async move {
            if is_in_trash(&path_clone) {
                if path_clone.is_dir() {
                    let _ = std::fs::remove_dir_all(&path_clone);
                } else {
                    let _ = std::fs::remove_file(&path_clone);
                }
            } else {
                let _ = kaze_shared::move_to_trash(&path_clone);
            }
        })
        .detach();

        self.queue_deletion(path.clone(), cx);
    }

    // Apple macOS Finder: Instantaneous micro-fade + upward spring gap closure
    pub fn delete_file_by_path(&mut self, path: &PathBuf, cx: &mut Context<Self>) {
        let path_clone = path.clone();
        cx.background_spawn(async move {
            if is_in_trash(&path_clone) {
                if path_clone.is_dir() {
                    let _ = std::fs::remove_dir_all(&path_clone);
                } else {
                    let _ = std::fs::remove_file(&path_clone);
                }
            } else {
                let _ = kaze_shared::move_to_trash(&path_clone);
            }
        })
        .detach();

        self.queue_deletion(path.clone(), cx);
    }

    pub fn empty_trash(&mut self, cx: &mut Context<Self>) {
        cx.background_spawn(async move {
            if let Ok(home) = std::env::var("HOME") {
                let trash_files = PathBuf::from(&home).join(".local/share/Trash/files");
                if let Ok(entries) = std::fs::read_dir(&trash_files) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.is_dir() {
                            let _ = std::fs::remove_dir_all(&p);
                        } else {
                            let _ = std::fs::remove_file(&p);
                        }
                    }
                }
                let trash_info = PathBuf::from(&home).join(".local/share/Trash/info");
                if let Ok(entries) = std::fs::read_dir(&trash_info) {
                    for entry in entries.flatten() {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
        })
        .detach();

        self.columns.truncate(1);
        if let Some(col) = self.columns.first_mut() {
            col.entries.clear();
            col.selected = None;
        }
        cx.notify();
    }

    fn queue_deletion(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.deleting.iter().any(|d| d.path == path) {
            return;
        }

        self.deleting.push(PendingDeletion {
            path,
            started_at: Instant::now(),
        });
        self.start_delete_animation(cx);
    }

    fn deletion_progress_for(&self, path: &Path) -> Option<f32> {
        self.deleting
            .iter()
            .find(|d| d.path == path)
            .map(|d| (d.started_at.elapsed().as_secs_f32() / DELETE_ANIMATION_DURATION.as_secs_f32()).min(1.0))
    }

    fn start_delete_animation(&mut self, cx: &mut Context<Self>) {
        if self.delete_animation_active {
            return;
        }
        self.delete_animation_active = true;

        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let keep_animating = this
                    .update(cx, |this, cx| {
                        let mut completed: Vec<PathBuf> = Vec::new();
                        this.deleting.retain(|pending| {
                            if pending.started_at.elapsed() >= DELETE_ANIMATION_DURATION {
                                completed.push(pending.path.clone());
                                false
                            } else {
                                true
                            }
                        });

                        this.trash_flights.retain(|flight| {
                            flight.started_at.elapsed() < TRASH_FLIGHT_DURATION
                        });

                        if !completed.is_empty() {
                            for path in &completed {
                                // Remove any child columns whose path is under the deleted folder
                                this.columns.retain(|col| !col.path.starts_with(path));

                                for col in &mut this.columns {
                                    if let Some(pos) = col.entries.iter().position(|e| e.path == *path) {
                                        col.entries.remove(pos);
                                        if col.selected == Some(pos) {
                                            col.selected = if col.entries.is_empty() {
                                                None
                                            } else {
                                                Some(pos.min(col.entries.len() - 1))
                                            };
                                        } else if let Some(selected) = col.selected {
                                            if selected > pos {
                                                col.selected = Some(selected - 1);
                                            }
                                        }
                                    }
                                }
                            }

                            // Trim empty trailing columns
                            while this.columns.len() > 1 {
                                if this.columns.last().map_or(false, |c| c.entries.is_empty()) {
                                    this.columns.pop();
                                } else {
                                    break;
                                }
                            }
                        }

                        let keep = !this.deleting.is_empty() || !this.trash_flights.is_empty();
                        if !keep {
                            this.delete_animation_active = false;
                        }
                        cx.notify();
                        keep
                    })
                    .unwrap_or(false);

                if !keep_animating {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn delete_selected(&mut self, cx: &mut Context<Self>) {
        let to_delete: Vec<PathBuf> = self
            .columns
            .iter()
            .filter_map(|col| {
                col.selected.and_then(|entry_ix| {
                    col.entries.get(entry_ix).map(|e| e.path.clone())
                })
            })
            .collect();

        for path in to_delete {
            self.delete_file_by_path(&path, cx);
        }
    }
}

impl Render for FileListView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let searching = self.searching;
        let view_mode = self.view_mode;
        let breadcrumb: Vec<(usize, SharedString)> = self
            .columns
            .iter()
            .enumerate()
            .map(|(ix, column)| {
                let label = column
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| column.path.display().to_string());
                (ix, label.into())
            })
            .collect();
        let current_item_count = self
            .columns
            .last()
            .map(|column| column.entries.len())
            .unwrap_or_default();
        let search_result_count = self.search_results.len();
        let entity = cx.entity();
        let in_trash = self.columns.last().map_or(false, |c| is_in_trash(&c.path));

        let modal_overlay = self.render_modal(cx);
        let trash_flights = self.trash_flights.clone();

        v_flex()
            .id("file-list-container")
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(theme.background)
            .on_action(cx.listener(|this, _: &ToggleHidden, _window, cx| {
                this.toggle_hidden(cx);
            }))
            .on_action(cx.listener(|this, _: &DeleteSelected, _window, cx| {
                this.delete_selected(cx);
            }))
            .on_action(cx.listener(|this, _: &NavigateUp, _window, cx| {
                this.navigate_up(cx);
            }))
            .on_action(cx.listener(|this, _: &NewFolder, _window, cx| {
                this.create_folder(cx);
            }))
            .on_action(cx.listener(|this, _: &Refresh, _window, cx| {
                this.refresh(cx);
            }))
            .on_action(cx.listener(|this, _: &OpenSelected, window, cx| {
                this.open_selected(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectAll, _window, cx| {
                this.select_all(cx);
            }))
            .on_action(cx.listener(|this, _: &CloseModal, _window, cx| {
                this.close_modal(cx);
            }))
            .on_action(cx.listener(|this, _: &ConfirmModal, _window, cx| {
                this.confirm_modal(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleReduceMotion, _window, cx| {
                this.toggle_reduce_motion(cx);
            }))
            .on_action(cx.listener(|this, _: &Undo, _window, cx| {
                this.undo_last_delete(cx);
            }))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .flex_shrink_0()
                    .bg(theme.background.alpha(0.85))
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .child({
                                let entity = entity.clone();
                                let sidebar_collapsed = self.sidebar_collapsed;
                                let sidebar_tooltip = if sidebar_collapsed {
                                    "Show sidebar (Ctrl+B)"
                                } else {
                                    "Hide sidebar (Ctrl+B)"
                                };
                                div()
                                    .id("toggle-sidebar")
                                    .p_1()
                                    .rounded_sm()
                                    .text_color(theme.muted_foreground)
                                    .hover(|this| this.bg(theme.muted))
                                    .child(Icon::new(IconName::PanelLeft).small())
                                    .tooltip(move |_, cx| {
                                        cx.new(|_| Tooltip::new(sidebar_tooltip)).into()
                                    })
                                    .on_click(move |_, _, cx| {
                                        entity.update(cx, |_, cx| cx.emit(ToggleSidebarRequested));
                                    })
                            })
                            .child({
                                let entity = entity.clone();
                                div()
                                    .id("go-up")
                                    .p_1()
                                    .rounded_sm()
                                    .text_color(theme.muted_foreground)
                                    .hover(|this| this.bg(theme.muted))
                                    .child(Icon::new(IconName::ArrowUp).small())
                                    .tooltip(|_, cx| {
                                        cx.new(|_| Tooltip::new("Go up (Alt+Up)")).into()
                                    })
                                    .on_click(move |_, _, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.navigate_up(cx);
                                        });
                                    })
                            })
                            .child({
                                let entity = entity.clone();
                                div()
                                    .id("refresh")
                                    .p_1()
                                    .rounded_sm()
                                    .text_color(theme.muted_foreground)
                                    .hover(|this| this.bg(theme.muted))
                                    .child(Icon::new(IconName::Redo).small())
                                    .tooltip(|_, cx| {
                                        cx.new(|_| Tooltip::new("Refresh (Ctrl+R)")).into()
                                    })
                                    .on_click(move |_, _, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.refresh(cx);
                                        });
                                    })
                            })
                            .when(!in_trash, |this| {
                                let entity = entity.clone();
                                this.child(
                                    div()
                                        .id("new-folder")
                                        .p_1()
                                        .rounded_sm()
                                        .text_color(theme.muted_foreground)
                                        .hover(|this| this.bg(theme.muted))
                                        .child(Icon::new(IconName::Plus).small())
                                        .tooltip(|_, cx| {
                                            cx.new(|_| Tooltip::new("New Folder (Ctrl+Shift+N)")).into()
                                        })
                                        .on_click(move |_, _, cx| {
                                            entity.update(cx, |this, cx| {
                                                this.create_folder(cx);
                                            });
                                        }),
                                )
                            }),
                    )
                    .child(
                        h_flex().items_center().gap_1().children(
                            breadcrumb.into_iter().map(|(col_ix, label)| {
                                let entity = entity.clone();
                                h_flex()
                                    .id(("breadcrumb", col_ix))
                                    .items_center()
                                    .gap_1()
                                    .text_sm()
                                    .text_color(theme.muted_foreground)
                                    .when(col_ix > 0, |this| {
                                        this.child(Icon::new(IconName::ChevronRight).small())
                                    })
                                    .child(Icon::new(IconName::Folder).small())
                                    .child(label)
                                    .hover(|this| this.text_color(theme.foreground))
                                    .on_click(move |_, _, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.show_column(col_ix, cx);
                                        });
                                    })
                            }),
                        ),
                    )
                    .child(div().flex_1())
                    .when(in_trash, |this| {
                        let entity = entity.clone();
                        this.child(
                            Button::new("btn-empty-trash")
                                .child("Empty Trash")
                                .icon(IconName::Delete)
                                .ghost()
                                .small()
                                .on_click(move |_, _window, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.empty_trash(cx);
                                    });
                                }),
                        )
                    })
                    .child({
                        // Sliding active pill: absolute indicator moves between the two slots
                        // ponytail: real spring needs GPUI animation API; this is a static
                        // position swap that reads as a sliding pill on the next frame.
                        let entity_cols = entity.clone();
                        let entity_list = entity.clone();
                        let pill_left = view_mode == ViewMode::Columns;
                        h_flex()
                            .items_center()
                            .relative()
                            .gap_1()
                            .p_1()
                            .rounded_md()
                            .bg(theme.muted_foreground.alpha(0.08))
                            .child(
                                div()
                                    .absolute()
                                    .top(gpui::px(4.0))
                                    .bottom(gpui::px(4.0))
                                    .left(gpui::px(if pill_left { 4.0 } else { 32.0 }))
                                    .w(gpui::px(24.0))
                                    .rounded_sm()
                                    .bg(theme.sidebar_accent),
                            )
                            .child(
                                div()
                                    .id("columns-view")
                                    .p_1()
                                    .w(gpui::px(24.0))
                                    .flex_shrink_0()
                                    .when(view_mode == ViewMode::Columns, |this| {
                                        this.text_color(theme.sidebar_accent_foreground)
                                    })
                                    .when(view_mode != ViewMode::Columns, |this| {
                                        this.text_color(theme.muted_foreground).hover(|this| {
                                            this.bg(theme.muted_foreground.alpha(0.12))
                                        })
                                    })
                                    .child(Icon::new(IconName::Frame).small())
                                    .tooltip(|_, cx| {
                                        cx.new(|_| Tooltip::new("Columns view")).into()
                                    })
                                    .on_click(move |_, _, cx| {
                                        entity_cols.update(cx, |this, cx| {
                                            this.set_view_mode(ViewMode::Columns, cx);
                                        });
                                    }),
                            )
                            .child(
                                div()
                                    .id("list-view")
                                    .p_1()
                                    .w(gpui::px(24.0))
                                    .flex_shrink_0()
                                    .when(view_mode == ViewMode::List, |this| {
                                        this.text_color(theme.sidebar_accent_foreground)
                                    })
                                    .when(view_mode != ViewMode::List, |this| {
                                        this.text_color(theme.muted_foreground).hover(|this| {
                                            this.bg(theme.muted_foreground.alpha(0.12))
                                        })
                                    })
                                    .child(Icon::new(IconName::Menu).small())
                                    .tooltip(|_, cx| cx.new(|_| Tooltip::new("List view")).into())
                                    .on_click(move |_, _, cx| {
                                        entity_list.update(cx, |this, cx| {
                                            this.set_view_mode(ViewMode::List, cx);
                                        });
                                    }),
                            )
                    })
                    .child(
                        h_flex().w(gpui::px(240.0)).child(
                            Input::new(&self.search)
                                .prefix(Icon::new(IconName::Search).small())
                                .appearance(false),
                        ),
                    ),
            )
            .child(if searching {
                self.render_search_results(cx).into_any_element()
            } else if view_mode == ViewMode::List {
                self.render_list_view(cx).into_any_element()
            } else {
                self.render_columns(cx).into_any_element()
            })
            .child(
                h_flex()
                    .items_center()
                    .px_3()
                    .h(gpui::px(24.0))
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(theme.border)
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(if searching {
                        format!("{} search results", search_result_count)
                    } else {
                        format!("{} items", current_item_count)
                    })
                    .when_some(self.last_error.as_ref(), |this, (msg, _)| {
                        this.child(div().flex_1())
                            .child(
                                div()
                                    .text_color(theme.danger_foreground)
                                    .child(msg.clone()),
                            )
                    }),
            )
            .children(trash_flights.into_iter().enumerate().map(|(ix, flight)| {
                let p = (flight.started_at.elapsed().as_secs_f32() / TRASH_FLIGHT_DURATION.as_secs_f32()).min(1.0);
                let t = 1.0 - (1.0 - p) * (1.0 - p);
                let jump_arc = -140.0 * (std::f32::consts::PI * p).sin();
                let cur_x = flight.start_x + (flight.target_x - flight.start_x) * t;
                let cur_y = flight.start_y + (flight.target_y - flight.start_y) * t + jump_arc;
                let opacity = (1.0 - p * 0.85).max(0.0);
                let icon = if flight.is_dir { IconName::Folder } else { IconName::File };

                // Big file/folder icon flying in a curved trajectory into the sidebar Trash can
                div()
                    .id(("flying-trash-badge", ix))
                    .absolute()
                    .left(gpui::px(cur_x))
                    .top(gpui::px(cur_y))
                    .items_center()
                    .justify_center()
                    .opacity(opacity)
                    .child(
                        Icon::new(icon)
                            .large()
                            .text_color(if flight.is_dir { theme.accent } else { theme.foreground }),
                    )
            }))
            .when_some(modal_overlay, |this, modal| this.child(modal))
            .on_drag_move::<ColumnResize>({
                let entity = entity.clone();
                move |event, _window, cx| {
                    let col_ix = event.drag(cx).0;
                    let current_x = event.event.position.x;
                    entity.update(cx, |this, cx| {
                        if this.col_drag_start_x.is_none() {
                            this.col_drag_start_x = Some(current_x);
                            this.col_drag_start_width = this.columns.get(col_ix).map(|c| c.width).unwrap_or(MIN_COL_WIDTH);
                        }
                        if let Some(start_x) = this.col_drag_start_x {
                            let raw = this.col_drag_start_width + (current_x - start_x).as_f32();
                            let clamped = if raw < MIN_COL_WIDTH {
                                MIN_COL_WIDTH - (MIN_COL_WIDTH - raw) * 0.45
                            } else if raw > MAX_COL_WIDTH {
                                MAX_COL_WIDTH + (raw - MAX_COL_WIDTH) * 0.45
                            } else {
                                raw
                            };
                            if let Some(col) = this.columns.get_mut(col_ix) {
                                col.width = clamped;
                            }
                        }
                        cx.notify();
                    });
                }
            })
            .on_drop::<ColumnResize>({
                let entity = entity.clone();
                move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        // Snap rubber-band back to clamp
                        for col in &mut this.columns {
                            col.width = col.width.clamp(MIN_COL_WIDTH, MAX_COL_WIDTH);
                        }
                        this.col_drag_start_x = None;
                        cx.notify();
                    });
                }
            })
    }
}

impl FileListView {
    fn render_columns(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let border_color = theme.border;
        let muted_foreground = theme.muted_foreground;
        let accent_color = theme.accent;
        let muted_color = theme.muted;
        let empty_icon_color = theme.muted_foreground.alpha(0.4);
        let empty_text_color = theme.muted_foreground;
        let entity = cx.entity();

        let col_meta: Vec<(UniformListScrollHandle, usize, Option<usize>, f32, PathBuf)> = self
            .columns
            .iter()
            .map(|c| {
                (
                    c.scroll.clone(),
                    c.entries.len(),
                    c.selected,
                    c.width,
                    c.path.clone(),
                )
            })
            .collect();

        h_flex()
            .id("columns-container")
            .flex_1()
            .size_full()
            .overflow_x_scroll()
            .restrict_scroll_to_axis()
            .children(col_meta.into_iter().enumerate().map(
                |(col_ix, (scroll, item_count, selected, width, col_path))| {
                    let entity_bg = entity.clone();
                    let col_path_bg = col_path.clone();

                    h_flex()
                        .id(("col-block", col_ix))
                        .h_full()
                        .flex_shrink_0()
                        .child(
                            v_flex()
                                .id(("col-container", col_ix))
                                .relative()
                                .h_full()
                                .w(gpui::px(width))
                                .flex_shrink_0()
                                .when(item_count == 0, |this| {
                                    this.child(
                                        v_flex()
                                            .id(("col-empty", col_ix))
                                            .size_full()
                                            .items_center()
                                            .justify_center()
                                            .gap_2()
                                            .context_menu(move |menu, _window, _cx| {
                                                build_background_context_menu(
                                                    menu,
                                                    &col_path_bg,
                                                    entity_bg.clone(),
                                                )
                                            })
                                            .child(
                                                Icon::new(IconName::FolderOpen)
                                                    .large()
                                                    .text_color(empty_icon_color),
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(empty_text_color)
                                                    .child("No Items"),
                                            ),
                                    )
                                })
                                .when(item_count > 0, |this| {
                                    this.child({
                                        let mut list = uniform_list(
                                            ("col", col_ix),
                                            item_count,
                                            cx.processor(
                                                move |this,
                                                      range: std::ops::Range<usize>,
                                                      _window,
                                                      cx| {
                                                    let col = match this.columns.get(col_ix) {
                                                        Some(c) => c,
                                                        None => return vec![],
                                                    };
                                                    let entity = cx.entity();
                                                    range
                                                        .map(|entry_ix| {
                                                            let entry = &col.entries[entry_ix];
                                                            let is_selected = selected == Some(entry_ix);
                                                            let entity = entity.clone();
                                                            let deletion_progress =
                                                                this.deletion_progress_for(&entry.path);
                                                            render_row(
                                                                entry,
                                                                RowRenderContext {
                                                                    col_ix,
                                                                    entry_ix,
                                                                    is_selected,
                                                                    entity,
                                                                    deletion_progress,
                                                                    muted_foreground,
                                                                    accent: accent_color,
                                                                    muted: muted_color,
                                                                },
                                                            )
                                                        })
                                                        .collect()
                                                },
                                            ),
                                        );
                                        list.interactivity().base_style.restrict_scroll_to_axis = Some(true);
                                        list.track_scroll(&scroll).flex_1().size_full()
                                    })
                                    .child(
                                        Scrollbar::vertical(&scroll)
                                            .id(("col-scrollbar", col_ix))
                                            .mode(ScrollbarMode::Hover),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .id(("col-resize", col_ix))
                                .w(gpui::px(6.0))
                                .h_full()
                                .flex_shrink_0()
                                .cursor_col_resize()
                                .items_center()
                                .justify_center()
                                .child(
                                    div()
                                        .w(gpui::px(1.0))
                                        .h_full()
                                        .bg(border_color),
                                )
                                .hover(|this| this.bg(theme.accent.alpha(0.35)))
                                .on_drag(ColumnResize(col_ix), |_, _, _, cx| {
                                    cx.new(|_| gpui::Empty)
                                }),
                        )
                },
            ))
    }

    fn render_header_cell(
        &self,
        label: &'static str,
        column: SortColumn,
        width: Option<f32>,
        active_sort: SortColumn,
        sort_dir: SortDirection,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let is_active = active_sort == column;
        let is_size = column == SortColumn::Size;
        let entity = cx.entity();

        let icon = if is_active {
            match sort_dir {
                SortDirection::Ascending => Some(IconName::ChevronUp),
                SortDirection::Descending => Some(IconName::ChevronDown),
            }
        } else {
            None
        };

        let mut cell = h_flex()
            .id(SharedString::from(format!("header-cell-{}", label)))
            .items_center()
            .gap_1()
            .h_full()
            .px_3()
            .cursor_pointer()
            .hover(|this| this.bg(cx.theme().muted.alpha(0.35)))
            .border_r_1()
            .border_color(cx.theme().border.alpha(0.5))
            .when(is_active, |this| {
                this.text_color(cx.theme().foreground)
                    .font_weight(gpui::FontWeight::SEMIBOLD)
            })
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    this.toggle_sort(column, cx);
                });
            });

        if is_size {
            cell = cell.justify_end();
        }

        cell = cell.child(div().child(label));

        if let Some(icon) = icon {
            cell = cell.child(Icon::new(icon).xsmall().flex_shrink_0().text_color(cx.theme().accent));
        }

        match width {
            Some(w) => cell.w(gpui::px(w)).flex_shrink_0(),
            None => cell.flex_1(),
        }
    }

    fn render_list_view(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let col_ix = self.columns.len().saturating_sub(1);
        let current_path = self
            .columns
            .get(col_ix)
            .map(|c| c.path.clone())
            .unwrap_or_default();
        let item_count = self
            .columns
            .get(col_ix)
            .map(|column| column.entries.len())
            .unwrap_or_default();
        let scroll = self.list_scroll.clone();
        let sort_column = self.sort_column;
        let sort_direction = self.sort_direction;
        let entity = cx.entity();

        let list_theme = ListRowTheme {
            foreground: theme.foreground,
            muted_foreground: theme.muted_foreground,
            selection: theme.selection,
            accent: theme.accent,
            muted: theme.muted,
        };

        let entity_bg = entity.clone();
        let current_path_bg = current_path.clone();

        v_flex()
            .id("finder-list")
            .flex_1()
            .size_full()
            .overflow_hidden()
            .child(
                h_flex()
                    .id("finder-list-header")
                    .items_center()
                    .w_full()
                    .h(gpui::px(LIST_ROW_HEIGHT))
                    .flex_shrink_0()
                    .bg(theme.muted.alpha(0.20))
                    .border_b_1()
                    .border_color(theme.border)
                    .text_xs()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.muted_foreground)
                    .child(self.render_header_cell(
                        "Name",
                        SortColumn::Name,
                        None,
                        sort_column,
                        sort_direction,
                        cx,
                    ))
                    .child(self.render_header_cell(
                        "Date Modified",
                        SortColumn::DateModified,
                        Some(LIST_COL_DATE),
                        sort_column,
                        sort_direction,
                        cx,
                    ))
                    .child(self.render_header_cell(
                        "Size",
                        SortColumn::Size,
                        Some(LIST_COL_SIZE),
                        sort_column,
                        sort_direction,
                        cx,
                    ))
                    .child(self.render_header_cell(
                        "Kind",
                        SortColumn::Kind,
                        Some(LIST_COL_KIND),
                        sort_column,
                        sort_direction,
                        cx,
                    )),
            )
            .child(if item_count == 0 {
                v_flex()
                    .id("finder-list-empty")
                    .flex_1()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .context_menu(move |menu, _window, _cx| {
                        build_background_context_menu(menu, &current_path_bg, entity_bg.clone())
                    })
                    .child(
                        Icon::new(IconName::FolderOpen)
                            .large()
                            .text_color(theme.muted_foreground.alpha(0.4)),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child("No Items"),
                    )
                    .into_any_element()
            } else {
                v_flex()
                    .relative()
                    .flex_1()
                    .size_full()
                    .child({
                        let mut list = uniform_list(
                            ("finder-list-rows", col_ix),
                            item_count,
                            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                                let column = match this.columns.get(col_ix) {
                                    Some(column) => column,
                                    None => return vec![],
                                };
                                let entity = cx.entity();
                                range
                                    .map(|entry_ix| {
                                        let entry = &column.entries[entry_ix];
                                        let is_selected = column.selected == Some(entry_ix);
                                        let deletion_progress =
                                            this.deletion_progress_for(&entry.path);
                                        render_list_row(
                                            entry,
                                            col_ix,
                                            entry_ix,
                                            is_selected,
                                            entity.clone(),
                                            deletion_progress,
                                            list_theme,
                                        )
                                    })
                                    .collect()
                            }),
                        );
                        list.interactivity().base_style.restrict_scroll_to_axis = Some(true);
                        list.track_scroll(&scroll).flex_1().size_full()
                    })
                    .child(
                        Scrollbar::vertical(&scroll)
                            .id("finder-list-scrollbar")
                            .mode(ScrollbarMode::Hover),
                    )
                    .into_any_element()
            })
            .into_any_element()
    }

    fn render_search_results(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let item_count = self.search_results.len();
        let entity = cx.entity();
        let scroll = self.search_scroll.clone();
        let theme = cx.theme().clone();

        v_flex()
            .id("search-container")
            .flex_1()
            .size_full()
            .child(
                div()
                    .relative()
                    .flex_1()
                    .child({
                        let mut list = uniform_list(
                            "search-results",
                            item_count,
                            cx.processor(
                                move |this, range: std::ops::Range<usize>, _window, _cx| {
                                    range
                                        .map(|ix| {
                                            let entry = &this.search_results[ix];
                                            render_search_item(ix, entry, entity.clone(), &theme)
                                        })
                                        .collect()
                                },
                            ),
                        );
                        list.interactivity().base_style.restrict_scroll_to_axis = Some(true);
                        list.track_scroll(&scroll).size_full()
                    })
                    .child(
                        Scrollbar::vertical(&scroll)
                            .id("search-scrollbar")
                            .mode(ScrollbarMode::Hover),
                    ),
            )
    }

    fn render_modal(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let theme = cx.theme();
        let entity = cx.entity();

        match &self.modal_state {
            ModalState::None => None,
            ModalState::Rename { input, .. } => {
                let entity_cancel = entity.clone();
                let entity_confirm = entity.clone();
                Some(
                    div()
                        .id("modal-backdrop")
                        .absolute()
                        .inset_0()
                        .bg(theme.overlay)
                        .items_center()
                        .justify_center()
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            entity_cancel.update(cx, |this, cx| this.close_modal(cx));
                        })
                        .child(
                            v_flex()
                                .id("modal-card")
                                .w(gpui::px(380.0))
                                .bg(theme.popover)
                                .border_1()
                                .border_color(theme.border)
                                .rounded_lg()
                                .shadow_lg()
                                .p_4()
                                .gap_3()
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .child(
                                    h_flex()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .text_sm()
                                                .child("Rename Item"),
                                        )
                                        .child({
                                            let entity = entity.clone();
                                            Button::new("btn-close-rename")
                                                .icon(IconName::Close)
                                                .ghost()
                                                .xsmall()
                                                .on_click(move |_, _, cx| {
                                                    entity.update(cx, |this, cx| {
                                                        this.close_modal(cx);
                                                    });
                                                })
                                        }),
                                )
                                .child(Input::new(input).appearance(true))
                                .child(
                                    h_flex()
                                        .justify_end()
                                        .gap_2()
                                        .child(
                                            Button::new("btn-cancel")
                                                .child("Cancel")
                                                .ghost()
                                                .on_click({
                                                    let entity = entity.clone();
                                                    move |_, _, cx| {
                                                        entity.update(cx, |this, cx| {
                                                            this.close_modal(cx);
                                                        });
                                                    }
                                                }),
                                        )
                                        .child(
                                            Button::new("btn-rename")
                                                .child("Rename")
                                                .primary()
                                                .on_click(move |_, _, cx| {
                                                    entity_confirm.update(cx, |this, cx| {
                                                        this.confirm_modal(cx);
                                                    });
                                                }),
                                        ),
                                ),
                        )
                        .into_any_element(),
                )
            }
            ModalState::NewFolder { input, .. } => {
                let entity_cancel = entity.clone();
                let entity_confirm = entity.clone();
                Some(
                    div()
                        .id("modal-backdrop")
                        .absolute()
                        .inset_0()
                        .bg(theme.overlay)
                        .items_center()
                        .justify_center()
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            entity_cancel.update(cx, |this, cx| this.close_modal(cx));
                        })
                        .child(
                            v_flex()
                                .id("modal-card")
                                .w(gpui::px(380.0))
                                .bg(theme.popover)
                                .border_1()
                                .border_color(theme.border)
                                .rounded_lg()
                                .shadow_lg()
                                .p_4()
                                .gap_3()
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .child(
                                    h_flex()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .text_sm()
                                                .child("New Folder"),
                                        )
                                        .child({
                                            let entity = entity.clone();
                                            Button::new("btn-close-newfolder")
                                                .icon(IconName::Close)
                                                .ghost()
                                                .xsmall()
                                                .on_click(move |_, _, cx| {
                                                    entity.update(cx, |this, cx| {
                                                        this.close_modal(cx);
                                                    });
                                                })
                                        }),
                                )
                                .child(Input::new(input).appearance(true))
                                .child(
                                    h_flex()
                                        .justify_end()
                                        .gap_2()
                                        .child(
                                            Button::new("btn-cancel")
                                                .child("Cancel")
                                                .ghost()
                                                .on_click({
                                                    let entity = entity.clone();
                                                    move |_, _, cx| {
                                                        entity.update(cx, |this, cx| {
                                                            this.close_modal(cx);
                                                        });
                                                    }
                                                }),
                                        )
                                        .child(
                                            Button::new("btn-create")
                                                .child("Create")
                                                .primary()
                                                .on_click(move |_, _, cx| {
                                                    entity_confirm.update(cx, |this, cx| {
                                                        this.confirm_modal(cx);
                                                    });
                                                }),
                                        ),
                                ),
                        )
                        .into_any_element(),
                )
            }
            ModalState::NewFile { input, .. } => {
                let entity_cancel = entity.clone();
                let entity_confirm = entity.clone();
                Some(
                    div()
                        .id("modal-backdrop")
                        .absolute()
                        .inset_0()
                        .bg(theme.overlay)
                        .items_center()
                        .justify_center()
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            entity_cancel.update(cx, |this, cx| this.close_modal(cx));
                        })
                        .child(
                            v_flex()
                                .id("modal-card")
                                .w(gpui::px(380.0))
                                .bg(theme.popover)
                                .border_1()
                                .border_color(theme.border)
                                .rounded_lg()
                                .shadow_lg()
                                .p_4()
                                .gap_3()
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .child(
                                    h_flex()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .text_sm()
                                                .child("New File"),
                                        )
                                        .child({
                                            let entity = entity.clone();
                                            Button::new("btn-close-newfile")
                                                .icon(IconName::Close)
                                                .ghost()
                                                .xsmall()
                                                .on_click(move |_, _, cx| {
                                                    entity.update(cx, |this, cx| {
                                                        this.close_modal(cx);
                                                    });
                                                })
                                        }),
                                )
                                .child(Input::new(input).appearance(true))
                                .child(
                                    h_flex()
                                        .justify_end()
                                        .gap_2()
                                        .child(
                                            Button::new("btn-cancel")
                                                .child("Cancel")
                                                .ghost()
                                                .on_click({
                                                    let entity = entity.clone();
                                                    move |_, _, cx| {
                                                        entity.update(cx, |this, cx| {
                                                            this.close_modal(cx);
                                                        });
                                                    }
                                                }),
                                        )
                                        .child(
                                            Button::new("btn-create-file")
                                                .child("Create")
                                                .primary()
                                                .on_click(move |_, _, cx| {
                                                    entity_confirm.update(cx, |this, cx| {
                                                        this.confirm_modal(cx);
                                                    });
                                                }),
                                        ),
                                ),
                        )
                        .into_any_element(),
                )
            }
            ModalState::GetInfo { entry } => {
                let entity_close = entity.clone();
                let icon = match entry.kind {
                    FileKind::Directory => IconName::Folder,
                    FileKind::File => IconName::File,
                    FileKind::Symlink => IconName::ArrowRight,
                };
                let full_path = entry.path.to_string_lossy().to_string();
                let size_detail = if entry.is_dir() {
                    "--".to_string()
                } else {
                    format!(
                        "{} ({} bytes)",
                        entry.size_label(),
                        format_exact_bytes(entry.size)
                    )
                };
                let modified_str = entry.date_label().to_string();

                Some(
                    div()
                        .id("modal-backdrop")
                        .absolute()
                        .inset_0()
                        .bg(theme.overlay)
                        .items_center()
                        .justify_center()
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            entity_close.update(cx, |this, cx| this.close_modal(cx));
                        })
                        .child(
                            v_flex()
                                .id("modal-card")
                                .w(gpui::px(420.0))
                                .bg(theme.popover)
                                .border_1()
                                .border_color(theme.border)
                                .rounded_lg()
                                .shadow_lg()
                                .p_4()
                                .gap_3()
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .child(
                                    h_flex()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .text_sm()
                                                .child("Get Info"),
                                        )
                                        .child({
                                            let entity = entity.clone();
                                            Button::new("btn-close-getinfo")
                                                .icon(IconName::Close)
                                                .ghost()
                                                .xsmall()
                                                .on_click(move |_, _, cx| {
                                                    entity.update(cx, |this, cx| {
                                                        this.close_modal(cx);
                                                    });
                                                })
                                        }),
                                )
                                .child(
                                    h_flex()
                                        .items_center()
                                        .gap_3()
                                        .p_2()
                                        .rounded_md()
                                        .bg(theme.muted.alpha(0.12))
                                        .child(
                                            Icon::new(icon).large().text_color(if entry.is_dir() {
                                                theme.accent
                                            } else {
                                                theme.muted_foreground
                                            }),
                                        )
                                        .child(
                                            v_flex()
                                                .gap_0()
                                                .child(
                                                    div()
                                                        .font_weight(gpui::FontWeight::MEDIUM)
                                                        .text_sm()
                                                        .child(entry.name.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(theme.muted_foreground)
                                                        .child(entry.kind_label()),
                                                ),
                                        ),
                                )
                                .child(
                                    v_flex()
                                        .gap_2()
                                        .text_xs()
                                        .child(info_row(
                                            "Size",
                                            &size_detail,
                                            theme.muted_foreground,
                                        ))
                                        .child(info_row(
                                            "Modified",
                                            &modified_str,
                                            theme.muted_foreground,
                                        ))
                                        .child(info_row(
                                            "Where",
                                            &full_path,
                                            theme.muted_foreground,
                                        )),
                                )
                                .child(
                                    h_flex().justify_end().gap_2().child(
                                        Button::new("btn-close-info")
                                            .child("Close")
                                            .primary()
                                            .on_click({
                                                let entity = entity.clone();
                                                move |_, _, cx| {
                                                    entity.update(cx, |this, cx| {
                                                        this.close_modal(cx);
                                                    });
                                                }
                                            }),
                                    ),
                                ),
                        )
                        .into_any_element(),
                )
            }
            ModalState::ConfirmDelete { path, is_dir } => {
                let entity_cancel = entity.clone();
                let entity_confirm = entity.clone();
                let path_confirm = path.clone();
                let is_dir_confirm = *is_dir;
                let display_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                let kind_label = if *is_dir { "folder" } else { "file" };
                Some(
                    div()
                        .id("modal-backdrop")
                        .absolute()
                        .inset_0()
                        .bg(theme.overlay)
                        .items_center()
                        .justify_center()
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            entity_cancel.update(cx, |this, cx| this.close_modal(cx));
                        })
                        .child(
                            v_flex()
                                .id("modal-card")
                                .w(gpui::px(380.0))
                                .bg(theme.popover)
                                .border_1()
                                .border_color(theme.border)
                                .rounded_lg()
                                .shadow_lg()
                                .p_4()
                                .gap_3()
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .child(
                                    h_flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            Icon::new(IconName::Delete)
                                                .small()
                                                .text_color(theme.danger_foreground),
                                        )
                                        .child(
                                            div()
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .text_sm()
                                                .child("Delete Permanently"),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(theme.muted_foreground)
                                        .child(format!(
                                            "“{}” will be permanently deleted. This {} cannot be undone.",
                                            display_name, kind_label
                                        )),
                                )
                                .child(
                                    h_flex()
                                        .justify_end()
                                        .gap_2()
                                        .child(
                                            Button::new("btn-cancel-del")
                                                .child("Cancel")
                                                .ghost()
                                                .on_click({
                                                    let entity = entity.clone();
                                                    move |_, _, cx| {
                                                        entity.update(cx, |this, cx| {
                                                            this.close_modal(cx);
                                                        });
                                                    }
                                                }),
                                        )
                                        .child(
                                            Button::new("btn-confirm-del")
                                                .child("Delete")
                                                .danger()
                                                .on_click(move |_, _, cx| {
                                                    entity_confirm.update(cx, |this, cx| {
                                                        let path_clone = path_confirm.clone();
                                                        cx.background_spawn(async move {
                                                            if is_dir_confirm {
                                                                let _ = std::fs::remove_dir_all(&path_clone);
                                                            } else {
                                                                let _ = std::fs::remove_file(&path_clone);
                                                            }
                                                        })
                                                        .detach();
                                                        this.delete_file_by_path(&path_confirm, cx);
                                                        this.close_modal(cx);
                                                    });
                                                }),
                                        ),
                                ),
                        )
                        .into_any_element(),
                )
            }
        }
    }
}

fn info_row(label: &'static str, value: &str, muted: gpui::Hsla) -> impl IntoElement {
    h_flex()
        .items_start()
        .gap_2()
        .child(
            div()
                .w(gpui::px(70.0))
                .flex_shrink_0()
                .text_color(muted)
                .child(label),
        )
        .child(div().flex_1().child(value.to_string()))
}

#[derive(Clone, Copy)]
struct ListRowTheme {
    foreground: gpui::Hsla,
    muted_foreground: gpui::Hsla,
    selection: gpui::Hsla,
    accent: gpui::Hsla,
    muted: gpui::Hsla,
}

fn build_item_context_menu(
    menu: PopupMenu,
    entry: &FileEntry,
    _col_ix: usize,
    _entry_ix: usize,
    entity: Entity<FileListView>,
) -> PopupMenu {
    let path = entry.path.clone();
    let is_dir = entry.is_dir();
    let name = entry.name.clone();
    let in_trash = is_in_trash(&path);

    let mut menu = menu;

    // 1. Open
    {
        let path = path.clone();
        let entity = entity.clone();
        menu = menu.item(
            PopupMenuItem::new("Open")
                .icon(if is_dir {
                    IconName::FolderOpen
                } else {
                    IconName::ExternalLink
                })
                .on_click(move |_, _window, cx| {
                    entity.update(cx, |this, cx| {
                        if is_dir {
                            this.navigate(path.clone(), cx);
                        } else {
                            Command::new("xdg-open").arg(&path).spawn().ok();
                        }
                    });
                }),
        );
    }

    // 2. Open in New Tab (for folders)
    if is_dir {
        let path = path.clone();
        let entity = entity.clone();
        menu = menu.item(
            PopupMenuItem::new("Open in New Tab")
                .icon(IconName::Plus)
                .on_click(move |_, _window, cx| {
                    entity.update(cx, |this, cx| {
                        this.open_in_new_tab(path.clone(), cx);
                    });
                }),
        );
    }

    // 3. Open in Terminal
    {
        let path = path.clone();
        menu = menu.item(
            PopupMenuItem::new("Open in Terminal")
                .icon(IconName::SquareTerminal)
                .on_click(move |_, _window, _cx| {
                    open_in_terminal(&path);
                }),
        );
    }

    menu = menu.separator();

    // 4. Rename (if not in trash)
    if !in_trash {
        let path = path.clone();
        let entity = entity.clone();
        menu = menu.item(
            PopupMenuItem::new("Rename...")
                .icon(IconName::Replace)
                .on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.start_rename(path.clone(), window, cx);
                    });
                }),
        );
    }

    // 5. Duplicate (if not in trash)
    if !in_trash {
        let path = path.clone();
        let entity = entity.clone();
        menu = menu.item(
            PopupMenuItem::new("Duplicate")
                .icon(IconName::Copy)
                .on_click(move |_, _window, cx| {
                    entity.update(cx, |this, cx| {
                        this.duplicate_item(&path, cx);
                    });
                }),
        );
    }

    // 6. Copy Path
    {
        let path_str = path.to_string_lossy().to_string();
        menu = menu.item(
            PopupMenuItem::new("Copy Path")
                .icon(IconName::Copy)
                .on_click(move |_, _window, cx| {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(path_str.clone()));
                }),
        );
    }

    // 7. Copy Name
    {
        let name_str = name;
        menu = menu.item(
            PopupMenuItem::new("Copy Name")
                .icon(IconName::Copy)
                .on_click(move |_, _window, cx| {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(name_str.clone()));
                }),
        );
    }

    // 8. New Folder / File (if not in trash)
    if !in_trash {
        menu = menu.separator();

        let parent = if is_dir {
            path.clone()
        } else {
            path.parent().unwrap_or(&path).to_path_buf()
        };
        let entity_folder = entity.clone();
        let parent_folder = parent.clone();
        menu = menu.item(
            PopupMenuItem::new("New Folder")
                .icon(IconName::Plus)
                .on_click(move |_, window, cx| {
                    entity_folder.update(cx, |this, cx| {
                        this.prompt_new_folder(parent_folder.clone(), window, cx);
                    });
                }),
        );

        let entity_file = entity.clone();
        menu = menu.item(
            PopupMenuItem::new("New File")
                .icon(IconName::Plus)
                .on_click(move |_, window, cx| {
                    entity_file.update(cx, |this, cx| {
                        this.prompt_new_file(parent.clone(), window, cx);
                    });
                }),
        );
    }

    menu = menu.separator();

    if in_trash {
        // If already inside trash, DO NOT show "Move to Trash", show ONLY "Delete Permanently"
        let path = path.clone();
        let is_dir_perm = is_dir;
        let entity = entity.clone();
        menu = menu.item(
            PopupMenuItem::new("Delete Permanently")
                .icon(IconName::Delete)
                .on_click(move |_, _window, cx| {
                    entity.update(cx, |this, cx| {
                        this.confirm_permanent_delete(path.clone(), is_dir_perm, cx);
                    });
                }),
        );
    } else {
        // Outside trash: show "Move to Trash" and "Delete Permanently"
        let path_trash = path.clone();
        let entity_trash = entity.clone();
        menu = menu.item(
            PopupMenuItem::new("Move to Trash")
                .icon(IconName::Delete)
                .on_click(move |_, _window, cx| {
                    entity_trash.update(cx, |this, cx| {
                        this.delete_file_by_path(&path_trash, cx);
                    });
                }),
        );

        let path_perm = path.clone();
        let is_dir_perm = is_dir;
        let entity_perm = entity.clone();
        menu = menu.item(
            PopupMenuItem::new("Delete Permanently")
                .icon(IconName::Close)
                .on_click(move |_, _window, cx| {
                    entity_perm.update(cx, |this, cx| {
                        this.confirm_permanent_delete(path_perm.clone(), is_dir_perm, cx);
                    });
                }),
        );
    }

    menu = menu.separator();

    // Get Info
    {
        let entry_clone = entry.clone();
        let entity = entity;
        menu = menu.item(
            PopupMenuItem::new("Get Info")
                .icon(IconName::Info)
                .on_click(move |_, _window, cx| {
                    entity.update(cx, |this, cx| {
                        this.show_get_info(entry_clone.clone(), cx);
                    });
                }),
        );
    }

    menu
}

fn build_background_context_menu(
    menu: PopupMenu,
    dir_path: &Path,
    entity: Entity<FileListView>,
) -> PopupMenu {
    let dir = dir_path.to_path_buf();
    let in_trash = is_in_trash(&dir);
    let mut menu = menu;

    if in_trash {
        let entity = entity.clone();
        let dir_clone = dir.clone();
        menu = menu.item(
            PopupMenuItem::new("Empty Trash")
                .icon(IconName::Delete)
                .on_click(move |_, _window, cx| {
                    let dir_to_clean = dir_clone.clone();
                    cx.background_spawn(async move {
                        if let Ok(entries) = std::fs::read_dir(&dir_to_clean) {
                            for entry in entries.flatten() {
                                let p = entry.path();
                                if p.is_dir() {
                                    let _ = std::fs::remove_dir_all(&p);
                                } else {
                                    let _ = std::fs::remove_file(&p);
                                }
                            }
                        }
                    })
                    .detach();
                    entity.update(cx, |this, cx| {
                        this.refresh(cx);
                    });
                }),
        );
        menu = menu.separator();
    } else {
        // 1. New Folder
        {
            let dir = dir.clone();
            let entity = entity.clone();
            menu = menu.item(
                PopupMenuItem::new("New Folder")
                    .icon(IconName::Plus)
                    .on_click(move |_, window, cx| {
                        entity.update(cx, |this, cx| {
                            this.prompt_new_folder(dir.clone(), window, cx);
                        });
                    }),
            );
        }

        // 2. New File
        {
            let dir = dir.clone();
            let entity = entity.clone();
            menu = menu.item(
                PopupMenuItem::new("New File")
                    .icon(IconName::Plus)
                    .on_click(move |_, window, cx| {
                        entity.update(cx, |this, cx| {
                            this.prompt_new_file(dir.clone(), window, cx);
                        });
                    }),
            );
        }

        menu = menu.separator();
    }

    // Open in Terminal
    {
        let dir = dir.clone();
        menu = menu.item(
            PopupMenuItem::new("Open in Terminal")
                .icon(IconName::SquareTerminal)
                .on_click(move |_, _window, _cx| {
                    open_in_terminal(&dir);
                }),
        );
    }

    // Copy Path
    {
        let path_str = dir.to_string_lossy().to_string();
        menu = menu.item(
            PopupMenuItem::new("Copy Path")
                .icon(IconName::Copy)
                .on_click(move |_, _window, cx| {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(path_str.clone()));
                }),
        );
    }

    menu = menu.separator();

    // Refresh
    {
        let entity = entity.clone();
        menu = menu.item(
            PopupMenuItem::new("Refresh")
                .icon(IconName::Redo)
                .on_click(move |_, _window, cx| {
                    entity.update(cx, |this, cx| {
                        this.refresh(cx);
                    });
                }),
        );
    }

    // Show / Hide Hidden Files
    {
        let entity = entity.clone();
        menu = menu.item(
            PopupMenuItem::new("Toggle Hidden Files")
                .icon(IconName::Eye)
                .on_click(move |_, _window, cx| {
                    entity.update(cx, |this, cx| {
                        this.toggle_hidden(cx);
                    });
                }),
        );
    }

    // Select All
    {
        let entity = entity;
        menu = menu.item(
            PopupMenuItem::new("Select All")
                .icon(IconName::Check)
                .on_click(move |_, _window, cx| {
                    entity.update(cx, |this, cx| {
                        this.select_all(cx);
                    });
                }),
        );
    }

    menu
}

fn render_list_row(
    entry: &FileEntry,
    col_ix: usize,
    entry_ix: usize,
    is_selected: bool,
    entity: Entity<FileListView>,
    deletion_progress: Option<f32>,
    theme: ListRowTheme,
) -> AnyElement {
    let icon = match entry.kind {
        FileKind::Directory => IconName::Folder,
        FileKind::File => IconName::File,
        FileKind::Symlink => IconName::ArrowRight,
    };

    let (row_height, row_opacity) = if let Some(p) = deletion_progress {
        let p = p.clamp(0.0, 1.0);
        let collapse = 1.0 - p * p;
        ((LIST_ROW_HEIGHT * collapse).max(0.0), (1.0 - p).max(0.0))
    } else {
        (LIST_ROW_HEIGHT, 1.0)
    };

    let is_hidden = entry.name.starts_with('.');
    let name: SharedString = entry.name.clone().into();

    let drag_value = FileDrag {
        path: entry.path.clone(),
        name: entry.name.clone(),
        col_ix,
        entry_ix,
        is_dir: entry.is_dir(),
    };
    let entity_for_drag = entity.clone();
    let entity_for_right = entity.clone();
    let entity_for_click = entity.clone();
    let entry_context = entry.clone();
    let entity_for_menu = entity;

    let icon_color = if is_selected {
        theme.selection
    } else if entry.is_dir() {
        theme.accent
    } else {
        theme.muted_foreground
    };

    let name_color = if is_selected {
        theme.foreground
    } else if is_hidden {
        theme.muted_foreground
    } else {
        theme.foreground
    };

    let row_bg = if is_selected {
        theme.accent.alpha(0.22)
    } else if entry_ix % 2 == 1 {
        theme.muted.alpha(0.03)
    } else {
        gpui::transparent_black()
    };

    let name_str = entry.name.clone();
    let is_dir = entry.is_dir();

    let row = h_flex()
        .id(("list-item", col_ix * 100000 + entry_ix))
        .items_center()
        .w_full()
        .h(gpui::px(row_height))
        .flex_shrink_0()
        .text_sm()
        .opacity(if is_hidden { 0.72 * row_opacity } else { row_opacity })
        .overflow_hidden()
        .bg(row_bg)
        .when(!is_selected, |this| {
            this.hover(|this| this.bg(theme.muted.alpha(0.08)))
        })
        .when(is_selected, |this| {
            this.border_l_2().border_color(theme.accent)
        })
        .child(
            h_flex()
                .flex_1()
                .items_center()
                .gap_2()
                .px_3()
                .overflow_hidden()
                .child(Icon::new(icon).small().flex_shrink_0().text_color(icon_color))
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .text_color(name_color)
                        .when(entry.is_dir(), |this| {
                            this.font_weight(gpui::FontWeight::MEDIUM)
                        })
                        .child(name),
                ),
        )
        .child(
            div()
                .w(gpui::px(LIST_COL_DATE))
                .px_3()
                .truncate()
                .flex_shrink_0()
                .text_color(theme.muted_foreground)
                .child(gpui::SharedString::from(entry.date_label())),
        )
        .child(
            div()
                .w(gpui::px(LIST_COL_SIZE))
                .px_3()
                .truncate()
                .text_right()
                .flex_shrink_0()
                .text_color(theme.muted_foreground)
                .child(gpui::SharedString::from(entry.size_label())),
        )
        .child(
            div()
                .w(gpui::px(LIST_COL_KIND))
                .px_3()
                .truncate()
                .flex_shrink_0()
                .text_color(theme.muted_foreground)
                .child(entry.kind_label()),
        )
        .on_drag(drag_value.clone(), move |_, _, _, cx| {
            entity_for_drag.update(cx, |this, _cx| {
                this.dragged_path = Some(drag_value.path.clone());
            });
            cx.new(|_| FileDragGhost {
                name: name_str.clone(),
                is_dir,
            })
        })
        .on_click(move |_, _window, cx| {
            entity_for_click.update(cx, |this, cx| {
                this.select_in_column(col_ix, entry_ix, cx);
            });
        })
        .on_mouse_down(
            gpui::MouseButton::Right,
            move |_event: &gpui::MouseDownEvent, _window, cx| {
                cx.stop_propagation();
                entity_for_right.update(cx, |this, cx| {
                    this.select_in_column(col_ix, entry_ix, cx);
                });
            },
        );

    row.context_menu(move |menu, _window, _cx| {
        build_item_context_menu(
            menu,
            &entry_context,
            col_ix,
            entry_ix,
            entity_for_menu.clone(),
        )
    })
    .into_any_element()
}

fn render_row(entry: &FileEntry, context: RowRenderContext) -> AnyElement {
    let RowRenderContext {
        col_ix,
        entry_ix,
        is_selected,
        entity,
        deletion_progress,
        muted_foreground,
        accent,
        muted,
    } = context;

    let icon = match entry.kind {
        FileKind::Directory => IconName::Folder,
        FileKind::File => IconName::File,
        FileKind::Symlink => IconName::ArrowRight,
    };

    let (row_height, row_opacity) = if let Some(p) = deletion_progress {
        let p = p.clamp(0.0, 1.0);
        let collapse = 1.0 - p * p;
        ((ROW_HEIGHT * collapse).max(0.0), (1.0 - p).max(0.0))
    } else {
        (ROW_HEIGHT, 1.0)
    };

    let name: SharedString = entry.name.clone().into();

    let drag_value = FileDrag {
        path: entry.path.clone(),
        name: entry.name.clone(),
        col_ix,
        entry_ix,
        is_dir: entry.is_dir(),
    };
    let entity_for_drag = entity.clone();
    let entity_for_right = entity.clone();
    let entity_for_click = entity.clone();
    let entry_context = entry.clone();
    let entity_for_menu = entity;

    let icon_color = if is_selected {
        accent
    } else if entry.is_dir() {
        accent
    } else {
        muted_foreground
    };

    let row_bg = if is_selected {
        accent.alpha(0.22)
    } else {
        gpui::transparent_black()
    };

    let name_str = entry.name.clone();
    let is_dir = entry.is_dir();

    let row = h_flex()
        .id(("item", col_ix * 100000 + entry_ix))
        .items_center()
        .gap_2()
        .px_3()
        .h(gpui::px(row_height))
        .opacity(row_opacity)
        .rounded_sm()
        .text_sm()
        .overflow_hidden()
        .bg(row_bg)
        .when(!is_selected, |this| {
            this.hover(|this| this.bg(muted.alpha(0.08)))
        })
        .when(is_selected, |this| {
            this.border_l_2().border_color(accent)
        })
        .child(Icon::new(icon).small().flex_shrink_0().text_color(icon_color))
        .child(div().flex_1().truncate().child(name))
        .when(entry.has_children, |this| {
            this.child(Icon::new(IconName::ChevronRight).small().flex_shrink_0().text_color(muted_foreground.alpha(0.7)))
        })
        .on_drag(drag_value.clone(), move |_, _, _, cx| {
            entity_for_drag.update(cx, |this, _cx| {
                this.dragged_path = Some(drag_value.path.clone());
            });
            cx.new(|_| FileDragGhost {
                name: name_str.clone(),
                is_dir,
            })
        })
        .on_click(move |_, _window, cx| {
            entity_for_click.update(cx, |this, cx| {
                this.select_in_column(col_ix, entry_ix, cx);
            });
        })
        .on_mouse_down(
            gpui::MouseButton::Right,
            move |_event: &gpui::MouseDownEvent, _window, cx| {
                cx.stop_propagation();
                entity_for_right.update(cx, |this, cx| {
                    this.select_in_column(col_ix, entry_ix, cx);
                });
            },
        );

    row.context_menu(move |menu, _window, _cx| {
        build_item_context_menu(
            menu,
            &entry_context,
            col_ix,
            entry_ix,
            entity_for_menu.clone(),
        )
    })
    .into_any_element()
}

fn render_search_item(
    ix: usize,
    entry: &FileEntry,
    entity: Entity<FileListView>,
    theme: &gpui_component::Theme,
) -> AnyElement {
    let icon = match entry.kind {
        FileKind::Directory => IconName::Folder,
        FileKind::File => IconName::File,
        FileKind::Symlink => IconName::ArrowRight,
    };
    let name: SharedString = entry.name.clone().into();
    let path = entry.path.clone();
    let full_path_str: SharedString = path.to_string_lossy().to_string().into();

    h_flex()
        .id(("search-item", ix))
        .items_center()
        .gap_2()
        .px_3()
        .h(gpui::px(ROW_HEIGHT))
        .text_sm()
        .hover(|this| this.bg(theme.muted.alpha(0.08)))
        .child(
            Icon::new(icon)
                .small()
                .flex_shrink_0()
                .text_color(if entry.is_dir() { theme.accent } else { theme.muted_foreground }),
        )
        .child(
            div()
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(name),
        )
        .child(
            div()
                .flex_1()
                .truncate()
                .text_xs()
                .text_color(theme.muted_foreground.alpha(0.8))
                .child(full_path_str),
        )
        .on_click(move |_, window, cx| {
            entity.update(cx, |this, cx| {
                this.open_search_result(path.clone(), window, cx);
            });
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn make_test_dir(name: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(format!("kaze_test_{}", name));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("file_a.txt"), "a").unwrap();
        fs::write(tmp.join("file_b.txt"), "bb").unwrap();
        fs::create_dir_all(tmp.join("subdir")).unwrap();
        fs::write(tmp.join("subdir").join("inner.txt"), "inner").unwrap();
        tmp
    }

    #[test]
    fn test_scan_dir_finds_files() {
        let tmp = make_test_dir("scan");
        let entries = scan_dir(&tmp, false, SortColumn::Name, SortDirection::Ascending);
        assert!(
            entries.len() >= 3,
            "expected at least 3 entries, got {}",
            entries.len()
        );
        assert!(entries[0].is_dir(), "first entry should be a directory");
        assert!(entries[0].has_children, "subdir should have children");
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_empty_dir_has_no_children() {
        let tmp = std::env::temp_dir().join("kaze_empty_unique");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let entries = scan_dir(&tmp, false, SortColumn::Name, SortDirection::Ascending);
        assert!(entries.is_empty(), "empty dir should have no entries");
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_unique_child_path_uses_a_numbered_suffix() {
        let tmp = make_test_dir("new_folder");
        fs::create_dir(tmp.join("Untitled Folder")).unwrap();

        assert_eq!(
            unique_child_path(&tmp, "Untitled Folder"),
            tmp.join("Untitled Folder 2")
        );

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_unique_file_path() {
        let tmp = make_test_dir("new_file");
        fs::write(tmp.join("Untitled.txt"), "test").unwrap();

        assert_eq!(
            unique_file_path(&tmp, "Untitled", "txt"),
            tmp.join("Untitled 2.txt")
        );

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_duplicate_file_and_folder() {
        let tmp = make_test_dir("dup");
        let file_path = tmp.join("file_a.txt");
        let dup_file = duplicate_path(&file_path).unwrap();
        assert_eq!(dup_file, tmp.join("file_a copy.txt"));
        assert!(dup_file.exists());

        let dup_file_2 = duplicate_path(&file_path).unwrap();
        assert_eq!(dup_file_2, tmp.join("file_a copy 2.txt"));
        assert!(dup_file_2.exists());

        let dir_path = tmp.join("subdir");
        let dup_dir = duplicate_path(&dir_path).unwrap();
        assert_eq!(dup_dir, tmp.join("subdir copy"));
        assert!(dup_dir.join("inner.txt").exists());

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_sort_entries() {
        let tmp = make_test_dir("sorting");
        let mut entries = scan_dir(&tmp, false, SortColumn::Name, SortDirection::Ascending);
        assert_eq!(entries[0].name, "subdir"); // folder first

        sort_entries(&mut entries, SortColumn::Size, SortDirection::Descending);
        // Folder still first, then file_b (2 bytes) before file_a (1 byte)
        assert_eq!(entries[0].name, "subdir");
        assert_eq!(entries[1].name, "file_b.txt");
        assert_eq!(entries[2].name, "file_a.txt");

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_file_entry_has_children_flag() {
        let tmp = make_test_dir("children_flag");
        let entries = scan_dir(&tmp, false, SortColumn::Name, SortDirection::Ascending);
        for entry in &entries {
            if entry.is_dir() {
                assert!(
                    entry.has_children,
                    "dir {} should have has_children=true",
                    entry.name
                );
            } else {
                assert!(
                    !entry.has_children,
                    "file {} should have has_children=false",
                    entry.name
                );
            }
        }
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_move_to_trash_removes_file() {
        let tmp = make_test_dir("trash");
        let file_path = tmp.join("file_a.txt");
        assert!(file_path.exists());
        assert!(kaze_shared::move_to_trash(&file_path).is_ok());
        assert!(!file_path.exists(), "file should be gone after trash");
        let trash = std::env::var("HOME")
            .map(|h| PathBuf::from(h).join(".local/share/Trash/files"))
            .unwrap();
        let _ = fs::remove_file(trash.join("file_a.txt"));
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_hidden_files_filtered() {
        let tmp = make_test_dir("hidden");
        fs::write(tmp.join(".hidden"), "secret").unwrap();
        let entries = scan_dir(&tmp, false, SortColumn::Name, SortDirection::Ascending);
        assert!(
            !entries.iter().any(|e| e.name == ".hidden"),
            "hidden file should be filtered"
        );
        let entries_with_hidden = scan_dir(&tmp, true, SortColumn::Name, SortDirection::Ascending);
        assert!(
            entries_with_hidden.iter().any(|e| e.name == ".hidden"),
            "hidden file should show with show_hidden=true"
        );
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn vertical_wheel_does_not_pan_the_columns() {
        assert_eq!(horizontal_wheel_delta(0.0, 24.0, false), None);
    }

    #[test]
    fn horizontal_gestures_still_pan_the_columns() {
        assert_eq!(horizontal_wheel_delta(24.0, 0.0, false), Some(24.0));
        assert_eq!(horizontal_wheel_delta(0.0, 24.0, true), Some(24.0));
    }
}
