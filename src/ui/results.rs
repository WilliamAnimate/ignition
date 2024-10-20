use eframe::egui;
use eframe::egui::{Align2, Color32, FontFamily, FontId, NumExt, Pos2, Rect, Rounding, ScrollArea, Sense, Stroke, TextFormat, Ui, Vec2};
use eframe::egui::scroll_area::ScrollBarVisibility;
use eframe::egui::text::{LayoutJob, TextWrapping};
use eframe::emath::easing;
use splinter_icon::icon;
use crate::{ENTRY_HEIGHT, ENTRY_SPACING, IMAGE_SIZE};
use crate::apps::{AppId, AppManager};
use crate::apps::icons::AppIconManager;
use crate::search::{SearchResult, SearchResultEntry};
use crate::ui::framework::{draw_icon, Colors};

pub enum ResultsEvent {
    Hovered(AppId),
    Pressed(AppId),
}
pub struct ResultsWidget<'a> {
    pub apps: &'a AppManager,
    pub app_icons: &'a AppIconManager,
    pub results: &'a SearchResult,
    pub selected: Option<usize>,
}

impl ResultsWidget<'_> {
    pub fn ui(&self, ui: &mut Ui) -> Vec<ResultsEvent> {
        let mut events = Vec::new();
        let row_height = ENTRY_HEIGHT + ENTRY_SPACING;
        let num_rows = self.results.entries.len();

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
                    .results
                    .entries
                    .first()
                    .map(|v| v.score.score)
                    .unwrap_or(1.0);
                //let mut selected_rect = None;

                let mut hit_boxes = Vec::new();
                for i in first_item..last_item {
                    let entry = &self.results.entries[i];
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
                        0.4 + (entry.score.score.max(0.001) / top_score.max(0.001)) * 0.6;
                    if self.results.query.is_empty() {
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

                    hit_boxes.push((panel_rect, entry.id.clone()));
                    used_rect = used_rect.union(rect);
                }

                ui.input(|input| {
                    if let Some(pos) = input.pointer.hover_pos() {
                        //if self.mouse_lock_from.elapsed() > Duration::from_millis(300) {
                            for (rect, id) in &hit_boxes {
                                let rect = rect.expand2(Vec2::new(0.0, ENTRY_SPACING / 2.0));
                                if rect.contains(pos) {
                                    let Some((_, _)) = self
                                        .results
                                        .entries
                                        .iter()
                                        .enumerate()
                                        .find(|(_, v)| &v.id == id)
                                    else {
                                        continue;
                                    };

                                    if input.pointer.is_moving() {
                                        events.push(ResultsEvent::Hovered(id.clone()));
                                    }

                                    if input.pointer.primary_down() {
                                        events.push(ResultsEvent::Pressed(id.clone()));
                                    }
                                }
                            }
                        //}
                    }
                });

                ui.allocate_rect(used_rect, Sense::click());
            });

        events
    }

    fn draw_entry(
        &self,
        ui: &mut Ui,
        mut rect: Rect,
        selected: f32,
        opacity: f32,
        entry: &SearchResultEntry,
    ) -> Rect {
        let bg_rect = rect.expand(3.0);
        let Some(app) = self.apps.applications.get(&entry.id) else {
            return bg_rect;
        };

        rect.max.x -= 4.0;
        rect.min.x += 10.0;
        rect = rect.shrink2(Vec2::new(1.0, 0.0));

        let image_width = rect.height();
        if let Some(icon) = self.app_icons.read_icon(&entry.id) {
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

            for (i, char) in app.name.chars().enumerate() {
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

            if let Some(comment) = app.comment.as_ref() {
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
}