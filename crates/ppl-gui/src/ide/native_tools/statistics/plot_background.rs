use crate::ide::{
    native_apps::PlotWindow,
    native_plot_controls::{X_PIXEL_SPAN, Y_PIXEL_SPAN},
};
use ppl_graphics::Grob;
use ppl_vm::{
    PlotBackgroundPicture as Picture, PlotBackgroundPlacement, PortableValue, SessionSnapshot,
    StatisticsPlotBackground,
};
use std::{borrow::Cow, cell::OnceCell, collections::BTreeMap};

const LCD_WIDTH: i64 = 320;
const PLOT_HEIGHT: i64 = 219;

pub(super) struct GalleryPicture {
    pub(super) key: Picture,
    name: Cow<'static, str>,
    pixels: Grob,
}

type AppFiles = BTreeMap<String, BTreeMap<String, PortableValue>>;

#[derive(Default)]
pub(super) struct Pictures {
    builtins: OnceCell<Result<Vec<GalleryPicture>, String>>,
    app_files: AppFiles,
    app_pictures: BTreeMap<String, Vec<GalleryPicture>>,
    generation: Option<u64>,
}

impl Pictures {
    pub(super) fn refresh(&mut self, snapshot: &SessionSnapshot) {
        if self.generation == Some(snapshot.generation) {
            return;
        }
        self.generation = Some(snapshot.generation);
        if self.app_files == snapshot.app_files {
            return;
        }
        self.app_files.clone_from(&snapshot.app_files);
        self.app_pictures = self
            .app_files
            .iter()
            .filter_map(|(app, files)| {
                let images: Vec<_> = files
                    .iter()
                    .filter_map(|(filename, value)| {
                        if filename.is_empty() || filename.len() > 255 {
                            return None;
                        }
                        let pixels = ppl_engine::plot_images::app_picture(value)?;
                        Some(GalleryPicture {
                            key: Picture::AppFile(filename.clone()),
                            name: Cow::Owned(filename.clone()),
                            pixels,
                        })
                    })
                    .collect();
                (!images.is_empty()).then(|| (app.clone(), images))
            })
            .collect();
    }

    fn builtins(&self) -> Result<&[GalleryPicture], &str> {
        self.builtins
            .get_or_init(|| {
                ppl_engine::plot_images::configured().map(|pictures| {
                    pictures
                        .into_iter()
                        .map(|picture| GalleryPicture {
                            key: Picture::BuiltIn(picture.slot),
                            name: Cow::Borrowed(picture.filename),
                            pixels: picture.pixels,
                        })
                        .collect()
                })
            })
            .as_deref()
            .map_err(String::as_str)
    }

    pub(super) fn list(&self, app: &str, include_builtins: bool) -> Vec<&GalleryPicture> {
        let attached = self.app_pictures.get(app).map_or(&[][..], Vec::as_slice);
        let builtins = if include_builtins {
            self.builtins().unwrap_or_default()
        } else {
            &[]
        };
        attached.iter().chain(builtins).collect()
    }

    pub(super) fn admission_error(&self) -> Option<&str> {
        self.builtins().err()
    }

    pub(super) fn paint(
        &self,
        output: &mut Grob,
        window: &PlotWindow,
        settings: &StatisticsPlotBackground,
        app: &str,
    ) {
        if settings.placement == PlotBackgroundPlacement::None || settings.opacity() == 0 {
            return;
        }
        let Some(key) = settings.picture.as_ref() else {
            return;
        };
        let images = match key {
            Picture::BuiltIn(_) => self.builtins().ok(),
            Picture::AppFile(_) => self.app_pictures.get(app).map(Vec::as_slice),
        };
        let Some(picture) =
            images.and_then(|images| images.iter().find(|picture| &picture.key == key))
        else {
            return;
        };
        let pixels = &picture.pixels;
        let Some(placement) = destination(window, settings, pixels) else {
            return;
        };
        composite(output, pixels, placement, settings.opacity());
    }
}

#[derive(Debug, PartialEq)]
struct PicturePlacement {
    bounds: [f64; 4],
    step: [f64; 2],
    source_offset: f64,
    centered: bool,
}

