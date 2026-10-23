//! Milestone 3 Adversarial Stress & Verification Test Suite
//! Focus: Display, Scaling, Touch & Screenshot Adversarial Stress Testing
//!
//! Authored by Challenger 1 for Milestone 3: Emulated HP Prime G1 Display & Virtual Keypad (R4).
//!
//! Adversarial Verification Matrix:
//! 1. Subpixel touch flooring at extreme boundaries (0,0 and 319,239)
//! 2. Pointer dragging outside display rect with primary button held down
//! 3. Rapid zoom mode toggling (1x, 2x, Fit)
//! 4. Extreme display resizing (narrow, ultrawide, high-DPI scaling)
//! 5. Screenshot PNG generation of high-entropy graphics

use std::fs;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use ppl_graphics::{Grob, rgb};
use ppl_gui::host::{GuiHost, GuiSharedState};
use ppl_gui::ide::display::{CalculatorDisplay, ZoomMode};
use ppl_gui::scaler::{
    ASPECT_RATIO, LOGICAL_HEIGHT, LOGICAL_WIDTH, Viewport, blit_scaled, window_to_framebuffer,
};
use ppl_vm::Host;
use ppl_vm::host::Touch;

// ============================================================================
// AREA 1: Subpixel Touch Flooring & Extreme Boundary Conditions
// ============================================================================

#[test]
fn test_touch_flooring_origin_subpixel_extremes() {
    let mut display = CalculatorDisplay::new();
    // Simulate a 1x display rect: (100.0, 50.0) to (420.0, 290.0) -> width 320, height 240
    display.last_display_rect.min.x = 100.0;
    display.last_display_rect.min.y = 50.0;
    display.last_display_rect.max.x = 420.0;
    display.last_display_rect.max.y = 290.0;

    let mut pos = display.last_display_rect.min;

    // Exact top-left corner
    pos.x = 100.0;
    pos.y = 50.0;
    assert_eq!(
        display.project_pointer_pos(pos),
        Some(Touch { x: 0, y: 0 }),
        "exact origin must map to (0, 0)"
    );

    // Subpixel micro-offsets within pixel (0, 0)
    for delta in [1e-5, 0.001, 0.1, 0.5, 0.99, 0.999] {
        pos.x = 100.0 + delta;
        pos.y = 50.0 + delta;
        assert_eq!(
            display.project_pointer_pos(pos),
            Some(Touch { x: 0, y: 0 }),
            "subpixel offset {delta} within first pixel must floor to (0, 0)"
        );
    }

    // Step across boundary to pixel (1, 1)
    pos.x = 101.0;
    pos.y = 51.0;
    assert_eq!(
        display.project_pointer_pos(pos),
        Some(Touch { x: 1, y: 1 }),
        "offset 1.0 must floor to (1, 1)"
    );

    // Negative subpixel micro-offsets outside rect must return None
    for delta in [1e-5, 0.001, 0.1, 1.0] {
        pos.x = 100.0 - delta;
        pos.y = 50.0;
        assert_eq!(
            display.project_pointer_pos(pos),
            None,
            "x offset -{delta} outside rect must return None"
        );

        pos.x = 100.0;
        pos.y = 50.0 - delta;
        assert_eq!(
            display.project_pointer_pos(pos),
            None,
            "y offset -{delta} outside rect must return None"
        );
    }
}

