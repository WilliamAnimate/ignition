use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
use eframe::{egui, App, NativeOptions};
use egui_extras::install_image_loaders;
use eyre::{Context, ContextCompat};
use fork::{daemon, Fork};
use splinter_icon::icon;
use tracing::level_filters::LevelFilter;
use tracing::{debug, error, info, warn};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::colors::Colors;
use crate::fonts::load_fonts;
use crate::icons::IconManager;
use crate::search::{SearchEngine, SearchQuery};
use crate::shortcuts::{Shortcut, ShortcutId, ShortcutManager};

mod colors;
mod config;
mod fonts;
mod icons;
mod search;
mod shortcuts;

struct ApplicationLaunch {
    exec: String,
}

fn main() -> eyre::Result<()> {
    let to_launch: Arc<Mutex<Option<ApplicationLaunch>>> = Arc::new(Mutex::new(None));

    let cache_dir = cache_dir()
        .wrap_err("Failed to find cache dir")?
        .join("ignition");
    let data_local_dir = data_local_dir()
        .wrap_err("Failed to find data local dir")?
        .join("ignition");

    {
        let start = Instant::now();
        let filter = EnvFilter::from_default_env().add_directive("wgpu_core=error".parse()?);
        tracing_subscriber::fmt()
            .compact()
            .with_env_filter(filter)
            .with_max_level(LevelFilter::INFO)
            .finish()
            .init();

        info!("Initializing core");
        let mut icons =
            IconManager::new(&cache_dir).wrap_err("Failed to initialize IconManager")?;
        let search =
            SearchEngine::new(&data_local_dir).wrap_err("Failed to initialize SearchEngine")?;
        let shortcuts = ShortcutManager::new().wrap_err("Failed to initialize ShortcutManager")?;

        info!("Loading icons");
        for shortcut in shortcuts.shortcuts.values() {
            icons.prepare_icon(shortcut);
        }
        icons.save().wrap_err("Failed to save icons")?;

        info!("Initialized core in {:?}", start.elapsed());
        info!("Launching ui");
        let to_launch_c = to_launch.clone();
        eframe::run_native(
            "Ignition",
            NativeOptions {
                viewport: ViewportBuilder {
                    transparent: Some(true),
                    decorations: Some(false),
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
                let rankings: Vec<ShortcutId> = shortcuts.shortcuts.keys().cloned().collect();
                let mut application = Application {
                    start: Some(start),
                    to_launch: to_launch_c,
                    shortcuts,
                    last_top: rankings.first().cloned().unwrap(),
                    last_top_at: Instant::now(),
                    rankings,
                    query: "".to_string(),
                    icons,
                    selected: None,
                    search,
                    block_case: true,
                    has_focused: false,
                    mouse_lock_from: Instant::now(),
                    first_focused_at: Instant::now(),
                };
                application.search("");
                application.sort_rankings();

                Ok(Box::new(application))
            }),
        )?;
    }

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
pub fn draw_icon(painter: &Painter, icon: u32, pos: Pos2, size: f32, color: Color32) {
    let icon = char::from_u32(icon).expect("Could not parse icon char");
    let text = icon.to_string();

    let font_id = FontId::new(size, FontFamily::Name("Icons".into()));
    let job = LayoutJob::simple(text, font_id.clone(), color, f32::INFINITY);
    let arc = painter.ctx().fonts(|fonts| fonts.layout_job(job));

    let mut rect = Align2::CENTER_CENTER.anchor_rect(Rect::from_min_size(pos, arc.rect.size()));

    let data = painter
        .ctx()
        .fonts(|fonts| fonts.glyph_width(&font_id, icon));

    rect.set_height(data);
    painter.galley(rect.min, arc, color);
}

pub struct Application {
    start: Option<Instant>,
    to_launch: Arc<Mutex<Option<ApplicationLaunch>>>,

    shortcuts: ShortcutManager,
    icons: IconManager,
    search: SearchEngine,
    rankings: Vec<ShortcutId>,
    query: String,

    last_top: ShortcutId,
    last_top_at: Instant,
    mouse_lock_from: Instant,

    selected: Option<usize>,

    has_focused: bool,
    first_focused_at: Instant,

    block_case: bool,
}

const ENTRY_HEIGHT: f32 = 32.0;
const ENTRY_SPACING: f32 = 8.0;
const IMAGE_SIZE: f32 = 24.0;
impl Application {
    pub fn search(&mut self, query: &str) {
        let mut query = query.to_string();
        if self.block_case {
            query = query.to_lowercase();
        }
        self.mouse_lock_from = Instant::now();

        let start = Instant::now();

        let search_query = SearchQuery::from(query.to_string());
        for entry in &mut self.shortcuts.shortcuts.values_mut() {
            let search = self.search.score(entry, &search_query);
            entry.score = search;
        }

        self.sort_rankings();

        debug!("Search \"{query}\" took {:?}", start.elapsed());
    }

    fn sort_rankings(&mut self) {
        let mut scores: Vec<&Shortcut> = self.shortcuts.shortcuts.values().collect();
        scores.sort_by(|e0, e1| {
            e1.score
                .score
                .total_cmp(&e0.score.score)
                .then(e0.name.cmp(&e1.name))
        });
        let rankings = scores.into_iter().map(|v| v.id.clone()).collect();
        self.rankings = rankings;

        let top = self.rankings.first().cloned().unwrap();
        if self.last_top != top {
            self.last_top = top;
            self.last_top_at = Instant::now();
        }
    }

    pub fn draw_search_bar(&mut self, ui: &mut Ui) {
        let mut rect = ui.clip_rect();
        rect.set_height(64.0);

        let p = ui.painter();

        let mut message: Option<(String, Color32)> = None;
        if !self.block_case {
            message = Some(("Case-sensitive".to_string(), Colors::PEACH));
        } else if self.query.chars().any(|v| v.is_uppercase()) {
            message = Some(("CapsLock Ignored".to_string(), Colors::YELLOW));
        }

        if let Some((text, color)) = message {
            let text_rect = p.text(
                rect.shrink2(Vec2::new(16.0 + 8.0, 0.0)).right_center(),
                Align2::RIGHT_CENTER,
                text,
                FontId::new(15.0, FontFamily::Proportional),
                color,
            );
            let text_rect = text_rect.expand2(Vec2::new(10.0, 2.0));
            p.rect_filled(
                text_rect,
                Rounding::same(text_rect.height() / 2.0),
                color.gamma_multiply(0.2),
            );
        }

        p.line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            Stroke::new(1.0, Colors::SUBTEXT0.gamma_multiply(0.3)),
        );

        let font = FontId::new(18.0, FontFamily::Proportional);
        let output = TextEdit::singleline(&mut self.query)
            .frame(false)
            .vertical_align(Align::Center)
            .text_color(Colors::TEXT)
            .font(FontSelection::FontId(font.clone()))
            .margin(Margin {
                left: 8.0 + 32.0 + 6.0,
                right: 8.0,
                top: 0.0,
                bottom: 0.0,
            })
            .min_size(rect.size())
            .show(ui);

        ui.memory_mut(|memory| {
            memory.request_focus(output.response.id);
        });

        if output.response.changed() {
            let query = &self.query.clone();
            if query.trim().is_empty() {
                self.selected = None;
            } else {
                self.selected = Some(0);
            }
            self.search(query);
        }

        let p = ui.painter();

        draw_icon(
            p,
            icon!("search"),
            rect.left_center() + Vec2::new(18.0 + 12.0, -1.0),
            18.0,
            Colors::TEXT,
        );

        if self.query.is_empty() {
            p.text(
                output.text_clip_rect.left_center(),
                Align2::LEFT_CENTER,
                "Search for a program",
                font,
                Colors::SUBTEXT0,
            );
        }
    }
    pub fn draw_entries(&mut self, ui: &mut Ui) {
        let row_height = ENTRY_HEIGHT + ENTRY_SPACING;
        let num_rows = self.rankings.len();

        let rect = ui.max_rect();
        ScrollArea::vertical()
            .max_height(row_height * num_rows as f32)
            .enable_scrolling(false)
            .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
            .show_viewport(ui, |ui, viewport| {
                ui.set_height(row_height);

                let first_item = (viewport.min.y / row_height).floor().at_least(0.0f32) as usize;
                let last_item = (viewport.max.y / row_height).ceil() as usize + 1;
                let last_item = last_item.at_most(num_rows);

                let mut used_rect = Rect::NOTHING;

                let min_rect = ui.min_rect();

                let selected_f32 = self.selected.map(|v| v as f32).unwrap_or(-1.0);

                let opacity_t = ui.ctx().animate_value_with_time(
                    ui.id().with("animated-selected-opacity"),
                    (selected_f32 + 1.0).clamp(0.0, 1.0),
                    0.15,
                );

                let t = ui.ctx().animate_value_with_time(
                    ui.id().with("animated-selected"),
                    selected_f32.max(0.0),
                    if opacity_t == 0.0 && selected_f32 >= 0.0 {
                        // We skip animating if we are invisible
                        0.0
                    } else {
                        0.15
                    },
                );

                let selected_opacity = easing::cubic_out(opacity_t);
                let mut current_selected = t;

                current_selected = current_selected.floor()
                    + if current_selected < selected_f32 {
                        easing::cubic_out(current_selected.fract())
                    } else {
                        easing::cubic_in(current_selected.fract())
                    };
                current_selected = min_rect.top() + current_selected * row_height + 9.0;
                let highlight_rect = {
                    let mut rect = ui.clip_rect().shrink(8.0);
                    rect.min.y = current_selected - 4.0;
                    rect.max.y = current_selected + ENTRY_HEIGHT + 4.0;
                    rect
                };

                {
                    let p = ui.painter();

                    ui.scroll_to_rect(highlight_rect.expand2(Vec2::new(0.0, 16.0)), None);
                    p.rect(
                        highlight_rect,
                        Rounding::same(6.0),
                        Colors::BG.gamma_multiply(selected_opacity),
                        Stroke::new(0.0, Colors::SURFACE0),
                    );
                    draw_icon(
                        p,
                        icon!("play_arrow"),
                        highlight_rect.right_center() - Vec2::new(20.0, 0.0),
                        24.0,
                        Colors::SUBTEXT0.gamma_multiply(selected_opacity),
                    );
                }
                let top_score = self
                    .rankings
                    .first()
                    .and_then(|v| self.shortcuts.shortcuts.get(v))
                    .map(|v| v.score.score)
                    .unwrap_or(1.0);
                //let mut selected_rect = None;

                let mut hit_boxes = Vec::new();
                for i in first_item..last_item {
                    let id = &self.rankings[i];
                    let entry = &self.shortcuts.shortcuts[id];
                    let x = min_rect.left();
                    let y = min_rect.top() + i as f32 * row_height + 9.0;

                    let rect = Rect::from_min_size(
                        Pos2::new(x, y),
                        Vec2::new(rect.width() - 24.0, row_height),
                    );

                    let panel_rect = {
                        let mut rect = rect;
                        rect.set_height(ENTRY_HEIGHT);
                        rect
                    };

                    let mut opacity =
                        0.4 + (entry.score.score.max(0.01) / top_score.max(0.01)) * 0.6;
                    if self.query.is_empty() {
                        opacity = 1.0;
                    }

                    let mut selected_t = (highlight_rect.intersect(panel_rect).height()
                        / ENTRY_HEIGHT)
                        .clamp(0.0, 1.0);
                    let mut selected_t2 =
                        (highlight_rect.intersect(panel_rect.expand(16.0)).height() / ENTRY_HEIGHT)
                            .clamp(0.0, 1.0);

                    selected_t *= selected_opacity;
                    selected_t2 *= selected_opacity;

                    self.draw_entry(
                        ui,
                        panel_rect,
                        selected_t,
                        opacity.max(selected_t2).clamp(0.0, 1.0),
                        entry,
                    );

                    hit_boxes.push((panel_rect, id.clone()));
                    used_rect = used_rect.union(rect);
                }

                ui.input(|input| {
                    if let Some(pos) = input.pointer.hover_pos() {
                        if self.mouse_lock_from.elapsed() > Duration::from_millis(300) {
                            for (rect, id) in &hit_boxes {
                                let rect = rect.expand2(Vec2::new(0.0, ENTRY_SPACING / 2.0));
                                if rect.contains(pos) {
                                    let Some((id, _)) =
                                        self.rankings.iter().enumerate().find(|(_, v)| *v == id)
                                    else {
                                        continue;
                                    };

                                    if input.pointer.is_moving() {
                                        self.selected = Some(id);
                                    }

                                    // If we press on an entry, we select it
                                    if input.pointer.primary_down() {
                                        self.open_selected();
                                    }
                                }
                            }
                        }
                    }
                });

                ui.allocate_rect(used_rect, Sense::click());
            });
    }
    pub fn draw_entry(
        &self,
        ui: &mut Ui,
        mut rect: Rect,
        selected: f32,
        opacity: f32,
        entry: &Shortcut,
    ) -> Rect {
        let bg_rect = rect.expand(3.0);

        rect.max.x -= 4.0;
        rect.min.x += 10.0;
        rect = rect.shrink2(Vec2::new(1.0, 0.0));

        let image_width = rect.height();
        if let Some(icon) = self.icons.read_icon(&entry.id) {
            let string = format!("file://{}", icon.to_str().unwrap());
            let image = egui::Image::from_uri(string)
                .tint(Color32::WHITE.gamma_multiply(opacity))
                .rounding(Rounding::same(4.0));
            let mut image_rect = rect;
            image_rect.set_width(image_width);
            image.paint_at(
                ui,
                Rect::from_center_size(image_rect.center(), Vec2::splat(IMAGE_SIZE)),
            );
        }

        let text_color = Colors::SUBTEXT0
            .lerp_to_gamma(Colors::TEXT, selected)
            .gamma_multiply(opacity);
        rect = rect.with_min_x(rect.min.x + image_width + 2.0);

        let p = ui.painter();

        let font = FontId::new(18.0, FontFamily::Proportional);
        let galley = ui.ctx().fonts(|fonts| {
            let mut job = LayoutJob {
                wrap: TextWrapping::truncate_at_width(rect.width() - 12.0),
                ..LayoutJob::default()
            };

            for (i, char) in entry.name.chars().enumerate() {
                let value = entry.score.indices.get(&i).unwrap_or(&0.0);
                let value = if *value > 0.0 { *value } else { 0.0 };
                let text_color = text_color.lerp_to_gamma(Colors::ROSEWATER, value);
                job.append(
                    &char.to_string(),
                    0.0,
                    TextFormat {
                        font_id: font.clone(),

                        color: text_color,
                        ..TextFormat::default()
                    },
                )
            }

            // DEBUG
            //{
            //    job.append(
            //        &format!(
            //            "{}: {}",
            //            self.search.get_popularity(&entry.id),
            //            entry.score.score
            //        ),
            //        8.0,
            //        TextFormat {
            //            font_id: font.clone(),
            //            color: text_color,
            //            ..TextFormat::default()
            //        },
            //    )
            //}

            if let Some(comment) = entry.comment.as_ref() {
                if selected > 0.0 {
                    job.append(
                        &format!(" {comment}"),
                        8.0,
                        TextFormat {
                            font_id: font.clone(),
                            color: text_color.gamma_multiply(0.5 * selected),
                            ..TextFormat::default()
                        },
                    )
                }
            }
            fonts.layout_job(job)
        });

        {
            let rect = Align2::LEFT_CENTER.anchor_size(rect.left_center(), galley.size());
            p.galley(rect.min, galley, Color32::RED);
        }

        bg_rect
    }

    fn open_selected(&mut self) {
        let Some(selected) = self.selected else {
            return;
        };
        let id = &self.rankings[selected];
        let shortcut = self.shortcuts.shortcuts.get(id).unwrap();

        let duration = self.last_top_at.elapsed();
        if selected == 0 && duration < Duration::from_millis(150) {
            warn!(
                "Blocked launch of {} because it changed too quickly ({duration:?} < 150ms)",
                shortcut.name
            );
            return;
        }
        let buf = shortcut.path.canonicalize().unwrap();
        let file_path = buf.to_str().unwrap();
        let launch = ApplicationLaunch {
            exec: file_path.to_string(),
        };

        let mut to_launch = self.to_launch.lock().unwrap();
        *to_launch = Some(launch);
        self.search.record_use(id.clone()).unwrap();
    }
}