fn destination(
    window: &PlotWindow,
    settings: &StatisticsPlotBackground,
    pixels: &Grob,
) -> Option<PicturePlacement> {
    use PlotBackgroundPlacement as P;
    let [width, height] = [pixels.width() as i64, pixels.height() as i64];
    let center = |w: i64, h: i64| {
        let left = (LCD_WIDTH - w).div_euclid(2);
        let top = (PLOT_HEIGHT - h).div_euclid(2);
        [left, top, left + w, top + h].map(|side| side as f64)
    };
    let (bounds, source_offset) = match settings.placement {
        P::None => return None,
        P::Centered => (center(width, height), 0.),
        P::BestFit => {
            let (w, h) = if width * PLOT_HEIGHT >= height * LCD_WIDTH {
                (LCD_WIDTH, (height * LCD_WIDTH / width).max(1))
            } else {
                ((width * PLOT_HEIGHT / height).max(1), PLOT_HEIGHT)
            };
            (center(w, h), 0.)
        }
        P::Stretched => ([0., 0., LCD_WIDTH as f64, PLOT_HEIGHT as f64], 0.),
        P::Range => {
            window.validate().ok()?;
            let [xmin, xmax, ymin, ymax] = settings.bounds().map(ppl_math::Real::to_f64);
            if xmin >= xmax || ymin >= ymax {
                return None;
            }
            let x = |value: f64| {
                1. + ((value - window.xmin) / (window.xmax - window.xmin) * X_PIXEL_SPAN).round()
            };
            let y = |value: f64| {
                ((window.ymax - value) / (window.ymax - window.ymin) * Y_PIXEL_SPAN).round()
            };
            ([x(xmin), y(ymax), x(xmax) + 1., y(ymin)], 0.5)
        }
    };
    let [left, top, right, bottom] = bounds;
    if !bounds.into_iter().all(f64::is_finite) || right <= left || bottom <= top {
        return None;
    }
    let centered = settings.placement == P::Centered;
    let step = if centered {
        [1.; 2]
    } else {
        // G1 quantizes each axis step to seven fractional bits before sampling.
        let scaled =
            |length: i64, extent: f64| ((length - 1) as f64 * 128. / extent).floor() / 128.;
        [scaled(width, right - left), scaled(height, bottom - top)]
    };
    Some(PicturePlacement {
        bounds,
        step,
        source_offset,
        centered,
    })
}

fn composite(output: &mut Grob, source: &Grob, placement: PicturePlacement, opacity: u8) {
    let [left, top, right, bottom] = placement.bounds;
    let [step_x, step_y] = placement.step;
    let sx_max = (source.width() - 1) as f64;
    let sy_max = (source.height() - 1) as f64;
    let ink = (u32::from(opacity) * 256 + 50) / 100;
    for y in (top.max(0.) as i64)..(bottom.min(PLOT_HEIGHT as f64) as i64) {
        let sy = ((y as f64 - top) * step_y + placement.source_offset).min(sy_max);
        for x in (left.max(0.) as i64)..(right.min(LCD_WIDTH as f64) as i64) {
            let sx = ((x as f64 - left) * step_x + placement.source_offset).min(sx_max);
            let mut color = sample(source, sx, sy);
            // The G1's unscaled PNG path treats alpha zero as white.
            // Scaled sampling uses the stored RGB channels at every pixel.
            if placement.centered && color >> 24 == 255 {
                color = 0xffffff;
            }
            let background = output.get(x, y).unwrap_or(0xffffff);
            let mixed = [16, 8, 0].into_iter().fold(0, |pixel, shift| {
                let value = ((((color >> shift) & 255) * ink)
                    + (((background >> shift) & 255) * (256 - ink)))
                    >> 8;
                pixel | (value << shift)
            });
            output.set(x, y, mixed);
        }
    }
}

fn sample(image: &Grob, x: f64, y: f64) -> u32 {
    let [x0, y0] = [x.floor() as i64, y.floor() as i64];
    let [x1, y1] = [x.ceil() as i64, y.ceil() as i64];
    let a = image.get(x0, y0).unwrap();
    if x1 == x0 && y1 == y0 {
        return a;
    }
    let [b, c, d] = [
        image.get(x1, y0).unwrap(),
        image.get(x0, y1).unwrap(),
        image.get(x1, y1).unwrap(),
    ];
    let [fx, fy] = [x.fract(), y.fract()];
    [16, 8, 0].into_iter().fold(0, |pixel, shift| {
        let channel = |color: u32| f64::from((color >> shift) & 255_u32);
        let top = channel(a) + (channel(b) - channel(a)) * fx;
        let bottom = channel(c) + (channel(d) - channel(c)) * fx;
        pixel | (((top + (bottom - top) * fy).round() as u32) << shift)
    })
}