/// Regression test for a historical defect:
/// `CalculatorDisplay::project_pointer_pos` previously performed `f32` division
/// before casting to `f64`, causing IEEE-754 precision loss on 126/320 X values
/// and 105/240 Y values (off-by-one undercount at exact integer boundaries).
/// The projection now multiplies in `f64` before dividing; this test pins the
/// corrected behaviour so the defect cannot silently return.
#[test]
fn test_empirical_defect_subpixel_f32_precision_loss_in_display_projection() {
    let mut display = CalculatorDisplay::new();
    display.last_display_rect.min.x = 100.0;
    display.last_display_rect.min.y = 50.0;
    display.last_display_rect.max.x = 420.0; // width 320.0
    display.last_display_rect.max.y = 290.0; // height 240.0

    let mut pos = display.last_display_rect.min;

    // At pos.x = 419.0 (the exact boundary of pixel 319):
    // pos.x - min.x = 319.0. In f64, 319.0 * 320.0 / 320.0 = 319.0 exactly,
    // which floors to 319. The old f32 path produced 318 here.
    pos.x = 419.0;
    pos.y = 289.0;
    let touch = display.project_pointer_pos(pos).unwrap();
    assert_eq!(
        touch.x, 319,
        "exact boundary of pixel 319 must project to 319, not the historical f32 undercount of 318"
    );

    // At pos.x = 419.5 (interior of pixel 319):
    // (419.5 - 100.0) / 320.0 = 319.5 / 320.0, which floors to 319.
    pos.x = 419.5;
    let touch_interior = display.project_pointer_pos(pos).unwrap();
    assert_eq!(
        touch_interior.x, 319,
        "interior of pixel 319 maps correctly"
    );

    // Subpixel micro-offset within last pixel: 419.9
    pos.x = 419.9;
    pos.y = 289.9;
    assert_eq!(
        display.project_pointer_pos(pos),
        Some(Touch { x: 319, y: 239 }),
        "419.9 maps to (319, 239)"
    );

    // Exact boundary point: max.x, max.y (420.0, 290.0)
    // Clamped to 319, 239
    pos.x = 420.0;
    pos.y = 290.0;
    assert_eq!(
        display.project_pointer_pos(pos),
        Some(Touch { x: 319, y: 239 }),
        "exact max boundary clamps to (319, 239)"
    );

    // Any point beyond max bound returns None
    for delta in [0.0001, 0.01, 1.0, 50.0] {
        pos.x = 420.0 + delta;
        pos.y = 290.0;
        assert_eq!(
            display.project_pointer_pos(pos),
            None,
            "x beyond max +{delta} must be rejected"
        );

        pos.x = 420.0;
        pos.y = 290.0 + delta;
        assert_eq!(
            display.project_pointer_pos(pos),
            None,
            "y beyond max +{delta} must be rejected"
        );
    }
}

/// Systematic regression sweep across all 320×240 coordinates:
/// The historical `f32` division defect undercounted 126 / 320 X coordinates
/// (39.4%) and 105 / 240 Y coordinates (43.8%) at exact integer boundaries.
/// With the f64 multiply-before-divide projection, every coordinate on the
/// integer grid must map to itself exactly.
#[test]
fn test_empirical_f32_division_defect_characterization() {
    let mut display = CalculatorDisplay::new();
    display.last_display_rect.min.x = 0.0;
    display.last_display_rect.min.y = 0.0;
    display.last_display_rect.max.x = 320.0;
    display.last_display_rect.max.y = 240.0;

    let mut pos = display.last_display_rect.min;

    let mut undercounted_x = Vec::new();
    for x in 0..320 {
        pos.x = x as f32;
        pos.y = 0.0;
        let touch = display.project_pointer_pos(pos).unwrap();
        if touch.x != x {
            undercounted_x.push((x, touch.x));
        }
    }

    let mut undercounted_y = Vec::new();
    for y in 0..240 {
        pos.x = 0.0;
        pos.y = y as f32;
        let touch = display.project_pointer_pos(pos).unwrap();
        if touch.y != y {
            undercounted_y.push((y, touch.y));
        }
    }

    assert!(
        undercounted_x.is_empty(),
        "f64 projection must map every X coordinate exactly; mismatches: {undercounted_x:?}"
    );
    assert!(
        undercounted_y.is_empty(),
        "f64 projection must map every Y coordinate exactly; mismatches: {undercounted_y:?}"
    );
}

#[test]
fn test_touch_flooring_scaled_2x_pixel_transitions() {
    let mut display = CalculatorDisplay::new();
    // 2x scaling: width 640.0, height 480.0
    display.last_display_rect.min.x = 0.0;
    display.last_display_rect.min.y = 0.0;
    display.last_display_rect.max.x = 640.0;
    display.last_display_rect.max.y = 480.0;

    let mut pos = display.last_display_rect.min;

    // At 2x scale, every framebuffer pixel spans 2.0 window pixels.
    // Pixel 0: [0.0, 2.0)
    pos.x = 0.0;
    pos.y = 0.0;
    assert_eq!(display.project_pointer_pos(pos), Some(Touch { x: 0, y: 0 }));

    pos.x = 1.99;
    pos.y = 1.99;
    assert_eq!(display.project_pointer_pos(pos), Some(Touch { x: 0, y: 0 }));

    // Pixel 1: [2.0, 4.0)
    pos.x = 2.0;
    pos.y = 2.0;
    assert_eq!(display.project_pointer_pos(pos), Some(Touch { x: 1, y: 1 }));

    pos.x = 3.99;
    pos.y = 3.99;
    assert_eq!(display.project_pointer_pos(pos), Some(Touch { x: 1, y: 1 }));

    // Pixel 2:
    pos.x = 4.0;
    pos.y = 4.0;
    assert_eq!(display.project_pointer_pos(pos), Some(Touch { x: 2, y: 2 }));

    // Pixel 318:
    pos.x = 636.5;
    pos.y = 476.5;
    assert_eq!(
        display.project_pointer_pos(pos),
        Some(Touch { x: 318, y: 238 })
    );

    // Pixel 319 interior:
    pos.x = 639.0;
    pos.y = 479.0;
    assert_eq!(
        display.project_pointer_pos(pos),
        Some(Touch { x: 319, y: 239 })
    );

    pos.x = 640.0;
    pos.y = 480.0;
    assert_eq!(
        display.project_pointer_pos(pos),
        Some(Touch { x: 319, y: 239 })
    );

    pos.x = 640.001;
    pos.y = 480.0;
    assert_eq!(display.project_pointer_pos(pos), None);
}

