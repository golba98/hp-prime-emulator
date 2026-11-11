//! Capture the actual composited Native-tools LCD from an asynchronous viewport
//! screenshot. A GROB underneath an egui overlay is not a screenshot of that UI.
//!
//! Integration order: render tools, construct the current stamp, then `poll`.
//! Save/Compare requests must retain the calculator viewport until completion.
//! Automation window captures must ignore `is_lcd_capture_event` replies.

use egui::{ColorImage, Context, Event, Rect, UserData, ViewportId};
use ppl_graphics::Grob;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

pub const CAPTURE_VERSION: u32 = 1;
static NEXT_CAPTURE_OWNER: AtomicU64 = AtomicU64::new(1);
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(5);
/// Distinguishes captured overlays from the underlying Native framebuffer.
const CAPTURE_GENERATION_NAMESPACE: u64 = 1 << 63;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureBackend {
    Native,
    Firmware,
}

/// Identity of the UI being captured, in the screenshot viewport's logical
/// coordinates. `frame_generation` is the source/model revision, not the egui
/// frame counter: unchanged UI must retain its identity while awaiting a reply.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptureStamp {
    pub backend: CaptureBackend,
    pub tool_fingerprint: u64,
    pub frame_generation: u64,
    pub lcd_rect: Rect,
    pub viewport_rect: Rect,
    pub pixels_per_point: f32,
    pub viewport_id: ViewportId,
}

impl CaptureStamp {
    fn validate(&self) -> Result<(), CaptureError> {
        if self.backend != CaptureBackend::Native {
            return Err(CaptureError::WrongBackend);
        }
        if !self.pixels_per_point.is_finite()
            || self.pixels_per_point <= 0.0
            || !finite_rect(self.lcd_rect)
            || !finite_rect(self.viewport_rect)
            || !self.viewport_rect.contains_rect(self.lcd_rect)
        {
            return Err(CaptureError::InvalidGeometry);
        }
        let sx = self.lcd_rect.width() / 320.0;
        let sy = self.lcd_rect.height() / 240.0;
        if (sx - sy).abs() > sx.max(sy) * 0.005 {
            return Err(CaptureError::InvalidAspect);
        }
        Ok(())
    }
}

fn finite_rect(rect: Rect) -> bool {
    [rect.min.x, rect.min.y, rect.max.x, rect.max.y]
        .iter()
        .all(|n| n.is_finite())
        && rect.width() > 0.0
        && rect.height() > 0.0
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureAction {
    Save(PathBuf),
    Compare,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureError {
    WrongBackend,
    InvalidGeometry,
    InvalidAspect,
    InvalidImage,
    ViewportSizeChanged,
    StaleFrame,
    InputChanged,
    Busy,
    Timeout,
    RequestCounterExhausted,
}
impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::WrongBackend=>"The Native LCD capture belongs to a different execution backend",
            Self::InvalidGeometry=>"The calculator LCD is not fully visible in a valid viewport",
            Self::InvalidAspect=>"The calculator LCD does not have the required 320:240 aspect ratio",
            Self::InvalidImage=>"The viewport screenshot has invalid pixel data",
            Self::ViewportSizeChanged=>"The viewport pixel size changed before the LCD capture completed",
            Self::StaleFrame=>"The calculator screen changed before the LCD capture completed; capture it again",
            Self::InputChanged=>"Calculator input changed the visible screen while capture was pending; capture it again",
            Self::Busy=>"A capture of a different calculator screen is already pending",
            Self::Timeout=>"The viewport did not return the calculator LCD screenshot within five seconds",
            Self::RequestCounterExhausted=>"The LCD capture request counter is exhausted",
        })
    }
}
impl std::error::Error for CaptureError {}

#[derive(Clone, Debug)]
struct CaptureToken {
    owner: u64,
    version: u32,
    request: u64,
}

/// Only our typed requests match. Default-user-data automation screenshots and
/// other viewport screenshot consumers remain independent.
pub fn is_lcd_capture_event(event: &Event) -> bool {
    matches!(event,Event::Screenshot {user_data,..}
        if user_data.data.as_ref().is_some_and(|data|data.is::<CaptureToken>()))
}