pub(super) const PLACEMENT_LABELS: [&str; 5] = [
    "No Background",
    "Centered",
    "Stretched",
    "Best Fit",
    "XY Range",
];
pub(super) const PLACEMENT_BOX: [i64; 4] = [4, 22, 163, 45];
pub(super) const PICTURE_BOX: [i64; 4] = [17, 49, 300, 149];
pub(super) const MODE: usize = 0;
pub(super) const OPACITY: usize = 1;
pub(super) const GALLERY: usize = 2;
pub(super) const FIRST_BOUND: usize = 3;

pub(super) struct NumericField {
    pub(super) field: ppl_vm::PlotBackgroundField,
    pub(super) bounds: [i64; 4],
    pub(super) caption: &'static str,
    pub(super) prompt: &'static str,
}

pub(super) const NUMERIC_FIELDS: [NumericField; 5] = [
    NumericField {
        field: ppl_vm::PlotBackgroundField::Opacity,
        bounds: [274, 23, 314, 44],
        caption: "Opacity:",
        prompt: "Enter opacity between 0 and 100",
    },
    NumericField {
        field: ppl_vm::PlotBackgroundField::XMin,
        bounds: [60, 153, 179, 174],
        caption: "X Rng:",
        prompt: "Enter minimum horizontal image value",
    },
    NumericField {
        field: ppl_vm::PlotBackgroundField::XMax,
        bounds: [182, 153, 314, 174],
        caption: "",
        prompt: "Enter maximum horizontal image value",
    },
    NumericField {
        field: ppl_vm::PlotBackgroundField::YMin,
        bounds: [60, 179, 179, 200],
        caption: "Y Rng:",
        prompt: "Enter minimum vertical image value",
    },
    NumericField {
        field: ppl_vm::PlotBackgroundField::YMax,
        bounds: [182, 179, 314, 200],
        caption: "",
        prompt: "Enter maximum vertical image value",
    },
];

pub(super) fn numeric_field(
    selection: usize,
    placement: PlotBackgroundPlacement,
) -> Option<&'static NumericField> {
    match (selection, placement) {
        (OPACITY, PlotBackgroundPlacement::None) => None,
        (OPACITY, _) => NUMERIC_FIELDS.first(),
        (FIRST_BOUND.., PlotBackgroundPlacement::Range) => {
            NUMERIC_FIELDS.get(selection - FIRST_BOUND + 1)
        }
        _ => None,
    }
}

pub(super) fn controls(placement: PlotBackgroundPlacement) -> &'static [usize] {
    match placement {
        PlotBackgroundPlacement::None => &[MODE, GALLERY],
        PlotBackgroundPlacement::Range => &[MODE, OPACITY, GALLERY, FIRST_BOUND, 4, 5, 6],
        _ => &[MODE, OPACITY, GALLERY],
    }
}

pub(super) fn picture_index(key: Option<&Picture>, pictures: &[&GalleryPicture]) -> usize {
    key.and_then(|key| pictures.iter().position(|picture| &picture.key == key))
        .map_or(0, |index| index + 1)
}

pub(super) fn move_picture(
    settings: &mut StatisticsPlotBackground,
    pictures: &[&GalleryPicture],
    delta: isize,
) {
    let index = picture_index(settings.picture.as_ref(), pictures)
        .saturating_add_signed(delta)
        .min(pictures.len());
    settings.picture = index
        .checked_sub(1)
        .map(|index| pictures[index].key.clone());
}

pub(super) const THUMBNAIL_SPACING: f32 = 145.;
const THUMBNAIL_HEIGHT: i64 = 93;

fn visible_pictures(scroll: f32, count: usize) -> std::ops::RangeInclusive<usize> {
    let first = (scroll.floor() as usize).saturating_sub(1);
    let last = (scroll.ceil() as usize + 1).min(count);
    first..=last
}

