use eframe::egui::{Align2, Color32, FontFamily, FontId, Painter, Pos2, Rect};
use eframe::egui::text::LayoutJob;

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