#[derive(Clone, Debug)]
pub struct CapturedLcd {
    pub grob: Arc<Grob>,
    pub generation: u64,
    pub stamp: CaptureStamp,
}
#[derive(Clone, Debug)]
pub struct CaptureCompletion {
    pub action: CaptureAction,
    pub result: Result<CapturedLcd, CaptureError>,
}
struct Pending {
    id: u64,
    stamp: CaptureStamp,
    started: Instant,
    requested_frame: u64,
    actions: Vec<CaptureAction>,
}
pub struct NativeLcdCapture {
    owner: u64,
    pending: Option<Pending>,
    cached: Option<CapturedLcd>,
    next_request: u64,
    next_generation: u64,
}
impl Default for NativeLcdCapture {
    fn default() -> Self {
        Self {
            owner: NEXT_CAPTURE_OWNER.fetch_add(1, Ordering::Relaxed),
            pending: None,
            cached: None,
            next_request: 0,
            next_generation: 0,
        }
    }
}
impl NativeLcdCapture {
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }
    pub fn pending_actions(&self) -> impl Iterator<Item = &CaptureAction> {
        self.pending
            .iter()
            .flat_map(|pending| pending.actions.iter())
    }
    /// Returns a cache only for the exact current screen. Call `poll` every
    /// frame so transient hover, scrolling, caret and pointer edits invalidate it.
    pub fn cached(&self, current: &CaptureStamp) -> Option<&CapturedLcd> {
        self.cached
            .as_ref()
            .filter(|cached| cached.stamp == *current)
    }
    /// Bind an action to the rendered LCD. Even a matching old cache is refreshed:
    /// Save and Compare must use what is actually visible at the user's request.
    pub fn request(
        &mut self,
        ctx: &Context,
        stamp: CaptureStamp,
        action: CaptureAction,
    ) -> Result<u64, CaptureError> {
        stamp.validate()?;
        if let Some(pending) = &mut self.pending {
            if pending.stamp != stamp {
                return Err(CaptureError::Busy);
            }
            pending.actions.push(action);
            return Ok(pending.id);
        }
        let id = self
            .next_request
            .checked_add(1)
            .ok_or(CaptureError::RequestCounterExhausted)?;
        self.next_request = id;
        self.pending = Some(Pending {
            id,
            stamp: stamp.clone(),
            started: Instant::now(),
            requested_frame: ctx.cumulative_frame_nr(),
            actions: vec![action],
        });
        ctx.send_viewport_cmd_to(
            stamp.viewport_id,
            egui::ViewportCommand::Screenshot(UserData::new(CaptureToken {
                owner: self.owner,
                version: CAPTURE_VERSION,
                request: id,
            })),
        );
        ctx.request_repaint_after(CAPTURE_TIMEOUT);
        Ok(id)
    }
    /// Process only this module's typed screenshot replies after current tool
    /// state and LCD geometry have been rendered. Closing/hiding the LCD, backend
    /// switching, viewport resize, model edits and input all invalidate requests.
    pub fn poll(
        &mut self,
        ctx: &Context,
        current: Option<&CaptureStamp>,
    ) -> Vec<CaptureCompletion> {
        let events = ctx.input(|input| input.events.clone());
        self.process(&events, current, ctx.cumulative_frame_nr(), Instant::now())
    }
    fn process(
        &mut self,
        events: &[Event],
        current: Option<&CaptureStamp>,
        frame: u64,
        now: Instant,
    ) -> Vec<CaptureCompletion> {
        let input_changed = events.iter().any(changes_visible_input);
        if input_changed
            || current.is_none()
            || self
                .cached
                .as_ref()
                .is_some_and(|cached| Some(&cached.stamp) != current)
        {
            self.cached = None;
        }
        let Some(pending) = &self.pending else {
            return Vec::new();
        };
        if Some(&pending.stamp) != current {
            return self.finish(Err(CaptureError::StaleFrame));
        }
        if frame > pending.requested_frame && input_changed {
            return self.finish(Err(CaptureError::InputChanged));
        }
        if now.saturating_duration_since(pending.started) >= CAPTURE_TIMEOUT {
            return self.finish(Err(CaptureError::Timeout));
        }
        for event in events {
            let Event::Screenshot {
                viewport_id,
                user_data,
                image,
            } = event
            else {
                continue;
            };
            let Some(token) = user_data
                .data
                .as_ref()
                .and_then(|data| data.downcast_ref::<CaptureToken>())
            else {
                continue;
            };
            if token.owner != self.owner
                || token.version != CAPTURE_VERSION
                || token.request != pending.id
            {
                continue;
            }
            if *viewport_id != pending.stamp.viewport_id {
                return self.finish(Err(CaptureError::StaleFrame));
            }
            let result = crop_lcd(image, &pending.stamp);
            return match result {
                Ok(grob) => {
                    let Some(generation) = self
                        .next_generation
                        .checked_add(1)
                        .filter(|n| *n < CAPTURE_GENERATION_NAMESPACE)
                    else {
                        return self.finish(Err(CaptureError::RequestCounterExhausted));
                    };
                    self.next_generation = generation;
                    let captured = CapturedLcd {
                        grob: Arc::new(grob),
                        generation: CAPTURE_GENERATION_NAMESPACE | generation,
                        stamp: pending.stamp.clone(),
                    };
                    self.cached = Some(captured.clone());
                    self.finish(Ok(captured))
                }
                Err(error) => self.finish(Err(error)),
            };
        }
        Vec::new()
    }
    fn finish(&mut self, result: Result<CapturedLcd, CaptureError>) -> Vec<CaptureCompletion> {
        if result.is_err() {
            self.cached = None;
        }
        self.pending.take().map_or_else(Vec::new, |pending| {
            pending
                .actions
                .into_iter()
                .map(|action| CaptureCompletion {
                    action,
                    result: result.clone(),
                })
                .collect()
        })
    }
}