fn thumbnail_bounds(pictures: &[&GalleryPicture], position: usize, scroll: f32) -> [i64; 2] {
    let width = PICTURE_BOX[2] - PICTURE_BOX[0] - 1;
    let center = width / 2 + ((position as f32 - scroll) * THUMBNAIL_SPACING).round() as i64;
    if position == 0 {
        [center - 33, center + 32]
    } else {
        let pixels = &pictures[position - 1].pixels;
        let width = pixels.width() as i64 * THUMBNAIL_HEIGHT / pixels.height() as i64;
        let left = center - width / 2;
        [left, left + width]
    }
}

pub(super) fn picture_at(pictures: &[&GalleryPicture], scroll: f32, x: i64) -> Option<usize> {
    let x = x - PICTURE_BOX[0] - 1;
    visible_pictures(scroll, pictures.len()).find(|&position| {
        let [left, right] = thumbnail_bounds(pictures, position, scroll);
        (left..right).contains(&x)
    })
}

fn caption(output: &mut Grob, title: &str, x: i64, y: i64) {
    ppl_graphics::text::draw_text(output, title, x, y, 14., 0, Some(LCD_WIDTH - x), None);
}

pub(super) fn paint_setup(
    output: &mut Grob,
    background: &StatisticsPlotBackground,
    number_format: &ppl_vm::Settings,
    pictures: &[&GalleryPicture],
    selection: (usize, Option<usize>),
    gallery_scroll: Option<f32>,
    error: Option<&str>,
) {
    let (selected, picker) = selection;
    crate::ide::native::chrome::header(output, "Statistics 1Var Plot Setup");
    let accent = 0xc3dfff;
    let color = |focus| if focus { 0 } else { 0xe1e1e1 };
    let [x, y, right, bottom] = PLACEMENT_BOX;
    output.rect(x, y, right, bottom, color(selected == MODE), 0xffffff);
    output.fill_rect(right - 16, y + 2, right - 1, bottom - 2, accent);
    caption(
        output,
        PLACEMENT_LABELS[background.placement as usize],
        x + 3,
        y + 3,
    );
    caption(output, "▾", right - 12, y + 3);
    for &index in controls(background.placement) {
        let Some(field) = numeric_field(index, background.placement) else {
            continue;
        };
        let [x, y, right, bottom] = field.bounds;
        caption(
            output,
            field.caption,
            x - 3 - ppl_graphics::text::measure(field.caption, 14.),
            y + 2,
        );
        output.rect(x, y, right, bottom, color(index == selected), 0xffffff);
        let value = ppl_vm::Value::Real(background.value(field.field));
        caption(
            output,
            &value.display(number_format.home_format()).replace('-', "−"),
            x + 2,
            y + 2,
        );
    }
    let index = picture_index(background.picture.as_ref(), pictures);
    paint_gallery(
        output,
        pictures,
        index,
        gallery_scroll.unwrap_or(index as f32),
        selected == GALLERY,
        background.placement == PlotBackgroundPlacement::None,
    );
    let message = if let Some(error) = error {
        error.to_owned()
    } else if let Some(field) = numeric_field(selected, background.placement) {
        field.prompt.to_owned()
    } else if selected == GALLERY {
        picture_caption(pictures, index).unwrap_or_else(|| "Choose a Plot image".into())
    } else {
        "Select image background type".into()
    };
    caption(output, &message, 0, 203);
    if let Some(active) = picker {
        let [x, y, right, _] = PLACEMENT_BOX;
        let row = 21;
        output.rect(x, y, right, y + row * 5 + 1, 0, 0xffffff);
        for (index, title) in PLACEMENT_LABELS.iter().enumerate() {
            let top = y + row * index as i64 + 1;
            if index == active {
                output.fill_rect(x + 1, top, right - 1, top + row - 1, accent);
            }
            if index == background.placement as usize {
                caption(output, "✓", x + 3, top + 2);
            }
            caption(output, title, x + 15, top + 2);
        }
    }
}

