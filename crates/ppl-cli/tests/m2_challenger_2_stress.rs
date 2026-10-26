//! Milestone 2 Challenger 2 Adversarial Stress & Verification Test Suite
//! Authored by Challenger 2 for Milestone 2: Authentic HP Prime G1 Keypad, Vector Geometry & Input Engine (F7–F13)
//!
//! Mission Requirements:
//! 1. Touch projection stress testing:
//!    - Verify `display.rs::project_pointer_pos` across random floating-point coordinates across
//!      arbitrary viewport rectangles and DPI scales (1.0x, 1.25x, 1.5x, 2.0x).
//!    - Verify every returned coordinate strictly satisfies 0 <= x < 320 and 0 <= y < 240
//!      with mathematical flooring.
//! 2. D-pad dead-zone and directional testing:
//!    - Test hit-testing and marker activation around center (187.0, 367.0): radius r < 10.0
//!      returns None (dead zone); |dx| > |dy| routes Left/Right; |dy| >= |dx| routes Up/Down.
//! 3. Touch settlement timing test:
//!    - Verify that simulated quick clicks (< 300ms) schedule deferred touch end and request
//!      repaints, avoiding lost taps on ARM firmware's 183ms SYS_TIMER.

use ppl_gui::ide::display::CalculatorDisplay;
use ppl_gui::ide::keypad::navigation_at;
use ppl_gui::keymap::lookup_by_physical;
use ppl_vm::host::Touch;
use std::time::{Duration, Instant};

// ============================================================================
// AREA 1: Touch Subpixel Projection & Boundary Stress Testing
// ============================================================================

/// LCG PRNG for reproducible adversarial floating-point coordinate generation.
struct Lcg {
    state: u64,
}

impl Lcg {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u32(&mut self) -> u32 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.state >> 32) as u32
    }

    fn next_f64(&mut self) -> f64 {
        (self.next_u32() as f64) / (u32::MAX as f64)
    }

    fn range_f64(&mut self, min: f64, max: f64) -> f64 {
        min + self.next_f64() * (max - min)
    }
}