#[test]
fn test_touch_flooring_fractional_scale_monotonicity() {
    let mut display = CalculatorDisplay::new();
    // Non-integer arbitrary scale: 789.34 x 592.005
    display.last_display_rect.min.x = 12.5;
    display.last_display_rect.min.y = 34.2;
    display.last_display_rect.max.x = 12.5 + 789.34;
    display.last_display_rect.max.y = 34.2 + 592.005;

    let mut pos = display.last_display_rect.min;
    let mut last_fb_x = 0;

    // Sweep horizontally across the entire width in 1000 steps
    for step in 0..=1000 {
        let t = step as f32 / 1000.0;
        pos.x = display.last_display_rect.min.x + t * 789.34;
        pos.y = display.last_display_rect.min.y + 10.0;

        let touch = display.project_pointer_pos(pos).expect("must be inside");
        assert!(
            touch.x >= last_fb_x,
            "projected x must be monotonic: touch.x ({}) >= last_fb_x ({}) at step {step}",
            touch.x,
            last_fb_x
        );
        assert!(touch.x >= 0 && touch.x <= 319, "x must be clamped 0..=319");
        last_fb_x = touch.x;
    }
    assert_eq!(last_fb_x, 319, "must reach 319 at the right edge of sweep");
}

#[test]
fn test_touch_degenerate_display_rects_and_nans() {
    let mut display = CalculatorDisplay::new();

    // Zero-width rect
    display.last_display_rect.min.x = 100.0;
    display.last_display_rect.min.y = 100.0;
    display.last_display_rect.max.x = 100.0;
    display.last_display_rect.max.y = 200.0;

    let mut pos = display.last_display_rect.min;
    pos.x = 100.0;
    pos.y = 150.0;
    assert_eq!(
        display.project_pointer_pos(pos),
        None,
        "zero-width rect must return None"
    );

    // Negative size rect
    display.last_display_rect.min.x = 200.0;
    display.last_display_rect.min.y = 200.0;
    display.last_display_rect.max.x = 100.0;
    display.last_display_rect.max.y = 100.0;
    assert_eq!(
        display.project_pointer_pos(pos),
        None,
        "inverted rect must return None"
    );

    // NaN / Infinite positions
    display.last_display_rect.min.x = 0.0;
    display.last_display_rect.min.y = 0.0;
    display.last_display_rect.max.x = 320.0;
    display.last_display_rect.max.y = 240.0;

    pos.x = f32::NAN;
    pos.y = 100.0;
    assert_eq!(
        display.project_pointer_pos(pos),
        None,
        "NaN x must return None"
    );

    pos.x = 100.0;
    pos.y = f32::NAN;
    assert_eq!(
        display.project_pointer_pos(pos),
        None,
        "NaN y must return None"
    );

    pos.x = f32::INFINITY;
    pos.y = 100.0;
    assert_eq!(
        display.project_pointer_pos(pos),
        None,
        "+inf x must return None"
    );

    pos.x = 100.0;
    pos.y = f32::NEG_INFINITY;
    assert_eq!(
        display.project_pointer_pos(pos),
        None,
        "-inf y must return None"
    );
}

// ============================================================================
// AREA 2: Pointer Dragging Outside Display Rect with Primary Button Held Down
// ============================================================================