fn paint_gallery(
    output: &mut Grob,
    pictures: &[&GalleryPicture],
    index: usize,
    scroll: f32,
    focused: bool,
    dimmed: bool,
) {
    let paper = 0xe8f1f6;
    let accent = 0xc3dfff;
    let [x, y, right, bottom] = PICTURE_BOX;
    let mut gallery = Grob::new((right - x - 1) as usize, (bottom - y - 1) as usize, paper);
    for position in visible_pictures(scroll, pictures.len()) {
        let [left, right] = thumbnail_bounds(pictures, position, scroll);
        if position == 0 {
            if position == index {
                gallery.fill_rect(left, 0, right, 97, accent);
            }
            caption(&mut gallery, "none", left + 18, 41);
        } else {
            let pixels = &pictures[position - 1].pixels;
            if position == index {
                gallery.fill_rect(left - 4, 0, right + 4, 97, accent);
            }
            let alpha = if dimmed { 140 } else { 255 };
            gallery.blit(
                (left, 3, right, 3 + THUMBNAIL_HEIGHT),
                pixels,
                (0, 0, pixels.width() as i64, pixels.height() as i64),
                None,
                alpha,
            );
        }
    }
    output.rect(
        x,
        y,
        right,
        bottom,
        if focused { 0 } else { 0xe1e1e1 },
        paper,
    );
    output.blit(
        (x + 1, y + 1, right, bottom),
        &gallery,
        (0, 0, right - x - 1, bottom - y - 1),
        None,
        255,
    );
}

fn picture_caption(pictures: &[&GalleryPicture], index: usize) -> Option<String> {
    let pic = pictures.get(index.checked_sub(1)?)?;
    Some(format!(
        "Picture: {} ({}×{})",
        pic.name,
        pic.pixels.width(),
        pic.pixels.height()
    ))
}

use super::super::gallery::APPS;
use crate::ide::native::{chrome, expression::EditCommand, keys::KeyAction};

const APP_BOX: [i64; 4] = [60, 22, 314, 45];
const APP_ROWS: usize = 7;

#[derive(Default)]
pub(super) struct ImportSelection {
    app: usize,
    picture: usize,
    gallery: bool,
    chooser: Option<usize>,
}

pub(super) enum ImportAction {
    Keep,
    Close,
    Copy(usize),
}