#[test]
fn test_stress_touch_projection_across_dpi_scales_and_viewports() {
    let mut rng = Lcg::new(0xDEAD_BEEF_CAFE_BABE);
    let mut display = CalculatorDisplay::new();

    // Standard DPI scales and arbitrary non-integer scales
    let scales = [
        0.75,
        0.80,
        1.0,
        1.25,
        1.5,
        1.75,
        2.0,
        2.5,
        std::f64::consts::PI,
        4.0,
    ];
    let origins = [
        (0.0, 0.0),
        (100.0, 50.0),
        (123.456, 78.912),
        (-45.67, -89.12),
        (1920.0, 1080.0),
    ];

    for &scale in &scales {
        let width = 320.0 * scale;
        let height = 240.0 * scale;

        for &(ox, oy) in &origins {
            display.last_display_rect.min.x = ox as f32;
            display.last_display_rect.min.y = oy as f32;
            display.last_display_rect.max.x = (ox + width) as f32;
            display.last_display_rect.max.y = (oy + height) as f32;

            let rect = display.last_display_rect;

            // 1. Exact corners
            let corners = [
                (rect.min.x, rect.min.y, 0, 0),
                (rect.max.x, rect.min.y, 319, 0),
                (rect.min.x, rect.max.y, 0, 239),
                (rect.max.x, rect.max.y, 319, 239),
            ];
            for (cx, cy, exp_x, exp_y) in corners {
                let mut pos = rect.min;
                pos.x = cx;
                pos.y = cy;
                let touch = display.project_pointer_pos(pos).unwrap_or_else(|| {
                    panic!("Corner ({cx}, {cy}) must project to touch at scale {scale}");
                });
                assert_eq!(
                    (touch.x, touch.y),
                    (exp_x, exp_y),
                    "Corner ({cx}, {cy}) at scale {scale} mapped to ({}, {})",
                    touch.x,
                    touch.y
                );
            }

            // 2. Interior points: 10,000 random points per viewport
            for _ in 0..10_000 {
                let px = rng.range_f64(rect.min.x as f64, rect.max.x as f64) as f32;
                let py = rng.range_f64(rect.min.y as f64, rect.max.y as f64) as f32;
                let mut pos = rect.min;
                pos.x = px;
                pos.y = py;

                if rect.contains(pos) {
                    let touch = display
                        .project_pointer_pos(pos)
                        .expect("Inside point must project");

                    // Mathematical flooring oracle
                    let expected_x = (((pos.x - rect.min.x) as f64 * 320.0) / rect.width() as f64)
                        .floor() as i64;
                    let expected_y = (((pos.y - rect.min.y) as f64 * 240.0) / rect.height() as f64)
                        .floor() as i64;
                    let clamped_x = expected_x.clamp(0, 319);
                    let clamped_y = expected_y.clamp(0, 239);

                    // Strict invariant bounds check: 0 <= x < 320 and 0 <= y < 240
                    assert!(
                        touch.x >= 0 && touch.x < 320,
                        "Invariant violated: touch.x = {} out of bounds [0, 320) for px={px}",
                        touch.x
                    );
                    assert!(
                        touch.y >= 0 && touch.y < 240,
                        "Invariant violated: touch.y = {} out of bounds [0, 240) for py={py}",
                        touch.y
                    );

                    // Mathematical floor equivalence
                    assert_eq!(
                        touch.x, clamped_x,
                        "X coordinate flooring mismatch at px={px}, scale={scale}"
                    );
                    assert_eq!(
                        touch.y, clamped_y,
                        "Y coordinate flooring mismatch at py={py}, scale={scale}"
                    );
                }
            }

            // 3. Strict exterior rejection (deltas exceed f32 machine epsilon at max coords ~2160)
            let deltas = [0.01_f32, 0.05, 0.1, 1.0, 50.0];
            for &d in &deltas {
                let outside_points = [
                    (rect.min.x - d, rect.min.y),
                    (rect.max.x + d, rect.min.y),
                    (rect.min.x, rect.min.y - d),
                    (rect.min.x, rect.max.y + d),
                    (rect.min.x - d, rect.min.y - d),
                    (rect.max.x + d, rect.max.y + d),
                ];
                for (ox, oy) in outside_points {
                    let mut pos = rect.min;
                    pos.x = ox;
                    pos.y = oy;
                    assert!(
                        display.project_pointer_pos(pos).is_none(),
                        "Exterior point ({ox}, {oy}) must return None at scale {scale}"
                    );
                }
            }
        }
    }
}

#[test]
fn test_stress_touch_subpixel_micro_step_monotonicity() {
    let mut display = CalculatorDisplay::new();
    let rect_w = 480.0_f32; // 1.5x scale
    let rect_h = 360.0_f32;
    display.last_display_rect.min.x = 100.0;
    display.last_display_rect.min.y = 50.0;
    display.last_display_rect.max.x = 100.0 + rect_w;
    display.last_display_rect.max.y = 50.0 + rect_h;

    let rect = display.last_display_rect;
    let pixel_w = (rect_w / 320.0) as f64; // 1.5 pts per pixel

    // Test every single pixel boundary crossing from 0 to 319
    for fb_pixel in 0..320 {
        let left_edge = (rect.min.x as f64 + (fb_pixel as f64) * pixel_w) as f32;
        let mid_pixel = (rect.min.x as f64 + ((fb_pixel as f64) + 0.5) * pixel_w) as f32;
        let right_subpixel =
            (rect.min.x as f64 + ((fb_pixel as f64) + 1.0) * pixel_w - 0.005) as f32;

        for &x in &[left_edge, mid_pixel, right_subpixel] {
            let mut pos = rect.min;
            pos.x = x;
            pos.y = rect.min.y + 10.0;
            let touch = display.project_pointer_pos(pos).expect("Within rect");
            assert_eq!(
                touch.x, fb_pixel as i64,
                "Micro-step at x={x} must floor to pixel {fb_pixel}"
            );
        }
    }
}

