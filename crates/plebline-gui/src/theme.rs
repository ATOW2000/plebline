//! Window colours and fonts: the site's, Archivo for reading and IBM Plex
//! Mono for columns and labels (SIL OFL 1.1, licences in `fonts/`).
//!
//! PlebLine palette: dark blue backgrounds, a brighter blue for accents,
//! amber for attention. Red is not a brand colour; it marks mainnet and
//! errors.

use eframe::egui;
use eframe::egui::Color32;

/// Accent: titles, the main button, and the step line while the network is
/// unplugged.
pub const COLD: Color32 = Color32::from_rgb(0x4c, 0x8d, 0xff);
/// Warnings that are not errors, and the step line while the network is on.
pub const AMBER: Color32 = Color32::from_rgb(0xff, 0xb0, 0x00);
/// Steps not reached yet on the step line, network on.
pub const AMBER_DIM: Color32 = Color32::from_rgb(0x6b, 0x4f, 0x12);
/// Surfaces: panels, frames, fields.
pub const NIGHT: Color32 = Color32::from_rgb(0x03, 0x1a, 0x44);

/// Mainnet and failures. Nothing else is red.
pub const RED: Color32 = Color32::from_rgb(0xe0, 0x46, 0x4e);

/// Status light once a step is done. Green, since red/green needs no
/// explaining.
pub const GREEN: Color32 = Color32::from_rgb(0x3d, 0xd6, 0x6b);

/// Window background, a shade darker than `NIGHT`.
pub const BACKGROUND: Color32 = Color32::from_rgb(0x02, 0x0f, 0x2a);
/// Input fields and code.
pub const SUNKEN: Color32 = Color32::from_rgb(0x01, 0x0a, 0x1c);
/// Button fill.
const RAISED: Color32 = Color32::from_rgb(0x07, 0x2a, 0x52);
const RAISED_HOVER: Color32 = Color32::from_rgb(0x0c, 0x3d, 0x6d);
pub const BORDER: Color32 = Color32::from_rgb(0x0a, 0x33, 0x5c);
/// Body text, slightly blue.
pub const TEXT: Color32 = Color32::from_rgb(0xcf, 0xde, 0xee);
/// Notes and hints.
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x76, 0x93, 0xa8);
/// `strong()` text and pressed buttons.
pub const TEXT_STRONG: Color32 = Color32::from_rgb(0xea, 0xf4, 0xff);
/// Background of pressed or selected widgets, and the rule under titles.
pub const COLD_DIM: Color32 = Color32::from_rgb(0x2a, 0x4e, 0x8c);

/// Background of the warning panel.
pub const WARNING_BG: Color32 = Color32::from_rgb(0x33, 0x24, 0x06);

/// Call once at startup.
pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    // Always dark, so a light system theme cannot switch colours mid-ceremony.
    ctx.set_theme(egui::ThemePreference::Dark);
    // All styles, so a theme change cannot undo this.
    ctx.all_styles_mut(|style| {
        style.visuals = visuals();
        style.text_styles = type_scale();
        style.spacing.item_spacing = egui::vec2(8.0, 4.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
    });
}

/// Archivo's weight for running text; titles ask for more with
/// `RichText::variation`.
pub const WEIGHT_BODY: f32 = 400.0;
pub const WEIGHT_TITLE: f32 = 600.0;
pub const WEIGHT_BRAND: f32 = 700.0;

/// The site's fonts, compiled into the binary so nothing is read from the
/// system at run time. egui's own fonts remain as fallback for missing
/// glyphs.
fn fonts() -> egui::FontDefinitions {
    use egui::epaint::text::VariationCoords;
    use egui::{FontData, FontFamily, FontTweak};
    use std::sync::Arc;

    let mut fonts = egui::FontDefinitions::default();
    // Variable font: its default instance is 600, too heavy for reading.
    let archivo =
        FontData::from_static(include_bytes!("../fonts/Archivo-latin.ttf")).tweak(FontTweak {
            coords: VariationCoords::new([("wght", WEIGHT_BODY)]),
            ..FontTweak::default()
        });
    let plex = FontData::from_static(include_bytes!("../fonts/IBMPlexMono-Regular.ttf"));
    fonts.font_data.insert("archivo".into(), Arc::new(archivo));
    fonts.font_data.insert("plex-mono".into(), Arc::new(plex));
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "archivo".into());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "plex-mono".into());
    fonts
}