impl ImportSelection {
    pub(super) fn source(&self) -> &'static str {
        APPS[self.app]
    }

    fn first_app(&self) -> usize {
        self.chooser
            .unwrap_or(self.app)
            .saturating_sub(APP_ROWS - 1)
    }

    pub(super) fn action(&mut self, action: KeyAction, available: usize) -> ImportAction {
        use KeyAction as A;
        if let Some(choice) = &mut self.chooser {
            match action {
                A::Back | A::On => self.chooser = None,
                A::History(delta) => {
                    *choice = choice
                        .saturating_add_signed(delta as isize)
                        .min(APPS.len() - 1);
                }
                A::Evaluate => {
                    self.app = *choice;
                    self.picture = 0;
                    self.chooser = None;
                }
                _ => {}
            }
            return ImportAction::Keep;
        }
        match action {
            A::Back | A::On => ImportAction::Close,
            A::History(delta) => {
                self.gallery = delta > 0;
                ImportAction::Keep
            }
            A::Edit(EditCommand::Left(false)) if self.gallery => {
                self.picture = self.picture.min(available).saturating_sub(1);
                ImportAction::Keep
            }
            A::Edit(EditCommand::Right(false)) if self.gallery => {
                self.picture = (self.picture + 1).min(available);
                ImportAction::Keep
            }
            A::Evaluate if !self.gallery => {
                self.chooser = Some(self.app);
                ImportAction::Keep
            }
            A::Evaluate if (1..=available).contains(&self.picture) => {
                ImportAction::Copy(self.picture - 1)
            }
            _ => ImportAction::Keep,
        }
    }

    pub(super) fn click(&mut self, x: i64, y: i64) -> Option<KeyAction> {
        let contains = |[left, top, right, bottom]: [i64; 4]| {
            (left..=right).contains(&x) && (top..=bottom).contains(&y)
        };
        let first_app = self.first_app();
        if let Some(choice) = &mut self.chooser {
            let [left, top, right, _] = APP_BOX;
            if (left..=right).contains(&x) && y > top {
                let row = ((y - top - 1) / 21) as usize;
                if row < APP_ROWS && first_app + row < APPS.len() {
                    *choice = first_app + row;
                    return Some(KeyAction::Evaluate);
                }
            }
            self.chooser = None;
            return None;
        }
        if y >= 220 {
            return match chrome::softkey_index(x) {
                1 => {
                    self.gallery = false;
                    Some(KeyAction::Evaluate)
                }
                4 => Some(KeyAction::Back),
                5 => {
                    self.gallery = true;
                    Some(KeyAction::Evaluate)
                }
                _ => None,
            };
        }
        if contains(APP_BOX) {
            self.gallery = false;
            return Some(KeyAction::Evaluate);
        }
        if contains(PICTURE_BOX) {
            self.gallery = true;
            return match x {
                ..79 => Some(KeyAction::Edit(EditCommand::Left(false))),
                245.. => Some(KeyAction::Edit(EditCommand::Right(false))),
                _ => None,
            };
        }
        None
    }

    pub(super) fn paint(&self, output: &mut Grob, pictures: &[&GalleryPicture]) {
        chrome::header(output, "Import Image");
        let accent = 0xc3dfff;
        let [left, top, right, bottom] = APP_BOX;
        caption(output, "App:", 27, top + 3);
        output.rect(
            left,
            top,
            right,
            bottom,
            if self.gallery { 0xe1e1e1 } else { 0 },
            0xffffff,
        );
        output.fill_rect(right - 16, top + 2, right - 1, bottom - 2, accent);
        caption(output, self.source(), left + 3, top + 3);
        caption(output, "▾", right - 12, top + 3);
        let index = self.picture.min(pictures.len());
        paint_gallery(output, pictures, index, index as f32, self.gallery, false);
        if self.gallery {
            let text = picture_caption(pictures, index)
                .unwrap_or_else(|| "Choose an image to import".into());
            caption(output, &text, 0, 203);
        } else {
            caption(output, "Choose app to import image from", 0, 203);
        }
        let choose = if self.chooser.is_none() && !self.gallery {
            "Choose"
        } else {
            ""
        };
        let submit = if self.chooser.is_none() && index > 0 {
            "OK"
        } else {
            ""
        };
        chrome::softkeys(output, &["", choose, "", "", "Cancel", submit]);
        if let Some(choice) = self.chooser {
            let first = self.first_app();
            let rows = &APPS[first..(first + APP_ROWS).min(APPS.len())];
            let height = 21;
            output.rect(
                left,
                top,
                right,
                top + height * rows.len() as i64 + 1,
                0,
                0xffffff,
            );
            for (index, title) in rows.iter().enumerate() {
                let y = top + 1 + index as i64 * height;
                if first + index == choice {
                    output.fill_rect(left + 1, y, right - 1, y + height - 1, accent);
                }
                if first + index == self.app {
                    caption(output, "✓", left + 3, y + 2);
                }
                caption(output, title, left + 16, y + 2);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaled_picture_steps_follow_observed_portrait_and_landscape_placements() {
        use PlotBackgroundPlacement as Placement;
        let window = PlotWindow {
            xmin: -1.8,
            xmax: 30.,
            ymin: -1.8,
            ymax: 20.,
            ..PlotWindow::default()
        };
        let landscape = Grob::new(37, 23, 0);
        let portrait = Grob::new(21, 63, 0);
        let mut settings = StatisticsPlotBackground::default();
        settings.placement = Placement::Stretched;
        assert_eq!(
            destination(&window, &settings, &landscape).unwrap().step,
            [14. / 128., 12. / 128.]
        );
        assert_eq!(
            destination(&window, &settings, &portrait).unwrap().step,
            [8. / 128., 36. / 128.]
        );
        settings.placement = Placement::BestFit;
        assert_eq!(
            destination(&window, &settings, &landscape),
            Some(PicturePlacement {
                bounds: [0., 10., 320., 208.],
                step: [14. / 128.; 2],
                source_offset: 0.,
                centered: false,
            }),
        );
        assert_eq!(
            destination(&window, &settings, &portrait),
            Some(PicturePlacement {
                bounds: [123., 0., 196., 219.],
                step: [35. / 128., 36. / 128.],
                source_offset: 0.,
                centered: false,
            }),
        );
        settings.placement = Placement::Range;
        assert_eq!(
            destination(&window, &settings, &portrait),
            Some(PicturePlacement {
                bounds: [19., 0., 320., 200.],
                step: [8. / 128., 39. / 128.],
                source_offset: 0.5,
                centered: false,
            }),
        );
        let shifted = PlotWindow {
            xmin: -2.,
            xmax: 14.,
            ymin: -5.,
            ymax: 25.,
            ..window
        };
        assert_eq!(
            destination(&shifted, &settings, &portrait),
            Some(PicturePlacement {
                bounds: [41., 36., 638., 182.],
                step: [4. / 128., 54. / 128.],
                source_offset: 0.5,
                centered: false,
            }),
        );
    }
}