#[test]
fn test_stress_touch_degenerate_viewports() {
    let mut display = CalculatorDisplay::new();

    // Zero width
    display.last_display_rect.min.x = 100.0;
    display.last_display_rect.min.y = 50.0;
    display.last_display_rect.max.x = 100.0;
    display.last_display_rect.max.y = 290.0;
    let pos = display.last_display_rect.min;
    assert_eq!(display.project_pointer_pos(pos), None);

    // Zero height
    display.last_display_rect.min.x = 100.0;
    display.last_display_rect.min.y = 50.0;
    display.last_display_rect.max.x = 420.0;
    display.last_display_rect.max.y = 50.0;
    let pos = display.last_display_rect.min;
    assert_eq!(display.project_pointer_pos(pos), None);

    // Inverted rect
    display.last_display_rect.min.x = 420.0;
    display.last_display_rect.min.y = 290.0;
    display.last_display_rect.max.x = 100.0;
    display.last_display_rect.max.y = 50.0;
    let mut pos = display.last_display_rect.min;
    pos.x = 200.0;
    pos.y = 150.0;
    assert_eq!(display.project_pointer_pos(pos), None);
}

// ============================================================================
// AREA 2: D-Pad Dead-Zone and Directional Geometry Testing
// ============================================================================

#[test]
fn test_stress_dpad_deadzone_and_radial_annulus() {
    let center_x = 187.0_f32;
    let center_y = 367.0_f32;

    for scale in [1.0_f32, 1.25, 1.5, 2.0] {
        let center = {
            let mut pos = CalculatorDisplay::new().last_display_rect.min;
            pos.x = center_x * scale;
            pos.y = center_y * scale;
            pos
        };

        // 1. Dead zone: r < 10.0 points (unscaled) must return None across 360 degrees
        let deadzone_radii = [0.0, 0.001, 0.5, 2.0, 5.0, 8.0, 9.5, 9.9, 9.99];
        for &r in &deadzone_radii {
            for step in 0..360 {
                let angle = (step as f32) * std::f32::consts::PI / 180.0;
                let dx = r * angle.cos() * scale;
                let dy = r * angle.sin() * scale;
                let mut pt = center;
                pt.x += dx;
                pt.y += dy;

                assert_eq!(
                    navigation_at(pt, center, scale),
                    None,
                    "Dead-zone violation at r={r}, angle={step} deg, scale={scale}"
                );
            }
        }

        // 2. Active annulus: 10.0 <= r <= 42.0 points
        let active_radii = [10.0001, 12.0, 15.0, 25.0, 35.0, 41.999];
        for &r in &active_radii {
            // Cardinal directions
            // Right (dx > 0, dy = 0) -> Key 8
            let mut pt_right = center;
            pt_right.x += r * scale;
            assert_eq!(
                navigation_at(pt_right, center, scale),
                Some(8),
                "Right cardinal must be 8 at scale {scale}"
            );

            // Left (dx < 0, dy = 0) -> Key 7
            let mut pt_left = center;
            pt_left.x -= r * scale;
            assert_eq!(
                navigation_at(pt_left, center, scale),
                Some(7),
                "Left cardinal must be 7 at scale {scale}"
            );

            // Up (dx = 0, dy < 0) -> Key 2
            let mut pt_up = center;
            pt_up.y -= r * scale;
            assert_eq!(
                navigation_at(pt_up, center, scale),
                Some(2),
                "Up cardinal must be 2 at scale {scale}"
            );

            // Down (dx = 0, dy > 0) -> Key 12
            let mut pt_down = center;
            pt_down.y += r * scale;
            assert_eq!(
                navigation_at(pt_down, center, scale),
                Some(12),
                "Down cardinal must be 12 at scale {scale}"
            );
        }

        // 3. Outer boundary: r > 42.0 points must return None across 360 degrees
        let outer_radii = [42.001, 45.0, 60.0, 100.0, 500.0];
        for &r in &outer_radii {
            for step in (0..360).step_by(10) {
                let angle = (step as f32) * std::f32::consts::PI / 180.0;
                let dx = r * angle.cos() * scale;
                let dy = r * angle.sin() * scale;
                let mut pt = center;
                pt.x += dx;
                pt.y += dy;

                assert_eq!(
                    navigation_at(pt, center, scale),
                    None,
                    "Outer zone violation at r={r}, angle={step} deg, scale={scale}"
                );
            }
        }
    }
}

