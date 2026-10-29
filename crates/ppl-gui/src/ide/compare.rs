//! Side-by-side check of the emulator display against a screenshot read
//! from the calculator. Read-only: the user starts the program on the
//! calculator; the IDE never presses calculator keys.

use crate::theme::{ACCENT_AMBER, ACCENT_GREEN, ACCENT_RED, TEXT_MUTED};
use egui::{Color32, ColorImage, RichText, TextureHandle, TextureOptions};
use ppl_graphics::Grob;
use std::time::{SystemTime, UNIX_EPOCH};

const WIDTH: usize = 320;
const HEIGHT: usize = 240;

#[derive(Default)]
pub struct ScreenCompare {
    source: Option<ScreenIdentity>,
    captured_at: Option<u64>,
    device_captured_at: Option<u64>,
    emulator: Option<Grob>,
    device: Option<Grob>,
    error: Option<String>,
    highlight: bool,
    textures: Option<[TextureHandle; 2]>,
}

/// Identifies the actual selected emulator frame, independently of the editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenIdentity {
    pub firmware: bool,
    pub generation: u64,
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn capture_time(seconds: u64) -> String {
    let day = seconds % 86_400;
    format!("{:02}:{:02}:{:02} UTC", day / 3600, day / 60 % 60, day % 60)
}

/// Pixels that differ between the two screens, as an RGB565 comparison.
fn mismatches(a: &Grob, b: &Grob) -> Option<usize> {
    ppl_graphics::compare_rgb565(a, b).map(|d| d.mismatched)
}

fn image(grob: &Grob, other: Option<&Grob>, highlight: bool) -> ColorImage {
    let other = other.filter(|o| o.width() == grob.width() && o.height() == grob.height());
    let pixels = grob
        .pixels()
        .iter()
        .enumerate()
        .map(|(i, &p)| {
            let differs = highlight
                && other.is_some_and(|o| {
                    o.pixels()
                        .get(i)
                        .is_some_and(|&q| ppl_graphics::to_rgb565(q) != ppl_graphics::to_rgb565(p))
                });
            if differs {
                Color32::from_rgb(255, 0, 255)
            } else {
                let (r, g, b) = ppl_graphics::channels(p);
                Color32::from_rgb(r, g, b)
            }
        })
        .collect();
    ColorImage::new([grob.width(), grob.height()], pixels)
}

impl ScreenCompare {
    /// Starts a new pair of snapshots; the device image arrives on the worker.
    pub fn begin(&mut self, emulator: Grob, source: ScreenIdentity) {
        self.source = Some(source);
        self.captured_at = Some(now_seconds());
        self.device_captured_at = None;
        self.emulator = Some(emulator);
        self.device = None;
        self.error = None;
        self.textures = None;
        if self
            .emulator
            .as_ref()
            .is_some_and(|g| g.width() != WIDTH || g.height() != HEIGHT)
        {
            self.error = Some("The emulator screen must be 320×240 pixels.".into());
        }
    }

    pub fn set_device_png(&mut self, png: &[u8]) {
        let mut decoder = ppl_graphics::PrimeScreenshotDecoder::default();
        match decoder.decode(png, Some((WIDTH, HEIGHT))) {
            Ok((grob, _)) => {
                self.device = Some(grob);
                self.device_captured_at = Some(now_seconds());
            }
            Err(e) => {
                self.device_captured_at = None;
                self.device = None;
                self.error = Some(format!("Could not decode the calculator screen: {e}"));
            }
        }
        self.textures = None;
    }

    pub fn set_error(&mut self, message: String) {
        self.device = None;
        self.device_captured_at = None;
        self.textures = None;
        self.error = Some(message);
    }

    pub fn capture_unavailable(&mut self, message: String) {
        self.source = None;
        self.captured_at = None;
        self.emulator = None;
        self.set_error(message);
    }

    fn refresh_textures(&mut self, ctx: &egui::Context) {
        let (Some(emu), Some(dev)) = (&self.emulator, &self.device) else {
            return;
        };
        let options = TextureOptions::NEAREST;
        let emu_img = image(emu, Some(dev), self.highlight);
        let dev_img = image(dev, Some(emu), self.highlight);
        self.textures = Some([
            ctx.load_texture("compare_emulator", emu_img, options),
            ctx.load_texture("compare_device", dev_img, options),
        ]);
    }

    /// Observable evidence for the opt-in IDE recorder; no hardware access.
    pub fn snapshot_summary(&self) -> serde_json::Value {
        serde_json::json!({
            "engine": self.source.as_ref().map(|s| if s.firmware { "firmware" } else { "native" }),
            "generation": self.source.as_ref().map(|s| s.generation),
            "emulator_captured_at_unix_s": self.captured_at,
            "physical_captured_at_unix_s": self.device_captured_at,
            "emulator_dimensions": self.emulator.as_ref().map(|g| [g.width(), g.height()]),
            "physical_dimensions": self.device.as_ref().map(|g| [g.width(), g.height()]),
            "mismatched_pixels_rgb565": self.emulator.as_ref().zip(self.device.as_ref()).and_then(|(a,b)| mismatches(a,b)),
            "error": self.error,
        })
    }

