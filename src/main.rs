use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::apps::icons::AppIconManager;
use crate::apps::{App, AppId, AppManager};
use crate::search::{SearchEngine, SearchQuery, SearchResult, SearchResultEntry};
use crate::ui::results::{ResultsEvent, ResultsWidget};
use crate::ui::search_bar::{SearchBarMessage, SearchBarWidget};
use dirs::{cache_dir, data_local_dir};
use eframe::egui::scroll_area::ScrollBarVisibility;
use eframe::egui::style::{Spacing, TextCursorStyle};
use eframe::egui::text::LayoutJob;
use eframe::egui::{
    Align, Align2, CentralPanel, Color32, Event, FontId, FontSelection, Frame, Id, Key, LayerId,
    Margin, NumExt, Order, Painter, Pos2, Rect, Rounding, ScrollArea, Sense, Shadow, Stroke, Style,
    TextEdit, TextFormat, Ui, Vec2, ViewportBuilder, Visuals, X11WindowType,
};
use eframe::emath::easing;
use eframe::epaint::text::TextWrapping;
use eframe::epaint::FontFamily;
use eframe::{egui, NativeOptions};
use egui_extras::install_image_loaders;
use eyre::{Context, ContextCompat};
use fork::{daemon, Fork};
use splinter_icon::icon;
use tracing::level_filters::LevelFilter;
use tracing::{debug, error, info, warn};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;
use ui::framework::draw_icon;
use ui::framework::load_fonts;
use ui::framework::Colors;

mod apps;
mod config;
mod search;
mod ui;

#[cfg(feature = "rounded_corners")]
const ROUNDED_CORNERS_LEVEL: f32 = 0.0;
#[cfg(not(feature = "rounded_corners"))]
const ROUNDED_CORNERS_LEVEL: f32 = 16.0;

struct ApplicationLaunch {
    exec: String,
}

fn main() -> eyre::Result<()> {
    let filter = EnvFilter::from_default_env().add_directive("wgpu_core=error".parse()?);
    tracing_subscriber::fmt()
        .compact()
        .with_env_filter(filter)
        .with_max_level(LevelFilter::INFO)
        .finish()
        .init();

    let start = Instant::now();
    let to_launch: Arc<Mutex<Option<ApplicationLaunch>>> = Arc::new(Mutex::new(None));

    let cache_dir = cache_dir()
        .wrap_err("Failed to find cache dir")?
        .join("ignition");
    let data_local_dir = data_local_dir()
        .wrap_err("Failed to find data local dir")?
        .join("ignition");

    info!("Initializing core");
    let apps = AppManager::new().wrap_err("Failed to initialize ShortcutManager")?;
    let mut icons = AppIconManager::new(&cache_dir).wrap_err("Failed to initialize IconManager")?;
    let search =
        SearchEngine::new(&data_local_dir).wrap_err("Failed to initialize SearchEngine")?;

    //icons.clear_icons();
    info!("Loading icons");
    for shortcut in apps.applications.values() {
        icons.prepare_icon(shortcut);
    }

    info!("Initialized core in {:?}", start.elapsed());
    info!("Launching ui");
    let to_launch_c = to_launch.clone();
    eframe::run_native(
        "Ignition",
        NativeOptions {
            viewport: ViewportBuilder {
                transparent: Some(true),
                decorations: Some(false),
                fullscreen: Some(false),
                maximized: Some(false),
                window_type: Some(X11WindowType::Utility),
                ..ViewportBuilder::default()
            },
            ..NativeOptions::default()
        },
        Box::new(move |context| {
            context.egui_ctx.set_style(Style {
                visuals: Visuals {
                    window_fill: Color32::TRANSPARENT,
                    panel_fill: Color32::TRANSPARENT,
                    window_shadow: Shadow::NONE,
                    text_cursor: TextCursorStyle {
                        stroke: Stroke::new(1.0, Colors::OVERLAY0),
                        blink: false,
                        ..TextCursorStyle::default()
                    },
                    ..Visuals::dark()
                },
                spacing: Spacing {
                    item_spacing: Vec2::new(16.0, 4.0),
                    ..Spacing::default()
                },
                ..Style::default()
            });
            context.egui_ctx.set_fonts(load_fonts());
            install_image_loaders(&context.egui_ctx);
            let mut application = Application {
                start: Some(start),
                to_launch: to_launch_c,
                apps,
                last_top: AppId::default(),
                last_top_at: Instant::now(),
                search_query: "".to_string(),
                search_result: SearchResult::default(),
                app_icons: icons,
                selected: Some(0),
                search,
                case_sensitive: false,
                has_window_ever_received_focus: false,
                mouse_lock_from: Instant::now(),
                first_focused_at: Instant::now(),
            };
            application.search("");

            Ok(Box::new(application))
        }),
    )?;

    let quard = to_launch.lock().expect("Failed to lock launch mutex.");
    if let Some(to_launch) = &*quard {
        info!("Launching {}", to_launch.exec);

        match daemon(false, false) {
            Ok(Fork::Child) => {
                let output = Command::new("gio")
                    .arg("launch")
                    .arg(&to_launch.exec)
                    .spawn()
                    .expect("failed to execute process");

                let output = output.wait_with_output().expect("Failed to run program");
                println!("{:?}", output.status);
                println!("{:?}", String::from_utf8(output.stdout));
                println!("{:?}", String::from_utf8(output.stderr));
            }
            Ok(Fork::Parent(_)) => {
                error!("Wrong forl");
            }
            Err(e) => {
                error!("Error {e}");
            }
        }
        println!("Launched");
    }
    Ok(())
}