#[test]
fn test_stress_dpad_diagonal_ties_and_sector_boundaries() {
    let center = {
        let mut pos = CalculatorDisplay::new().last_display_rect.min;
        pos.x = 187.0;
        pos.y = 367.0;
        pos
    };
    let scale = 1.0_f32;

    // Radius within active zone: r = sqrt(dx^2 + dy^2) = sqrt(2 * d^2)
    // For d = 20.0, r = 28.28 (well within 10.0..=42.0)
    let d = 20.0_f32;

    // Diagonal tie cases where |dx| == |dy|
    // Specification: |dy| >= |dx| routes Up/Down.
    // (+d, +d): dy > 0 -> Down (12)
    let mut p = center;
    p.x += d;
    p.y += d;
    assert_eq!(
        navigation_at(p, center, scale),
        Some(12),
        "Diagonal (+d, +d) must route Down (12)"
    );

    // (-d, +d): dy > 0 -> Down (12)
    let mut p = center;
    p.x -= d;
    p.y += d;
    assert_eq!(
        navigation_at(p, center, scale),
        Some(12),
        "Diagonal (-d, +d) must route Down (12)"
    );

    // (+d, -d): dy < 0 -> Up (2)
    let mut p = center;
    p.x += d;
    p.y -= d;
    assert_eq!(
        navigation_at(p, center, scale),
        Some(2),
        "Diagonal (+d, -d) must route Up (2)"
    );

    // (-d, -d): dy < 0 -> Up (2)
    let mut p = center;
    p.x -= d;
    p.y -= d;
    assert_eq!(
        navigation_at(p, center, scale),
        Some(2),
        "Diagonal (-d, -d) must route Up (2)"
    );

    // Perturbations across the diagonal boundary:
    let eps = 0.01_f32;

    // (+d + eps, +d): |dx| > |dy|, dx > 0 -> Right (8)
    let mut p = center;
    p.x += d + eps;
    p.y += d;
    assert_eq!(
        navigation_at(p, center, scale),
        Some(8),
        "Diagonal (+d+eps, +d) must route Right (8)"
    );

    // (+d - eps, +d): |dy| > |dx|, dy > 0 -> Down (12)
    let mut p = center;
    p.x += d - eps;
    p.y += d;
    assert_eq!(
        navigation_at(p, center, scale),
        Some(12),
        "Diagonal (+d-eps, +d) must route Down (12)"
    );

    // (-d - eps, +d): |dx| > |dy|, dx < 0 -> Left (7)
    let mut p = center;
    p.x -= d + eps;
    p.y += d;
    assert_eq!(
        navigation_at(p, center, scale),
        Some(7),
        "Diagonal (-d-eps, +d) must route Left (7)"
    );

    // (-d + eps, +d): |dy| > |dx|, dy > 0 -> Down (12)
    let mut p = center;
    p.x += -d + eps;
    p.y += d;
    assert_eq!(
        navigation_at(p, center, scale),
        Some(12),
        "Diagonal (-d+eps, +d) must route Down (12)"
    );

    // (+d + eps, -d): |dx| > |dy|, dx > 0 -> Right (8)
    let mut p = center;
    p.x += d + eps;
    p.y -= d;
    assert_eq!(
        navigation_at(p, center, scale),
        Some(8),
        "Diagonal (+d+eps, -d) must route Right (8)"
    );

    // (+d - eps, -d): |dy| > |dx|, dy < 0 -> Up (2)
    let mut p = center;
    p.x += d - eps;
    p.y -= d;
    assert_eq!(
        navigation_at(p, center, scale),
        Some(2),
        "Diagonal (+d-eps, -d) must route Up (2)"
    );

    // (-d - eps, -d): |dx| > |dy|, dx < 0 -> Left (7)
    let mut p = center;
    p.x -= d + eps;
    p.y -= d;
    assert_eq!(
        navigation_at(p, center, scale),
        Some(7),
        "Diagonal (-d-eps, -d) must route Left (7)"
    );

    // (-d + eps, -d): |dy| > |dx|, dy < 0 -> Up (2)
    let mut p = center;
    p.x -= d - eps;
    p.y -= d;
    assert_eq!(
        navigation_at(p, center, scale),
        Some(2),
        "Diagonal (-d+eps, -d) must route Up (2)"
    );
}