impl App for Application {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
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
                            self.open_selected();
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
                            } else {
                                self.selected = Some(0);
                            }
                        } else {
                            // decrease
                            if let Some(value) = &mut self.selected {
                                if *value == 0 {
                                    self.selected = None;
                                } else {
                                    *value = value.saturating_sub(1);
                                }
                            }
                        }
                    }

                    if let Some(value) = &mut self.selected {
                        *value = (*value).clamp(0, self.rankings.len() - 1);
                    }
                });

                self.draw_search_bar(ui);
                self.draw_entries(ui);

                ui.input(|input| {
                    if self.query.trim().is_empty() {
                        self.block_case = true;
                    } else if input.modifiers.shift && !input.keys_down.is_empty() {
                        self.block_case = false;
                    }
                });
            });

        let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("Border")));
        painter.rect_stroke(
            rect,
            Rounding::same(16.0),
            Stroke::new(2.0, Colors::SURFACE0),
        );

        if focused && !self.has_focused {
            self.has_focused = true;
            self.first_focused_at = Instant::now();
        }

        let is_long_enough = self.first_focused_at.elapsed().as_secs_f32() > 0.2;
        if self.to_launch.lock().unwrap().is_some()
            || should_close
            || (!focused && self.has_focused && is_long_enough)
        {
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            });
        }
    }

    fn clear_color(&self, _visuals: &Visuals) -> [f32; 4] {
        Colors::CRUST
            .linear_multiply(0.75)
            .to_normalized_gamma_f32()
    }
}