    pub fn is_stale(&self, current: Option<&ScreenIdentity>) -> bool {
        self.source
            .as_ref()
            .is_some_and(|captured| current != Some(captured))
    }

    /// Persistent dock; returns true only when the user requests a fresh pair.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        open: &mut bool,
        waiting: bool,
        busy: bool,
        current: Option<&ScreenIdentity>,
    ) -> bool {
        if !*open {
            return false;
        }
        if self.textures.is_none() {
            self.refresh_textures(ctx);
        }
        let mut refresh = false;
        let mut highlight = self.highlight;
        let max_width = (ctx.available_rect().width() * 0.6).max(260.0);
        egui::SidePanel::right("screen_comparison")
            .resizable(true)
            .default_width(680.0_f32.min(max_width))
            .width_range(260.0..=max_width)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Compare screens");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            *open = false;
                        }
                    });
                });
                refresh = ui.add_enabled(!busy, egui::Button::new("Refresh both captures"))
                    .on_hover_text("Read the selected emulator frame and the physical calculator screen. No programs or keys are sent.")
                    .clicked();
                if waiting {
                    ui.horizontal(|ui| { ui.spinner(); ui.label("Reading physical G1…"); });
                } else if busy {
                    ui.label("Calculator busy; refresh is available when the operation finishes.");
                }
                if let Some(e) = &self.error {
                    ui.colored_label(ACCENT_RED, e.as_str());
                }
                if self.is_stale(current) {
                    ui.colored_label(ACCENT_AMBER, "Stale: the selected engine or emulator frame has changed. Refresh both captures.");
                }
                if let (Some(emu), Some(dev)) = (&self.emulator, &self.device) {
                    match mismatches(emu, dev) {
                        Some(0) => { ui.colored_label(ACCENT_GREEN, "✓ Captured screens match exactly"); }
                        Some(n) => {
                            let pct = n as f64 * 100.0 / (WIDTH * HEIGHT) as f64;
                            ui.colored_label(ACCENT_AMBER, format!("{n} / {} pixels differ ({pct:.1}%)", WIDTH * HEIGHT));
                        }
                        None => { ui.colored_label(ACCENT_AMBER, "The screens have different sizes"); }
                    }
                } else if !waiting {
                    ui.label("Capture required. Arrange matching display states, then refresh.");
                }
                ui.checkbox(&mut highlight, "Highlight differences in magenta");
                ui.separator();
                let width = ((ui.available_width() - ui.spacing().item_spacing.x) / 2.0).max(1.0);
                let size = egui::vec2(width, width * HEIGHT as f32 / WIDTH as f32);
                ui.columns(2, |columns| {
                    let engine = match &self.source {
                        Some(s) if s.firmware => "G1 firmware engine",
                        Some(_) => "Native PPL engine",
                        None => "Emulator",
                    };
                    columns[0].label(engine);
                    columns[1].label("Physical G1 · USB screenshot");
                    for (index, time) in [self.captured_at, self.device_captured_at].into_iter().enumerate() {
                        columns[index].small(time.map(capture_time).unwrap_or_else(|| "Not captured".into()));
                        if let Some(textures) = &self.textures {
                            columns[index].image((textures[index].id(), size));
                        } else {
                            columns[index].allocate_space(size);
                        }
                    }
                });
                if let Some(source) = &self.source {
                    ui.small(format!("Emulator frame generation {}", source.generation));
                }
                ui.label(RichText::new(
                    "320×240 snapshots · exact RGB565 comparison, no crop or alignment shift. Captures are read sequentially, not simultaneously. The G1 may change after capture; refresh after each interaction."
                ).size(11.0).color(TEXT_MUTED));
            });
        if highlight != self.highlight {
            self.highlight = highlight;
            self.textures = None;
        }
        refresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(firmware: bool, generation: u64) -> ScreenIdentity {
        ScreenIdentity {
            firmware,
            generation,
        }
    }

    #[test]
    fn snapshot_staleness_tracks_engine_and_frame_generation() {
        let mut compare = ScreenCompare::default();
        assert!(!compare.is_stale(None));
        compare.begin(Grob::new(WIDTH, HEIGHT, 0), identity(false, 7));
        assert!(!compare.is_stale(Some(&identity(false, 7))));
        assert!(compare.is_stale(Some(&identity(false, 8))));
        assert!(compare.is_stale(Some(&identity(true, 7))));
        assert!(compare.is_stale(None));
    }

    #[test]
    fn closed_window_draws_nothing() {
        let mut compare = ScreenCompare::default();
        let mut open = false;
        assert!(!compare.show(&egui::Context::default(), &mut open, false, false, None));
        assert!(!open);
    }

    #[test]
    fn failed_recapture_cannot_leave_a_previous_matching_pair() {
        let mut compare = ScreenCompare::default();
        compare.begin(Grob::new(WIDTH, HEIGHT, 0), identity(false, 1));
        compare.set_device_png(&png(WIDTH as u32, HEIGHT as u32));
        assert!(compare.device_captured_at.is_some());
        compare.capture_unavailable("No selected engine frame".into());
        assert!(compare.device.is_none() && compare.emulator.is_none());
        assert!(compare.source.is_none() && compare.textures.is_none());
        assert!(compare.device_captured_at.is_none() && compare.captured_at.is_none());
    }

    #[test]
    fn bad_png_is_reported_not_panicked() {
        let mut compare = ScreenCompare::default();
        compare.begin(Grob::new(WIDTH, HEIGHT, 0), identity(false, 0));
        compare.set_device_png(b"not a png");
        assert!(compare.error.is_some());
    }

    #[test]
    fn highlighting_uses_the_same_rgb565_comparison_as_the_count() {
        let a = Grob::new(1, 1, ppl_graphics::rgb(0, 0, 0));
        let b = Grob::new(1, 1, ppl_graphics::rgb(1, 0, 0));
        assert_eq!(mismatches(&a, &b), Some(0));
        assert_eq!(image(&a, Some(&b), true).pixels[0], Color32::BLACK);
        let changed = Grob::new(1, 1, ppl_graphics::WHITE);
        assert_eq!(mismatches(&a, &changed), Some(1));
        assert_eq!(
            image(&a, Some(&changed), true).pixels[0],
            Color32::from_rgb(255, 0, 255)
        );
    }

    #[test]
    fn identical_screens_have_no_mismatch() {
        let a = Grob::new(WIDTH, HEIGHT, 0);
        assert_eq!(mismatches(&a, &a.clone()), Some(0));
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, w, h);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&vec![0; (w * h * 3) as usize])
                .unwrap();
        }
        bytes
    }

    #[test]
    fn fresh_capture_discards_previous_images_and_errors() {
        let mut compare = ScreenCompare::default();
        compare.begin(Grob::new(WIDTH, HEIGHT, 0), identity(false, 0));
        compare.set_device_png(&png(WIDTH as u32, HEIGHT as u32));
        assert!(compare.device.is_some());
        compare.set_error("old failure".into());
        compare.begin(
            Grob::new(WIDTH, HEIGHT, ppl_graphics::WHITE),
            identity(false, 1),
        );
        assert!(compare.device.is_none() && compare.error.is_none() && compare.textures.is_none());
        assert_eq!(
            compare.emulator.as_ref().unwrap().pixels()[0],
            ppl_graphics::WHITE
        );
        compare.set_device_png(&png(WIDTH as u32, HEIGHT as u32));
        assert_eq!(
            mismatches(
                compare.emulator.as_ref().unwrap(),
                compare.device.as_ref().unwrap()
            ),
            Some(WIDTH * HEIGHT)
        );
    }

    #[test]
    fn incompatible_dimensions_are_rejected_without_flat_index_highlights() {
        let a = Grob::new(2, 1, 0);
        let b = Grob::new(1, 2, ppl_graphics::WHITE);
        assert_eq!(mismatches(&a, &b), None);
        assert!(
            image(&a, Some(&b), true)
                .pixels
                .iter()
                .all(|p| *p == Color32::BLACK)
        );
        let mut compare = ScreenCompare::default();
        compare.begin(a, identity(false, 0));
        assert!(compare.error.is_some());
        compare.begin(Grob::new(WIDTH, HEIGHT, 0), identity(false, 0));
        compare.set_device_png(&png(160, 120));
        assert!(compare.error.as_ref().unwrap().contains("dimensions"));
        assert!(compare.device.is_none());
    }

    #[test]
    fn rectangular_change_highlights_exactly_the_changed_region() {
        let a = Grob::new(WIDTH, HEIGHT, ppl_graphics::WHITE);
        let mut b = a.clone();
        b.fill_rect(200, 100, 239, 139, 0);
        assert_eq!(mismatches(&a, &b), Some(1600));
        let highlighted = image(&a, Some(&b), true);
        for (i, p) in highlighted.pixels.iter().enumerate() {
            let inside = (200..240).contains(&(i % WIDTH)) && (100..140).contains(&(i / WIDTH));
            assert_eq!(*p == Color32::from_rgb(255, 0, 255), inside);
        }
    }
}