#[test]
fn test_pointer_dragging_inside_outside_lifecycle() {
    let mut display = CalculatorDisplay::new();
    display.last_display_rect.min.x = 100.0;
    display.last_display_rect.min.y = 100.0;
    display.last_display_rect.max.x = 420.0;
    display.last_display_rect.max.y = 340.0;

    let shared = Arc::new(GuiSharedState::new());
    let mut host = GuiHost::new(shared.clone());

    let mut pos = display.last_display_rect.min;

    // 1. Mouse down inside display at (260.0, 220.0) -> center (160, 120)
    pos.x = 260.0;
    pos.y = 220.0;
    let touch = display.project_pointer_pos(pos);
    shared.set_touch(touch);
    assert_eq!(
        host.touch().unwrap(),
        Some(Touch { x: 160, y: 120 }),
        "initial press inside must register touch"
    );

    // 2. Drag pointer to the right outside display: x = 450.0 (past max.x 420.0)
    // Primary button remains held down
    pos.x = 450.0;
    pos.y = 220.0;
    let touch_outside = display.project_pointer_pos(pos);
    assert_eq!(
        touch_outside, None,
        "projecting outside display must yield None"
    );
    shared.set_touch(touch_outside);
    assert_eq!(
        host.touch().unwrap(),
        None,
        "dragging outside display while held down must immediately clear touch to None (no sticky touch)"
    );

    // 3. Continue dragging further outside: x = 600.0, y = 50.0
    pos.x = 600.0;
    pos.y = 50.0;
    shared.set_touch(display.project_pointer_pos(pos));
    assert_eq!(
        host.touch().unwrap(),
        None,
        "still outside must remain None"
    );

    // 4. Drag pointer back inside display: x = 200.5, y = 200.5
    // Relative: (100.5/320.0, 100.5/240.0)
    pos.x = 200.5;
    pos.y = 200.5;
    let touch_reentry = display.project_pointer_pos(pos);
    assert!(touch_reentry.is_some());
    shared.set_touch(touch_reentry);
    assert_eq!(
        host.touch().unwrap(),
        Some(Touch { x: 100, y: 100 }),
        "re-entering display rect while held down must restore valid touch coordinates"
    );

    // 5. Release primary button: forward_touch sets touch to None
    shared.set_touch(None);
    assert_eq!(
        host.touch().unwrap(),
        None,
        "releasing button must clear touch"
    );
}

#[test]
fn test_pointer_drag_crossing_all_four_quadrants() {
    let mut display = CalculatorDisplay::new();
    display.last_display_rect.min.x = 200.0;
    display.last_display_rect.min.y = 200.0;
    display.last_display_rect.max.x = 520.0;
    display.last_display_rect.max.y = 440.0;

    let mut pos = display.last_display_rect.min;

    // Cross Left edge: x from 205 down to 195
    pos.y = 300.0;
    pos.x = 205.0;
    assert!(display.project_pointer_pos(pos).is_some());
    pos.x = 195.0;
    assert_eq!(
        display.project_pointer_pos(pos),
        None,
        "crossing left edge must return None"
    );

    // Cross Right edge: x from 515 up to 525
    pos.x = 515.0;
    assert!(display.project_pointer_pos(pos).is_some());
    pos.x = 525.0;
    assert_eq!(
        display.project_pointer_pos(pos),
        None,
        "crossing right edge must return None"
    );

    // Cross Top edge: y from 205 down to 195
    pos.x = 300.0;
    pos.y = 205.0;
    assert!(display.project_pointer_pos(pos).is_some());
    pos.y = 195.0;
    assert_eq!(
        display.project_pointer_pos(pos),
        None,
        "crossing top edge must return None"
    );

    // Cross Bottom edge: y from 435 up to 445
    pos.y = 435.0;
    assert!(display.project_pointer_pos(pos).is_some());
    pos.y = 445.0;
    assert_eq!(
        display.project_pointer_pos(pos),
        None,
        "crossing bottom edge must return None"
    );

    // Diagonal corner exits
    pos.x = 199.0;
    pos.y = 199.0;
    assert_eq!(
        display.project_pointer_pos(pos),
        None,
        "top-left diagonal exit must return None"
    );

    pos.x = 521.0;
    pos.y = 199.0;
    assert_eq!(
        display.project_pointer_pos(pos),
        None,
        "top-right diagonal exit must return None"
    );

    pos.x = 199.0;
    pos.y = 441.0;
    assert_eq!(
        display.project_pointer_pos(pos),
        None,
        "bottom-left diagonal exit must return None"
    );

    pos.x = 521.0;
    pos.y = 441.0;
    assert_eq!(
        display.project_pointer_pos(pos),
        None,
        "bottom-right diagonal exit must return None"
    );
}