pub struct Application {
    /// This is used to measure how long the application took to launch
    start: Option<Instant>,
    /// This is the mutex holding what application we will launch
    to_launch: Arc<Mutex<Option<ApplicationLaunch>>>,

    apps: AppManager,
    app_icons: AppIconManager,

    search: SearchEngine,
    search_query: String,
    search_result: SearchResult,

    last_top: AppId,
    last_top_at: Instant,

    /// Then you type on the keyboard, it will freeze the mouse for a given duration.
    mouse_lock_from: Instant,

    selected: Option<usize>,

    case_sensitive: bool,

    // These are to prevent the window from getting instantly closed on launch.
    // When your mouse is not instantly on the window.
    has_window_ever_received_focus: bool,
    first_focused_at: Instant,
}

const ENTRY_HEIGHT: f32 = 32.0;
const ENTRY_SPACING: f32 = 8.0;
const IMAGE_SIZE: f32 = 24.0;
impl Application {
    pub fn search(&mut self, query: &str) {
        let mut query = query.to_string();
        if !self.case_sensitive {
            query = query.to_lowercase();
        }
        self.mouse_lock_from = Instant::now();

        let start = Instant::now();

        let results = self.search.search(query.to_string(), &self.apps);
        let top = results
            .entries
            .first()
            .map(|v| v.id.clone())
            .unwrap_or_default();
        if self.last_top != top {
            self.last_top = top;
            self.last_top_at = Instant::now();
        }
        self.search_result = results;

        debug!("Search \"{query}\" took {:?}", start.elapsed());
    }

    pub fn draw_search_bar(&mut self, ui: &mut Ui) {
        let mut messages = Vec::new();
        if self.case_sensitive {
            messages.push(SearchBarMessage {
                text: "Case-sensitive".to_string(),
                color: Colors::PEACH,
            });
        } else if self.search_query.chars().any(|v| v.is_uppercase()) {
            messages.push(SearchBarMessage {
                text: "CapsLock Ignored".to_string(),
                color: Colors::YELLOW,
            });
        }

        let to_load_finished = self.app_icons.to_load_finished();
        let to_load = self.app_icons.to_load();
        if to_load != to_load_finished {
            messages.push(SearchBarMessage {
                text:  format!("Indexing Icons {to_load_finished}/{to_load} (this may freeze)"),
                color: Colors::BLUE,
            });
        }

        let output = SearchBarWidget {
            messages,
            query: &mut self.search_query,
        }
        .ui(ui);

        let mut case_changed = self.case_sensitive;
        ui.input(|input| {
            if self.search_query.trim().is_empty() {
                self.case_sensitive = false;
            } else if input.modifiers.shift {
                self.case_sensitive = true;
            }
        });
        case_changed = case_changed != self.case_sensitive;

        if output.response.changed() || case_changed {
            let query = &self.search_query.clone();
            if query.trim().is_empty() {
                self.selected = None;
            } else {
                self.selected = Some(0);
            }
            self.search(query);
        }
    }

    pub fn draw_entries(&mut self, ui: &mut Ui) {
        let events = ResultsWidget {
            apps: &self.apps,
            app_icons: &self.app_icons,
            results: &self.search_result,
            selected: self.selected,
        }
        .ui(ui);
        for event in events {
            match event {
                ResultsEvent::Hovered(app_id) => {
                    if self.mouse_lock_from.elapsed() > Duration::from_millis(300) {
                        let Some((idx, _)) = self
                            .search_result
                            .entries
                            .iter()
                            .enumerate()
                            .find(|(_, v)| v.id == app_id)
                        else {
                            continue;
                        };
                        self.selected = Some(idx);
                    }
                }
                ResultsEvent::Pressed(app) => {
                    self.open(app);
                }
            }
        }
    }