fn type_scale() -> std::collections::BTreeMap<egui::TextStyle, egui::FontId> {
    use egui::FontFamily::{Monospace, Proportional};
    use egui::{FontId, TextStyle};

    // Running text in Archivo; columns from `plebline::out`, labels and the
    // console in Plex Mono, so they stay aligned.
    [
        (TextStyle::Small, FontId::new(12.0, Proportional)),
        (TextStyle::Body, FontId::new(15.0, Proportional)),
        (TextStyle::Button, FontId::new(15.0, Proportional)),
        (TextStyle::Heading, FontId::new(22.0, Proportional)),
        (TextStyle::Monospace, FontId::new(13.0, Monospace)),
    ]
    .into()
}

fn visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();

    v.panel_fill = BACKGROUND;
    v.window_fill = BACKGROUND;
    v.faint_bg_color = NIGHT;
    v.extreme_bg_color = SUNKEN;
    v.code_bg_color = NIGHT;
    v.text_edit_bg_color = Some(SUNKEN);

    v.override_text_color = Some(TEXT);
    v.weak_text_color = Some(TEXT_DIM);
    v.hyperlink_color = COLD;
    v.warn_fg_color = AMBER;
    v.error_fg_color = RED;

    v.window_stroke = egui::Stroke::new(1.0, BORDER);
    v.window_corner_radius = egui::CornerRadius::ZERO;
    v.menu_corner_radius = egui::CornerRadius::ZERO;
    v.window_shadow = egui::epaint::Shadow::NONE;
    v.popup_shadow = egui::epaint::Shadow::NONE;

    // Selection derived from `COLD`, so it follows palette changes.
    v.selection.bg_fill = Color32::from_rgba_unmultiplied(COLD.r(), COLD.g(), COLD.b(), 70);
    v.selection.stroke = egui::Stroke::new(1.0, TEXT_STRONG);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = NIGHT;
    w.noninteractive.weak_bg_fill = NIGHT;
    w.noninteractive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    w.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT);

    w.inactive.bg_fill = RAISED;
    w.inactive.weak_bg_fill = RAISED;
    w.inactive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    w.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT);

    // Hover shows mostly on the border.
    w.hovered.bg_fill = RAISED_HOVER;
    w.hovered.weak_bg_fill = RAISED_HOVER;
    w.hovered.bg_stroke = egui::Stroke::new(1.0, COLD);
    w.hovered.fg_stroke = egui::Stroke::new(1.5, COLD);

    // Pressed widgets get a `COLD_DIM` background. This text colour is also
    // egui's `strong_text_color`, so it must stay readable on dark.
    w.active.bg_fill = COLD_DIM;
    w.active.weak_bg_fill = COLD_DIM;
    w.active.bg_stroke = egui::Stroke::new(1.0, COLD);
    w.active.fg_stroke = egui::Stroke::new(1.5, TEXT_STRONG);

    w.open.bg_fill = RAISED;
    w.open.weak_bg_fill = RAISED;
    w.open.bg_stroke = egui::Stroke::new(1.0, BORDER);
    w.open.fg_stroke = egui::Stroke::new(1.0, TEXT);

    for s in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        s.corner_radius = egui::CornerRadius::ZERO;
        // Widgets do not grow on hover.
        s.expansion = 0.0;
    }

    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Italian text needs no fallback font, and Archivo covers every weight
    /// the code asks for.
    #[test]
    fn the_site_fonts_carry_the_text_and_the_weights() {
        let mut ours = fonts();
        // Keep only a fallback face with no letters (just the replacement
        // glyph), so a letter our fonts lack shows up as missing.
        for names in ours.families.values_mut() {
            names.retain(|name| {
                ["archivo", "plex-mono", "NotoEmoji-Regular"].contains(&name.as_str())
            });
            assert_eq!(names.len(), 2, "{names:?}");
        }

        let weight = ours.font_data["archivo"]
            .variation_axes()
            .into_iter()
            .find(|axis| axis.tag == "wght")
            .expect("Archivo è variabile nel peso");
        for asked in [WEIGHT_BODY, WEIGHT_TITLE, WEIGHT_BRAND] {
            assert!(
                weight.range.contains(asked),
                "peso {asked} fuori da {:?}",
                weight.range
            );
        }

        let ctx = egui::Context::default();
        ctx.set_fonts(ours);
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        let text = "Più già perché «sì» — àèéìòù È 1.234,56 sat · 50%";
        ctx.fonts_mut(|fonts| {
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                let font = egui::FontId::new(15.0, family);
                let missing: String = text
                    .chars()
                    .filter(|c| !fonts.has_glyph(&font, *c))
                    .collect();
                assert!(missing.is_empty(), "{font:?} non ha «{missing}»");
            }
        });
    }
}