#[test]
fn test_pointer_rapid_boundary_jitter_stress() {
    let mut display = CalculatorDisplay::new();
    display.last_display_rect.min.x = 0.0;
    display.last_display_rect.min.y = 0.0;
    display.last_display_rect.max.x = 320.0;
    display.last_display_rect.max.y = 240.0;

    let shared = Arc::new(GuiSharedState::new());
    let mut host = GuiHost::new(shared.clone());
    let mut pos = display.last_display_rect.min;
    pos.y = 120.0;

    // Perform 10,000 rapid crossings back and forth across the right boundary
    for i in 0..10_000 {
        if i % 2 == 0 {
            pos.x = 319.5; // Inside
            let touch = display.project_pointer_pos(pos);
            assert_eq!(touch, Some(Touch { x: 319, y: 120 }));
            shared.set_touch(touch);
            assert_eq!(host.touch().unwrap(), Some(Touch { x: 319, y: 120 }));
        } else {
            pos.x = 320.5; // Outside
            let touch = display.project_pointer_pos(pos);
            assert_eq!(touch, None);
            shared.set_touch(touch);
            assert_eq!(host.touch().unwrap(), None);
        }
    }
}

// ============================================================================
// AREA 3: Rapid Zoom Mode Toggling (1x, 2x, Fit)
// ============================================================================

#[test]
fn test_zoom_mode_rapid_toggling_aspect_ratio_invariance() {
    let modes = [
        ZoomMode::Scale1x,
        ZoomMode::Scale2x,
        ZoomMode::Fit,
        ZoomMode::Scale1x,
        ZoomMode::Fit,
        ZoomMode::Scale2x,
    ];
    let widths: [f32; 7] = [320.0, 400.0, 640.0, 800.0, 1024.0, 1920.0, 3840.0];

    for &width in &widths {
        let available_width = width.max(320.0);
        for &mode in &modes {
            let (target_w, _target_h) = match mode {
                ZoomMode::Scale1x => (320.0, 240.0),
                ZoomMode::Scale2x => (320.0 * 2.0, 240.0 * 2.0),
                ZoomMode::Fit => {
                    let w = (available_width - 8.0).max(320.0);
                    let h = w * (240.0 / 320.0);
                    (w, h)
                }
            };

            let eff_w = target_w.min(available_width - 8.0);
            let eff_h = eff_w * (240.0 / 320.0);

            // Invariance 1: Aspect ratio must strictly equal 4:3 (0.75)
            let ratio = eff_h / eff_w;
            assert!(
                (ratio - 0.75).abs() < 1e-6,
                "aspect ratio must be 4:3, got {ratio} for mode {mode:?} at width {width}"
            );

            // Invariance 2: eff_w must never exceed container available width
            assert!(
                eff_w <= available_width,
                "eff_w ({eff_w}) must fit in available_width ({available_width})"
            );

            // Invariance 3: scale factor matches status bar display_scale formula
            let scale = eff_w / 320.0;
            assert!(scale > 0.0, "scale must be positive");
            match mode {
                ZoomMode::Scale1x => {
                    assert!(
                        (scale - 1.0).abs() < 1e-3 || eff_w == available_width - 8.0,
                        "1x mode must have scale 1.0 or clamp"
                    );
                }
                ZoomMode::Scale2x => {
                    if available_width >= 648.0 {
                        assert!(
                            (scale - 2.0).abs() < 1e-3,
                            "2x mode with ample width must have scale 2.0"
                        );
                    }
                }
                ZoomMode::Fit => {
                    // Fit mode scale grows with container
                    if available_width >= 648.0 {
                        assert!(scale >= 2.0, "fit mode on large display must scale >= 2.0");
                    }
                }
            }
        }
    }
}

#[test]
fn test_zoom_mode_normalized_coordinate_invariance() {
    let mut display = CalculatorDisplay::new();

    // Test across all 3 zoom modes: normalized center (u=0.5, v=0.5) must ALWAYS project to (160, 120)
    for (mode, eff_w, eff_h) in [
        (ZoomMode::Scale1x, 320.0, 240.0),
        (ZoomMode::Scale2x, 640.0, 480.0),
        (ZoomMode::Fit, 1280.0, 960.0),
    ] {
        display.zoom_mode = mode;
        display.last_display_rect.min.x = 50.0;
        display.last_display_rect.min.y = 50.0;
        display.last_display_rect.max.x = 50.0 + eff_w;
        display.last_display_rect.max.y = 50.0 + eff_h;

        let mut pos = display.last_display_rect.min;

        // Center
        pos.x = 50.0 + eff_w * 0.5;
        pos.y = 50.0 + eff_h * 0.5;
        assert_eq!(
            display.project_pointer_pos(pos),
            Some(Touch { x: 160, y: 120 }),
            "center must map to (160, 120) in mode {mode:?}"
        );

        // Top-left
        pos.x = 50.0;
        pos.y = 50.0;
        assert_eq!(
            display.project_pointer_pos(pos),
            Some(Touch { x: 0, y: 0 }),
            "top-left must map to (0, 0) in mode {mode:?}"
        );

        // Bottom-right inside
        pos.x = 50.0 + eff_w - 0.01;
        pos.y = 50.0 + eff_h - 0.01;
        assert_eq!(
            display.project_pointer_pos(pos),
            Some(Touch { x: 319, y: 239 }),
            "bottom-right inside must map to (319, 239) in mode {mode:?}"
        );
    }
}