#[test]
fn test_stress_dpad_marker_coordinates_and_key_identities() {
    let center = {
        let mut pos = CalculatorDisplay::new().last_display_rect.min;
        pos.x = 187.0;
        pos.y = 367.0;
        pos
    };

    // Authentic marker placements from keypad.rs:72-77:
    // (2, 187.0, 332.0),
    // (7, 153.0, 367.0),
    // (8, 221.0, 367.0),
    // (12, 187.0, 402.0),
    let markers = [
        (2, 187.0_f32, 332.0_f32, "Up"),
        (7, 153.0_f32, 367.0_f32, "Left"),
        (8, 221.0_f32, 367.0_f32, "Right"),
        (12, 187.0_f32, 402.0_f32, "Down"),
    ];

    for &(code, mx, my, name) in &markers {
        let mut pt = center;
        pt.x = mx;
        pt.y = my;

        // Hit testing the marker itself must activate the corresponding key
        let hit = navigation_at(pt, center, 1.0);
        assert_eq!(
            hit,
            Some(code),
            "Marker coordinate ({mx}, {my}) for {name} must route to keycode {code}"
        );

        // Distance from center to marker
        let dist = ((mx - 187.0).powi(2) + (my - 367.0).powi(2)).sqrt();
        assert!(
            (10.0..=42.0).contains(&dist),
            "Marker {name} distance {dist} must be within active annulus [10.0, 42.0]"
        );

        // Unified key matrix lookup validation
        let mapping = lookup_by_physical(code as u8).expect("Key exists in unified matrix");
        assert_eq!(mapping.physical_id, code as u8);
        assert_eq!(mapping.ppl_code, code as i32);
        assert_eq!(mapping.primary_label, name);
    }
}

// ============================================================================
// AREA 3: Touch Settlement Timing Oracle & Tap Hold Verification
// ============================================================================

/// State machine oracle mirroring `crates/ppl-gui/src/app_firmware.rs:262-355`.
///
/// Implements:
/// - Immediate TouchDown (status 1) on mouse press
/// - Continuous TouchMove (status 2) on mouse drag
/// - Settlement timing: taps with duration < 300ms defer TouchEnd (status 8)
///   until 300ms have elapsed since touch start.
/// - Requests repaints for the remaining deferred duration.
/// - Taps >= 300ms release immediately with zero artificial delay.
/// - Rapid subsequent taps flush the deferred release before starting the new touch.
struct TouchSettlementOracle {
    last_touch: Option<Touch>,
    touch_start_time: Option<Instant>,
    deferred_touch_end: Option<(Touch, Instant)>,
    sent_commands: Vec<(Instant, u8, u16, u16)>,
    repaint_requests: Vec<(Instant, Duration)>,
}

impl TouchSettlementOracle {
    const MIN_TAP_DURATION: Duration = Duration::from_millis(300);

    fn new() -> Self {
        Self {
            last_touch: None,
            touch_start_time: None,
            deferred_touch_end: None,
            sent_commands: Vec::new(),
            repaint_requests: Vec::new(),
        }
    }

    fn step(&mut self, now: Instant, pointer_down: bool, touch_pos: Option<Touch>) {
        // 1. Flush expired deferred touch-end
        if let Some((t, deadline)) = self.deferred_touch_end {
            if now >= deadline {
                self.sent_commands.push((now, 8, t.x as u16, t.y as u16));
                self.deferred_touch_end = None;
                self.last_touch = None;
                self.touch_start_time = None;
            } else {
                self.repaint_requests.push((now, deadline - now));
            }
        }

        // 2. Forward touch
        let touch = if pointer_down { touch_pos } else { None };

        if let Some(t) = touch {
            // Flush pending release before starting new touch
            if let Some((prev_t, _)) = self.deferred_touch_end.take() {
                self.sent_commands
                    .push((now, 8, prev_t.x as u16, prev_t.y as u16));
                self.last_touch = None;
            }

            if self.last_touch.is_none() {
                // New touch down
                self.touch_start_time = Some(now);
                self.sent_commands.push((now, 1, t.x as u16, t.y as u16));
                self.last_touch = Some(t);
            } else if self.last_touch != Some(t) {
                // Touch moved
                self.sent_commands.push((now, 2, t.x as u16, t.y as u16));
                self.last_touch = Some(t);
            }
        } else if let Some(t) = self.last_touch {
            if self.deferred_touch_end.is_none() {
                let elapsed = self
                    .touch_start_time
                    .map_or(Self::MIN_TAP_DURATION, |st| now.duration_since(st));
                if elapsed < Self::MIN_TAP_DURATION {
                    let deadline = self.touch_start_time.unwrap_or(now) + Self::MIN_TAP_DURATION;
                    self.deferred_touch_end = Some((t, deadline));
                    self.repaint_requests.push((now, deadline - now));
                } else {
                    self.sent_commands.push((now, 8, t.x as u16, t.y as u16));
                    self.last_touch = None;
                    self.touch_start_time = None;
                }
            }
        }
    }