    fn selected(&self) -> Option<&AppId> {
        self.selected
            .and_then(|v| self.search_result.entries.get(v).map(|v| &v.id))
    }

    fn open(&mut self, id: AppId) {
        let Some(app) = self.apps.applications.get(&id) else {
            return;
        };

        let duration = self.last_top_at.elapsed();

        // If this is the top entry, we prevent opening (from the keyboard)
        // if the top entry changed quickly before the enter press. (to prevent misfires)
        if self.selected == Some(0)
            && self.selected() == Some(&id)
            && duration < Duration::from_millis(150)
        {
            warn!(
                "Blocked launch of {} because the stop entry changed too quickly ({duration:?} < 150ms)",
                app.name
            );
            return;
        }

        let buf = app.path.canonicalize().unwrap();
        let file_path = buf.to_str().unwrap();
        let launch = ApplicationLaunch {
            exec: file_path.to_string(),
        };

        let mut to_launch = self.to_launch.lock().unwrap();
        *to_launch = Some(launch);
        self.search.record_use(id).unwrap();
    }
}

impl eframe::App for Application {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if self.app_icons.tick() {
            ctx.request_repaint();
        }


        if let Some(start) = self.start.take() {
            info!("Initialized in {:?}", start.elapsed());
        }
        let focused = ctx.viewport(|v| v.input.focused);

        let mut should_close = false;
        let rect = ctx.available_rect();
        CentralPanel::default()
            .frame(Frame::none().inner_margin(Margin::symmetric(0.0, 0.0)))
            .show(ctx, |ui| {
                ui.input(|input| {
                    let mut to_offset = 0isize;
                    for event in &input.events {
                        if let Event::Key {
                            key: Key::Enter,
                            pressed: true,
                            ..
                        } = event
                        {
                            if let Some(selected) = self.selected() {
                                self.open(selected.clone());
                            }
                        };
                        if let Event::Key {
                            key: Key::Escape,
                            pressed: true,
                            ..
                        } = event
                        {
                            should_close = true;
                        };
                        if let Event::Key {
                            key: Key::ArrowDown,
                            pressed: true,
                            ..
                        } = event
                        {
                            to_offset += 1;
                        };
                        if let Event::Key {
                            key: Key::ArrowUp,
                            pressed: true,
                            ..
                        } = event
                        {
                            to_offset -= 1;
                        };
                        if let Event::Key {
                            key: Key::R,
                            pressed: true,
                            modifiers,
                            ..
                        } = event
                        {
                            if modifiers.ctrl {
                                self.app_icons.clear_icons();
                                for shortcut in self.apps.applications.values() {
                                    self.app_icons.prepare_icon(shortcut);
                                }
                            }
                        };
                        if let Event::MouseWheel { delta, .. } = event {
                            if delta.y > 0.0 {
                                to_offset -= 1;
                            } else {
                                to_offset += 1;
                            }
                            self.mouse_lock_from = Instant::now();
                        };
                    }

                    let increase = to_offset.signum() == 1;
                    for _ in 0..to_offset.abs() {
                        if increase {
                            if let Some(value) = &mut self.selected {
                                *value = value.saturating_add(1);
                                if *value == self.apps.applications.len() {
                                    // reached end, wrap back to first (zeroth) entry
                                    *value = 0;
                                }
                            } else {
                                self.selected = Some(0);
                            }
                        } else {
                            // decrease
                            if let Some(value) = &mut self.selected {
                                if *value == 0 {
                                    // reached beginning, wrap back to final entry
                                    self.selected = Some(self.apps.applications.len());
                                } else {
                                    *value = value.saturating_sub(1);
                                }
                            }
                        }
                    }

                    if let Some(value) = &mut self.selected {
                        *value = (*value).clamp(0, self.search_result.entries.len() - 1);
                    }
                });

                self.draw_search_bar(ui);
                self.draw_entries(ui);
            });

        let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("Border")));
        painter.rect_stroke(
            rect,
            Rounding::same(ROUNDED_CORNERS_LEVEL),
            Stroke::new(2.0, Colors::SURFACE0),
        );

        if focused && !self.has_window_ever_received_focus {
            self.has_window_ever_received_focus = true;
            self.first_focused_at = Instant::now();
        }

        if self.to_launch.lock().unwrap().is_some()
            || should_close
        {
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            });
        }
    }

    fn on_exit(&mut self) {
        self.app_icons.finish().unwrap();
    }

    fn clear_color(&self, _visuals: &Visuals) -> [f32; 4] {
        Colors::CRUST
            .linear_multiply(0.75)
            .to_normalized_gamma_f32()
    }
}