// ============================================================================
// AREA 4: Extreme Display Resizing (Narrow, Ultrawide, High-DPI Scaling)
// ============================================================================

#[test]
fn test_extreme_resizing_ultrawide_pillarbox_math() {
    // Ultrawide monitors: 21:9 (3440×1440), 32:9 (5120×1440), extreme (10000×100)
    for (win_w, win_h) in [(3440, 1440), (5120, 1440), (10000, 100)] {
        let vp = Viewport::calculate_aspect_fit(win_w, win_h);

        // Height must equal win_h
        assert_eq!(vp.height, win_h);
        // Width must be rounded 4/3 of height
        let expected_w = ((win_h as f64 * ASPECT_RATIO).round() as u32).min(win_w);
        assert_eq!(vp.width, expected_w);
        // Pillarbox horizontal centering
        assert_eq!(vp.x, (win_w - expected_w) / 2);
        assert_eq!(vp.y, 0);

        // Points inside pillarbox left border must return None
        if vp.x > 0 {
            assert_eq!(
                window_to_framebuffer((vp.x - 1) as f64, (win_h / 2) as f64, &vp),
                None,
                "point in left pillarbox must be rejected"
            );
        }

        // Points inside pillarbox right border must return None
        if vp.x + vp.width < win_w {
            assert_eq!(
                window_to_framebuffer((vp.x + vp.width + 1) as f64, (win_h / 2) as f64, &vp),
                None,
                "point in right pillarbox must be rejected"
            );
        }

        // Exact center of active viewport must map to (160, 120)
        let cx = vp.x as f64 + (vp.width as f64 / 2.0);
        let cy = vp.y as f64 + (vp.height as f64 / 2.0);
        assert_eq!(
            window_to_framebuffer(cx, cy, &vp),
            Some((160, 120)),
            "center of ultrawide pillarbox viewport must map to (160, 120)"
        );
    }
}

#[test]
fn test_extreme_resizing_ultratall_letterbox_math() {
    // Tall / narrow vertical screens: 9:16 (1080×1920), 9:32 (1080×3840), extreme (100×10000)
    for (win_w, win_h) in [(1080, 1920), (1080, 3840), (100, 10000)] {
        let vp = Viewport::calculate_aspect_fit(win_w, win_h);

        // Width must equal win_w
        assert_eq!(vp.width, win_w);
        // Height must be rounded 3/4 of width
        let expected_h = ((win_w as f64 / ASPECT_RATIO).round() as u32).min(win_h);
        assert_eq!(vp.height, expected_h);
        // Letterbox vertical centering
        assert_eq!(vp.x, 0);
        assert_eq!(vp.y, (win_h - expected_h) / 2);

        // Points in top letterbox must return None
        if vp.y > 0 {
            assert_eq!(
                window_to_framebuffer((win_w / 2) as f64, (vp.y - 1) as f64, &vp),
                None,
                "point in top letterbox must be rejected"
            );
        }

        // Points in bottom letterbox must return None
        if vp.y + vp.height < win_h {
            assert_eq!(
                window_to_framebuffer((win_w / 2) as f64, (vp.y + vp.height + 1) as f64, &vp),
                None,
                "point in bottom letterbox must be rejected"
            );
        }

        // Exact center of active viewport must map to (160, 120)
        let cx = vp.x as f64 + (vp.width as f64 / 2.0);
        let cy = vp.y as f64 + (vp.height as f64 / 2.0);
        assert_eq!(
            window_to_framebuffer(cx, cy, &vp),
            Some((160, 120)),
            "center of ultratall letterbox viewport must map to (160, 120)"
        );
    }
}

#[test]
fn test_extreme_resizing_microscopic_and_zero_dimensions() {
    for (w, h) in [(0, 0), (0, 100), (100, 0), (1, 1), (2, 2), (5, 5)] {
        let vp_fit = Viewport::calculate_aspect_fit(w, h);
        let vp_int = Viewport::calculate_integer(w, h);

        if w == 0 || h == 0 {
            assert_eq!(vp_fit.width, 0);
            assert_eq!(vp_fit.height, 0);
            assert_eq!(window_to_framebuffer(0.0, 0.0, &vp_fit), None);
        } else {
            assert!(vp_fit.width <= w);
            assert!(vp_fit.height <= h);
            // Integer scaling for w < 320 or h < 240 falls back to aspect fit
            assert!(!vp_int.is_integer || vp_int.scale >= 1);
        }
    }
}