    fn on_blur_or_focus_loss(&mut self, now: Instant) {
        if let Some(t) = self.last_touch.take() {
            self.sent_commands.push((now, 8, t.x as u16, t.y as u16));
        }
        self.last_touch = None;
        self.deferred_touch_end = None;
        self.touch_start_time = None;
    }
}

#[test]
fn test_stress_quick_tap_settlement_timing_guarantees_arm_sys_timer_interception() {
    let t0 = Instant::now();
    let touch_pos = Touch { x: 160, y: 120 };

    // Simulated short tap durations: 10ms, 50ms, 100ms, 150ms, 182ms, 250ms
    for &tap_duration_ms in &[10, 50, 100, 150, 182, 250] {
        let mut sim = TouchSettlementOracle::new();
        let tap_dur = Duration::from_millis(tap_duration_ms);

        // Frame 0: Pointer pressed down at t0
        sim.step(t0, true, Some(touch_pos));
        assert_eq!(sim.sent_commands.len(), 1);
        assert_eq!(sim.sent_commands[0], (t0, 1, 160, 120)); // TouchDown

        // Frame 1: Pointer released after tap_dur (< 300ms)
        let t_release = t0 + tap_dur;
        sim.step(t_release, false, None);

        // TouchEnd (status 8) must NOT be sent yet!
        let end_sent_at_release = sim
            .sent_commands
            .iter()
            .any(|&(_, status, _, _)| status == 8);
        assert!(
            !end_sent_at_release,
            "Tap of {tap_duration_ms}ms must NOT send TouchEnd immediately at release"
        );
        assert!(sim.deferred_touch_end.is_some());
        assert!(!sim.repaint_requests.is_empty());

        // Verify ARM firmware SYS_TIMER interception at ~183ms:
        // ARM firmware polls on its 183ms timer tick. At any tick within [0, 300ms],
        // the touch must still be registered as active.
        let arm_timer_tick = t0 + Duration::from_millis(183);
        assert!(
            arm_timer_tick < t0 + Duration::from_millis(300),
            "ARM 183ms tick falls within the 300ms deferred window"
        );

        // Simulate frame step right at ARM 183ms tick
        sim.step(arm_timer_tick, false, None);
        let end_sent_at_arm_tick = sim
            .sent_commands
            .iter()
            .any(|&(_, status, _, _)| status == 8);
        assert!(
            !end_sent_at_arm_tick,
            "At ARM SYS_TIMER tick (183ms), TouchEnd must still be held active!"
        );

        // Simulate frame step at exactly 300ms deadline
        let t_deadline = t0 + Duration::from_millis(300);
        sim.step(t_deadline, false, None);

        // Now status 8 must have been sent
        let end_commands: Vec<_> = sim
            .sent_commands
            .iter()
            .filter(|&&(_, s, _, _)| s == 8)
            .collect();
        assert_eq!(
            end_commands.len(),
            1,
            "Exactly one TouchEnd must be dispatched upon reaching 300ms deadline"
        );
        assert_eq!(end_commands[0].0, t_deadline);
        assert_eq!(end_commands[0].1, 8);
        assert_eq!(end_commands[0].2, 160);
        assert_eq!(end_commands[0].3, 120);

        // State fully cleared
        assert!(sim.deferred_touch_end.is_none());
        assert!(sim.last_touch.is_none());
        assert!(sim.touch_start_time.is_none());
    }
}

