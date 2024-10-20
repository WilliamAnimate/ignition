use eframe::egui::{Align, Align2, Color32, FontFamily, FontId, FontSelection, Margin, Response, Rounding, Stroke, TextBuffer, TextEdit, Ui, Vec2, Widget};
use eframe::egui::text_edit::TextEditOutput;
use splinter_icon::icon;
use crate::ui::framework::{draw_icon, Colors};

pub struct SearchBarMessage {
    pub text: String,
    pub color: Color32,
}

pub struct SearchBarWidget<'a> {
    pub query: &'a mut dyn TextBuffer,
    pub messages: Vec<SearchBarMessage>,
}

pub struct  SearchBarResponse {
    pub text_response: TextEditOutput,
    pub text: String,
}

impl SearchBarWidget<'_> {
    pub fn ui(self, ui: &mut Ui) -> TextEditOutput {
        let mut rect = ui.clip_rect();
        rect.set_height(64.0);

        let p = ui.painter();

        for message in self.messages {
            let text_rect = p.text(
                rect.shrink2(Vec2::new(16.0 + 8.0, 0.0)).right_center(),
                Align2::RIGHT_CENTER,
                message.text,
                FontId::new(15.0, FontFamily::Proportional),
                message.color,
            );
            let text_rect = text_rect.expand2(Vec2::new(10.0, 2.0));
            p.rect_filled(
                text_rect,
                Rounding::same(text_rect.height() / 2.0),
                message.color.gamma_multiply(0.2),
            );
        }

        p.line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            Stroke::new(1.0, Colors::SUBTEXT0.gamma_multiply(0.3)),
        );

        let font = FontId::new(18.0, FontFamily::Proportional);
        let output = TextEdit::singleline(self.query)
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

        //if output.response.changed() {
        //    let query = &self.search_query.clone();
        //    if query.trim().is_empty() {
        //        self.selected = None;
        //    } else {
        //        self.selected = Some(0);
        //    }
        //    self.search(query);
        //}

        let p = ui.painter();

        draw_icon(
            p,
            icon!("search"),
            rect.left_center() + Vec2::new(18.0 + 12.0, -1.0),
            18.0,
            Colors::TEXT,
        );

        if self.query.as_str().is_empty() {
            p.text(
                output.text_clip_rect.left_center(),
                Align2::LEFT_CENTER,
                "Search for a program",
                font,
                Colors::SUBTEXT0,
            );
        }

        output
    }
}