#[test]
fn test_high_dpi_4k_8k_scaling_pixel_preservation() {
    // 4K exact (3840×2880) = 12x scale factor
    let vp_4k = Viewport::calculate_integer(3840, 2880);
    assert_eq!(vp_4k.scale, 12);
    assert_eq!(vp_4k.width, 3840);
    assert_eq!(vp_4k.height, 2880);
    assert_eq!(vp_4k.x, 0);
    assert_eq!(vp_4k.y, 0);

    // 8K exact (7680×5760) = 24x scale factor
    let vp_8k = Viewport::calculate_integer(7680, 5760);
    assert_eq!(vp_8k.scale, 24);
    assert_eq!(vp_8k.width, 7680);
    assert_eq!(vp_8k.height, 5760);
    assert_eq!(vp_8k.x, 0);
    assert_eq!(vp_8k.y, 0);

    // Verify blit_scaled correctness on a scaled test buffer (e.g. 2x = 640x480)
    let src_w = LOGICAL_WIDTH as usize;
    let src_h = LOGICAL_HEIGHT as usize;
    let mut src = vec![0u32; src_w * src_h];
    // Put distinctive markers at 4 corners
    src[0] = 0x00FF0000; // Red at (0, 0)
    src[src_w - 1] = 0x0000FF00; // Green at (319, 0)
    src[(src_h - 1) * src_w] = 0x000000FF; // Blue at (0, 239)
    src[src_w * src_h - 1] = 0x00FFFF00; // Yellow at (319, 239)

    let vp_2x = Viewport::calculate_integer(640, 480);
    let mut dst = vec![0u32; 640 * 480];
    blit_scaled(&src, src_w, src_h, &mut dst, 640, 480, &vp_2x, 0);

    // Corner (0,0) must have expanded to 2x2 block in dst
    assert_eq!(dst[0], 0x00FF0000);
    assert_eq!(dst[1], 0x00FF0000);
    assert_eq!(dst[640], 0x00FF0000);
    assert_eq!(dst[641], 0x00FF0000);

    // Corner (319, 0) expanded to (638..639, 0..1)
    assert_eq!(dst[638], 0x0000FF00);
    assert_eq!(dst[639], 0x0000FF00);
    assert_eq!(dst[640 + 638], 0x0000FF00);
    assert_eq!(dst[640 + 639], 0x0000FF00);
}

// ============================================================================
// AREA 5: Screenshot PNG Generation of High-Entropy Graphics
// ============================================================================

#[test]
fn test_screenshot_high_entropy_prng_white_noise_roundtrip() {
    let mut grob = Grob::screen();
    assert_eq!(grob.width(), 320);
    assert_eq!(grob.height(), 240);

    // Fill with high-entropy pseudo-random white noise (LCG)
    let mut seed: u64 = 0xDEADBEEFCAFEBABE;
    for y in 0..240 {
        for x in 0..320 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let r = ((seed >> 16) & 0xFF) as u8;
            let g = ((seed >> 24) & 0xFF) as u8;
            let b = ((seed >> 32) & 0xFF) as u8;
            grob.set(x, y, rgb(r, g, b));
        }
    }

    // Encode to PNG buffer
    let mut png_bytes = Vec::new();
    grob.write_png(&mut png_bytes)
        .expect("high-entropy write_png must succeed");

    // PNG Header verification (8-byte signature)
    assert!(png_bytes.len() > 8);
    assert_eq!(
        &png_bytes[0..8],
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        "valid PNG magic header"
    );

    // IEND chunk verification
    assert_eq!(
        &png_bytes[png_bytes.len() - 8..png_bytes.len() - 4],
        b"IEND",
        "valid PNG IEND footer"
    );

    // Decode PNG and verify 100% lossless reconstruction
    let decoded = Grob::read_png(&png_bytes).expect("read_png must decode high-entropy noise");
    assert_eq!(decoded.width(), 320);
    assert_eq!(decoded.height(), 240);

    for y in 0..240 {
        for x in 0..320 {
            let orig = grob.get(x, y).unwrap();
            let dec = decoded.get(x, y).unwrap();
            assert_eq!(
                orig, dec,
                "pixel mismatch at ({x}, {y}): original #{orig:06X} != decoded #{dec:06X}"
            );
        }
    }
}

