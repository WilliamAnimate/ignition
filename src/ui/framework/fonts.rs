use std::fs::read;
use std::path::Path;
use std::sync::Arc;
use eframe::egui::{FontData, FontDefinitions, FontFamily, FontTweak};

macro_rules! load_font {
    ($PATH:literal) => {
        #[allow(clippy::diverging_sub_expression)]
        'load: {
            #[cfg(debug_assertions)]
            break 'load load_font($PATH.replace("../", ""));
            #[cfg(not(debug_assertions))]
            break 'load load_font_static(include_bytes!($PATH));
        }
    };
}
pub fn load_fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::empty();

    add_font(
        &mut fonts,
        load_font!("../assets/Icons.ttf").tweak(FontTweak {
            scale: 1.0,
            y_offset_factor: 0.0,
            y_offset: 0.0,
            baseline_offset_factor: 0.0,
        }),
        "Icons",
    );
    add_font(
        &mut fonts,
        load_font!("../assets/Mukta-Regular.ttf").tweak(FontTweak {
            scale: 1.0,
            y_offset_factor: 0.0,
            y_offset: 0.0,
            baseline_offset_factor: 0.0,
        }),
        "Roboto-Regular",
    );

    add_font(
        &mut fonts,
        load_font!("../assets/Mukta-SemiBold.ttf").tweak(FontTweak {
            scale: 1.0,
            y_offset_factor: 0.0,
            y_offset: 0.0,
            baseline_offset_factor: -0.01,
        }),
        "Roboto-Bold",
    );

    fonts.families.insert(
        FontFamily::Proportional,
        vec!["Roboto-Regular".to_string(), "Icons".to_string()],
    );
    fonts
        .families
        .insert(FontFamily::Monospace, vec!["Roboto-Regular".to_string()]);

    fonts
}

#[allow(unused)]
fn load_font_static(font: &'static [u8]) -> FontData {
    FontData::from_static(font)
}
fn load_font<P: AsRef<Path>>(path: P) -> FontData {
    FontData::from_owned(read(path).unwrap())
}
fn add_font(fonts: &mut FontDefinitions, font: FontData, name: &str) {
    fonts.font_data.insert(name.to_owned(), font);
    fonts.families.insert(
        FontFamily::Name(Arc::from(name)),
        vec![name.to_string(), "Roboto-Regular".to_string()],
    );
}