fn changes_visible_input(event: &Event) -> bool {
    matches!(
        event,
        Event::Key { .. }
            | Event::Text(_)
            | Event::Paste(_)
            | Event::Copy
            | Event::Cut
            | Event::PointerMoved(_)
            | Event::MouseMoved(_)
            | Event::PointerButton { .. }
            | Event::PointerGone
            | Event::MouseWheel { .. }
            | Event::Touch { .. }
            | Event::Zoom(_)
            | Event::Ime(_)
            | Event::WindowFocused(_)
    )
}

/// Project destination pixel centres through the exact logical LCD rectangle.
/// No integer-rounded crop is stretched: this handles fractional origins, DPI
/// and calculator scaling while excluding the surrounding calculator housing.
pub fn crop_lcd(image: &ColorImage, stamp: &CaptureStamp) -> Result<Grob, CaptureError> {
    stamp.validate()?;
    let [width, height] = image.size;
    if width == 0 || height == 0 || width.checked_mul(height) != Some(image.pixels.len()) {
        return Err(CaptureError::InvalidImage);
    }
    let ppp = f64::from(stamp.pixels_per_point);
    let expected = [
        f64::from(stamp.viewport_rect.width()) * ppp,
        f64::from(stamp.viewport_rect.height()) * ppp,
    ];
    // Window-system drawable extents may round a fractional point by one pixel.
    if !expected.iter().all(|v| v.is_finite())
        || (expected[0] - width as f64).abs() > 1.0
        || (expected[1] - height as f64).abs() > 1.0
    {
        return Err(CaptureError::ViewportSizeChanged);
    }
    let left = f64::from(stamp.lcd_rect.left() - stamp.viewport_rect.left()) * ppp;
    let top = f64::from(stamp.lcd_rect.top() - stamp.viewport_rect.top()) * ppp;
    let lcd_width = f64::from(stamp.lcd_rect.width()) * ppp;
    let lcd_height = f64::from(stamp.lcd_rect.height()) * ppp;
    let mut grob = Grob::screen();
    for y in 0..240 {
        let source_y = (top + (y as f64 + 0.5) / 240.0 * lcd_height).floor();
        if source_y < 0.0 || source_y >= height as f64 {
            return Err(CaptureError::InvalidGeometry);
        }
        for x in 0..320 {
            let source_x = (left + (x as f64 + 0.5) / 320.0 * lcd_width).floor();
            if source_x < 0.0 || source_x >= width as f64 {
                return Err(CaptureError::InvalidGeometry);
            }
            let pixel = image.pixels[source_y as usize * width + source_x as usize];
            grob.pixels_mut()[y * 320 + x] =
                (u32::from(pixel.r()) << 16) | (u32::from(pixel.g()) << 8) | u32::from(pixel.b());
        }
    }
    Ok(grob)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Color32, Pos2, Vec2};
    fn stamp(ppp: f32) -> CaptureStamp {
        CaptureStamp {
            backend: CaptureBackend::Native,
            tool_fingerprint: 17,
            frame_generation: 42,
            lcd_rect: Rect::from_min_size(Pos2::new(20.0, 10.0), Vec2::new(320.0, 240.0)),
            viewport_rect: Rect::from_min_size(Pos2::ZERO, Vec2::new(640.0, 480.0)),
            pixels_per_point: ppp,
            viewport_id: ViewportId::ROOT,
        }
    }
    fn image(ppp: f32) -> ColorImage {
        let w = (640.0 * ppp).round() as usize;
        let h = (480.0 * ppp).round() as usize;
        ColorImage::new(
            [w, h],
            (0..w * h)
                .map(|index| {
                    Color32::from_rgb((index % w % 256) as u8, (index / w % 256) as u8, 53)
                })
                .collect(),
        )
    }
    fn reply(capture: &NativeLcdCapture, image: ColorImage) -> Event {
        Event::Screenshot {
            viewport_id: ViewportId::ROOT,
            user_data: UserData::new(CaptureToken {
                owner: capture.owner,
                version: CAPTURE_VERSION,
                request: capture.pending.as_ref().unwrap().id,
            }),
            image: Arc::new(image),
        }
    }
    #[test]
    fn exact_lcd_crop_excludes_the_surrounding_viewport() {
        let result = crop_lcd(&image(1.0), &stamp(1.0)).unwrap();
        assert_eq!((result.width(), result.height()), (320, 240));
        assert_eq!(result.get(0, 0), Some(0x140a35));
        assert_eq!(result.get(319, 239), Some(0x53f935));
    }
    #[test]
    fn hidpi_samples_pixel_centres_without_stretching_a_rounded_crop() {
        let result = crop_lcd(&image(2.0), &stamp(2.0)).unwrap();
        assert_eq!(result.get(0, 0), Some(0x291535));
        assert_eq!(result.get(319, 239), Some(0xa7f335));
        let mut s = stamp(1.5);
        s.lcd_rect = Rect::from_min_size(Pos2::new(20.25, 10.5), Vec2::new(240.0, 180.0));
        let result = crop_lcd(&image(1.5), &s).unwrap();
        assert_eq!(result.get(0, 0), Some(0x1e1035));
        assert_eq!(result.get(319, 239), Some(0x851d35));
    }
    #[test]
    fn viewport_origin_is_subtracted_before_mapping_to_image_pixels() {
        let mut s = stamp(1.0);
        s.viewport_rect = s.viewport_rect.translate(Vec2::new(50.0, 100.0));
        s.lcd_rect = s.lcd_rect.translate(Vec2::new(50.0, 100.0));
        assert_eq!(
            crop_lcd(&image(1.0), &s).unwrap().pixels(),
            crop_lcd(&image(1.0), &stamp(1.0)).unwrap().pixels()
        );
    }
    #[test]
    fn invalid_geometry_aspect_and_malformed_images_return_errors() {
        let mut s = stamp(1.0);
        s.pixels_per_point = f32::NAN;
        assert_eq!(
            crop_lcd(&image(1.0), &s),
            Err(CaptureError::InvalidGeometry)
        );
        let mut s = stamp(1.0);
        s.lcd_rect.max.y -= 30.0;
        assert_eq!(crop_lcd(&image(1.0), &s), Err(CaptureError::InvalidAspect));
        let mut s = stamp(1.0);
        s.lcd_rect = s.lcd_rect.translate(Vec2::new(-100.0, 0.0));
        assert_eq!(
            crop_lcd(&image(1.0), &s),
            Err(CaptureError::InvalidGeometry)
        );
        let mut bad = image(1.0);
        bad.pixels.pop();
        assert_eq!(crop_lcd(&bad, &stamp(1.0)), Err(CaptureError::InvalidImage));
        assert_eq!(
            crop_lcd(&image(2.0), &stamp(1.0)),
            Err(CaptureError::ViewportSizeChanged)
        );
    }
    #[test]
    fn automation_and_other_capture_owners_cannot_complete_lcd_requests() {
        let ctx = Context::default();
        let mut capture = NativeLcdCapture::default();
        let s = stamp(1.0);
        capture
            .request(&ctx, s.clone(), CaptureAction::Compare)
            .unwrap();
        let automatic = Event::Screenshot {
            viewport_id: ViewportId::ROOT,
            user_data: UserData::default(),
            image: Arc::new(image(1.0)),
        };
        assert!(!is_lcd_capture_event(&automatic));
        assert!(
            capture
                .process(&[automatic], Some(&s), 1, Instant::now())
                .is_empty()
        );
        let mut other = NativeLcdCapture::default();
        other
            .request(&ctx, s.clone(), CaptureAction::Compare)
            .unwrap();
        let other_event = reply(&other, image(1.0));
        assert!(is_lcd_capture_event(&other_event));
        assert!(
            capture
                .process(&[other_event], Some(&s), 1, Instant::now())
                .is_empty()
        );
        assert!(capture.is_pending());
    }
    #[test]
    fn one_bound_capture_completes_save_and_compare_with_identical_pixels() {
        let ctx = Context::default();
        let mut capture = NativeLcdCapture::default();
        let s = stamp(1.0);
        let id = capture
            .request(&ctx, s.clone(), CaptureAction::Save("lcd.png".into()))
            .unwrap();
        assert_eq!(
            capture
                .request(&ctx, s.clone(), CaptureAction::Compare)
                .unwrap(),
            id
        );
        assert_eq!(capture.pending_actions().count(), 2);
        let event = reply(&capture, image(1.0));
        let done = capture.process(&[event], Some(&s), 1, Instant::now());
        assert_eq!(done.len(), 2);
        assert!(!capture.is_pending());
        let first = done[0].result.as_ref().unwrap();
        let second = done[1].result.as_ref().unwrap();
        assert!(Arc::ptr_eq(&first.grob, &second.grob));
        assert_eq!(first.generation, second.generation);
        assert_eq!(first.generation, CAPTURE_GENERATION_NAMESPACE | 1);
        assert!(capture.cached(&s).is_some());
        let mut different = s.clone();
        different.tool_fingerprint += 1;
        assert!(capture.cached(&different).is_none());
    }
    #[test]
    fn changes_of_tool_backend_viewport_or_source_generation_reject_pending_capture() {
        let ctx = Context::default();
        for change in 0..5 {
            let mut capture = NativeLcdCapture::default();
            let s = stamp(1.0);
            capture
                .request(&ctx, s.clone(), CaptureAction::Compare)
                .unwrap();
            let event = reply(&capture, image(1.0));
            let mut changed = s.clone();
            match change {
                0 => changed.tool_fingerprint += 1,
                1 => changed.frame_generation += 1,
                2 => changed.backend = CaptureBackend::Firmware,
                3 => changed.pixels_per_point = 2.0,
                _ => changed.lcd_rect = changed.lcd_rect.translate(Vec2::new(1.0, 0.0)),
            }
            let done = capture.process(&[event], Some(&changed), 1, Instant::now());
            assert!(matches!(done[0].result, Err(CaptureError::StaleFrame)));
            assert!(capture.cached(&s).is_none());
        }
    }
    #[test]
    fn hiding_lcd_input_and_timeout_report_failure_without_using_underlying_grob() {
        let ctx = Context::default();
        let s = stamp(1.0);
        let mut capture = NativeLcdCapture::default();
        capture
            .request(&ctx, s.clone(), CaptureAction::Compare)
            .unwrap();
        let done = capture.process(&[], None, 1, Instant::now());
        assert!(matches!(done[0].result, Err(CaptureError::StaleFrame)));
        capture
            .request(&ctx, s.clone(), CaptureAction::Compare)
            .unwrap();
        let done = capture.process(
            &[Event::PointerMoved(Pos2::new(40.0, 40.0))],
            Some(&s),
            1,
            Instant::now(),
        );
        assert!(matches!(done[0].result, Err(CaptureError::InputChanged)));
        capture
            .request(&ctx, s.clone(), CaptureAction::Compare)
            .unwrap();
        let deadline = capture.pending.as_ref().unwrap().started + CAPTURE_TIMEOUT;
        let done = capture.process(&[], Some(&s), 1, deadline);
        assert!(matches!(done[0].result, Err(CaptureError::Timeout)));
    }
    #[test]
    fn late_reply_cannot_complete_a_new_request() {
        let ctx = Context::default();
        let s = stamp(1.0);
        let mut capture = NativeLcdCapture::default();
        capture
            .request(&ctx, s.clone(), CaptureAction::Compare)
            .unwrap();
        let old = reply(&capture, image(1.0));
        capture.process(&[], None, 1, Instant::now());
        capture
            .request(&ctx, s.clone(), CaptureAction::Compare)
            .unwrap();
        assert!(
            capture
                .process(&[old], Some(&s), 1, Instant::now())
                .is_empty()
        );
        assert!(capture.is_pending());
        let done = capture.process(&[reply(&capture, image(1.0))], Some(&s), 1, Instant::now());
        assert!(done[0].result.is_ok());
    }
    #[test]
    fn input_in_the_request_frame_is_part_of_the_image_being_requested() {
        let ctx = Context::default();
        let s = stamp(1.0);
        let mut capture = NativeLcdCapture::default();
        capture
            .request(&ctx, s.clone(), CaptureAction::Compare)
            .unwrap();
        let event = reply(&capture, image(1.0));
        let done = capture.process(
            &[Event::PointerMoved(Pos2::new(40.0, 40.0)), event],
            Some(&s),
            ctx.cumulative_frame_nr(),
            Instant::now(),
        );
        assert!(done[0].result.is_ok());
        assert!(
            capture
                .process(
                    &[Event::PointerMoved(Pos2::new(41.0, 40.0))],
                    Some(&s),
                    1,
                    Instant::now()
                )
                .is_empty()
        );
        assert!(capture.cached(&s).is_none());
    }
}