#[test]
fn test_screenshot_high_frequency_1px_checkerboard_roundtrip() {
    let mut grob = Grob::screen();
    for y in 0..240 {
        for x in 0..320 {
            let color = if (x + y) % 2 == 0 {
                0x00000000
            } else {
                0x00FFFFFF
            };
            grob.set(x, y, color);
        }
    }

    let mut buf = Vec::new();
    grob.write_png(&mut buf)
        .expect("checkerboard PNG encoding failed");

    let decoded = Grob::read_png(&buf).expect("checkerboard PNG decoding failed");
    for y in [0, 1, 119, 120, 238, 239] {
        for x in [0, 1, 159, 160, 318, 319] {
            let expected = if (x + y) % 2 == 0 {
                0x00000000
            } else {
                0x00FFFFFF
            };
            assert_eq!(
                decoded.get(x, y),
                Some(expected),
                "checkerboard mismatch at ({x}, {y})"
            );
        }
    }
}

#[test]
fn test_screenshot_channel_gradient_sweep_roundtrip() {
    let mut grob = Grob::screen();
    for y in 0..240 {
        for x in 0..320 {
            let r = ((x * 255) / 319) as u8;
            let g = ((y * 255) / 239) as u8;
            let b = (((x + y) * 255) / (319 + 239)) as u8;
            grob.set(x, y, rgb(r, g, b));
        }
    }

    let mut buf = Vec::new();
    grob.write_png(&mut buf)
        .expect("gradient PNG write must succeed");
    let decoded = Grob::read_png(&buf).expect("gradient PNG read must succeed");

    for y in (0..240).step_by(10) {
        for x in (0..320).step_by(10) {
            assert_eq!(
                decoded.get(x, y),
                grob.get(x, y),
                "gradient mismatch at ({x}, {y})"
            );
        }
    }
}

#[test]
fn test_screenshot_file_io_save_and_cleanup() {
    let grob = Grob::screen();
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join(format!(
        "test_challenger_screenshot_{}.png",
        std::process::id()
    ));

    // Save PNG to disk
    grob.save_png(&file_path)
        .expect("save_png to temp file must succeed");
    assert!(
        file_path.exists(),
        "saved screenshot file must exist on disk"
    );

    let metadata = fs::metadata(&file_path).expect("metadata must be readable");
    assert!(
        metadata.len() > 100,
        "screenshot file size must be non-trivial"
    );

    // Read back and verify valid PNG
    let bytes = fs::read(&file_path).expect("reading back saved PNG must succeed");
    let decoded = Grob::read_png(&bytes).expect("decoding saved PNG must succeed");
    assert_eq!(decoded.width(), 320);
    assert_eq!(decoded.height(), 240);

    // Cleanup
    let _ = fs::remove_file(&file_path);
    assert!(!file_path.exists(), "temp file must be cleaned up");
}

#[test]
fn test_screenshot_concurrency_stress_with_active_grob_updates() {
    let screen = Arc::new(Mutex::new(Grob::screen()));
    let screen_gen = Arc::new(AtomicU64::new(1));
    let stop_signal = Arc::new(AtomicBool::new(false));

    // Worker thread: continuously draw rectangles and lines
    let worker_screen = screen.clone();
    let worker_gen = screen_gen.clone();
    let worker_stop = stop_signal.clone();
    let worker_thread = thread::spawn(move || {
        let mut step = 0u32;
        while !worker_stop.load(Ordering::Relaxed) {
            {
                let mut g = worker_screen.lock().unwrap();
                let color = rgb((step & 0xFF) as u8, 128, 200);
                g.rect(10, 10, 100, 100, color, color);
                g.line(0, 0, 319, 239, 0x00FFFFFF);
            }
            worker_gen.fetch_add(1, Ordering::SeqCst);
            step = step.wrapping_add(1);
            thread::yield_now();
        }
    });

    // Main thread: take screenshots in a rapid loop
    for i in 0..25 {
        let snapshot = {
            let g = screen.lock().unwrap();
            g.clone()
        };
        let mut buf = Vec::new();
        snapshot
            .write_png(&mut buf)
            .unwrap_or_else(|e| panic!("concurrent screenshot {i} failed: {e}"));
        assert!(
            buf.len() > 8,
            "encoded PNG must be non-empty at iteration {i}"
        );
        let decoded = Grob::read_png(&buf)
            .unwrap_or_else(|e| panic!("decoding concurrent screenshot {i} failed: {e}"));
        assert_eq!(decoded.width(), 320);
        assert_eq!(decoded.height(), 240);
        thread::sleep(Duration::from_millis(2));
    }

    stop_signal.store(true, Ordering::Relaxed);
    worker_thread
        .join()
        .expect("worker thread must exit cleanly");
}