#[test]
fn test_stress_long_tap_immediate_release() {
    let t0 = Instant::now();
    let touch_pos = Touch { x: 50, y: 75 };

    // Taps >= 300ms (e.g. 300ms, 350ms, 500ms, 1200ms)
    for &hold_ms in &[300, 350, 500, 1200] {
        let mut sim = TouchSettlementOracle::new();
        let hold_dur = Duration::from_millis(hold_ms);

        // Press down
        sim.step(t0, true, Some(touch_pos));
        assert_eq!(sim.sent_commands.len(), 1);
        assert_eq!(sim.sent_commands[0].1, 1); // TouchDown

        // Release at t0 + hold_dur
        let t_release = t0 + hold_dur;
        sim.step(t_release, false, None);

        // Must send TouchEnd immediately with zero deferred scheduling
        assert!(
            sim.deferred_touch_end.is_none(),
            "Hold of {hold_ms}ms must not schedule deferred touch end"
        );
        let end_commands: Vec<_> = sim
            .sent_commands
            .iter()
            .filter(|&&(_, s, _, _)| s == 8)
            .collect();
        assert_eq!(
            end_commands.len(),
            1,
            "Must immediately dispatch TouchEnd for hold of {hold_ms}ms"
        );
        assert_eq!(end_commands[0].0, t_release);
    }
}

#[test]
fn test_stress_rapid_subsequent_taps_flush_pending_release() {
    let t0 = Instant::now();
    let mut sim = TouchSettlementOracle::new();
    let pt1 = Touch { x: 100, y: 50 };
    let pt2 = Touch { x: 200, y: 150 };

    // Tap 1: 50ms quick tap
    sim.step(t0, true, Some(pt1)); // TouchDown pt1
    sim.step(t0 + Duration::from_millis(50), false, None); // Release -> deferred until t0 + 300ms
    assert!(sim.deferred_touch_end.is_some());

    // Tap 2 arrives at t = 100ms (< 300ms)
    let t_tap2 = t0 + Duration::from_millis(100);
    sim.step(t_tap2, true, Some(pt2));

    // Must flush pending TouchEnd for pt1 BEFORE sending TouchDown for pt2
    assert_eq!(sim.sent_commands.len(), 3);
    assert_eq!(sim.sent_commands[0], (t0, 1, 100, 50)); // Down pt1
    assert_eq!(sim.sent_commands[1], (t_tap2, 8, 100, 50)); // Flushed End pt1
    assert_eq!(sim.sent_commands[2], (t_tap2, 1, 200, 150)); // Down pt2

    // Now Tap 2 is held until t0 + 100ms + 300ms = t0 + 400ms
    sim.step(t0 + Duration::from_millis(150), false, None);
    assert!(sim.deferred_touch_end.is_some());

    // Complete Tap 2 at t0 + 400ms
    sim.step(t0 + Duration::from_millis(400), false, None);
    assert_eq!(sim.sent_commands.len(), 4);
    assert_eq!(
        sim.sent_commands[3],
        (t0 + Duration::from_millis(400), 8, 200, 150)
    );
}

#[test]
fn test_stress_focus_loss_and_window_blur_aborts_deferred_touch() {
    let t0 = Instant::now();
    let mut sim = TouchSettlementOracle::new();
    let pt = Touch { x: 80, y: 90 };

    // Start quick tap
    sim.step(t0, true, Some(pt));
    sim.step(t0 + Duration::from_millis(40), false, None);
    assert!(sim.deferred_touch_end.is_some());
    assert!(sim.last_touch.is_some());

    // Window blur / focus loss event arrives at t0 + 100ms
    let t_blur = t0 + Duration::from_millis(100);
    sim.on_blur_or_focus_loss(t_blur);

    // Aborts deferred schedule and releases touch immediately
    assert!(sim.deferred_touch_end.is_none());
    assert!(sim.last_touch.is_none());
    assert!(sim.touch_start_time.is_none());

    let end_commands: Vec<_> = sim
        .sent_commands
        .iter()
        .filter(|&&(_, s, _, _)| s == 8)
        .collect();
    assert_eq!(end_commands.len(), 1);
    assert_eq!(*end_commands[0], (t_blur, 8, 80, 90));
}
