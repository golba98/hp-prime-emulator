//! Small resolution-independent UI icons, drawn without font glyphs.
use egui::{Color32, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};
#[derive(Clone, Copy)]
pub enum Icon {
    New,
    Open,
    Save,
    Compile,
    Run,
    Stop,
    Restart,
    Debug,
    Explorer,
    Calculator,
    Layout,
    Settings,
    Close,
}
pub fn button(ui: &mut Ui, icon: Icon, label: &str, enabled: bool, selected: bool) -> Response {
    ui.add_enabled_ui(enabled, |ui| {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
        if response.hovered() || selected {
            ui.painter().rect_filled(rect, 3.0, crate::theme::BG_WIDGET);
        }
        let color = if !enabled {
            Color32::from_gray(80)
        } else if selected {
            crate::theme::ACCENT_BLUE
        } else {
            crate::theme::TEXT_MUTED
        };
        paint(
            ui,
            icon,
            Rect::from_center_size(rect.center(), Vec2::splat(16.0)),
            color,
        );
        response.on_hover_text(label)
    })
    .inner
}
pub fn paint(ui: &Ui, icon: Icon, r: Rect, c: Color32) {
    let p = ui.painter();
    let s = Stroke::new(1.3, c);
    let at = |x: f32, y: f32| Pos2::new(r.left() + x * r.width(), r.top() + y * r.height());
    let line = |a, b| {
        p.line_segment([a, b], s);
    };
    let box_at = |a, b| {
        p.rect_stroke(Rect::from_min_max(a, b), 1.0, s, egui::StrokeKind::Inside);
    };
    match icon {
        Icon::New => {
            box_at(at(0.18, 0.05), at(0.8, 0.95));
            line(at(0.35, 0.5), at(0.65, 0.5));
            line(at(0.5, 0.35), at(0.5, 0.65));
        }
        Icon::Open => {
            box_at(at(0.05, 0.3), at(0.95, 0.85));
            line(at(0.05, 0.3), at(0.05, 0.15));
            line(at(0.05, 0.15), at(0.4, 0.15));
            line(at(0.4, 0.15), at(0.55, 0.3));
        }
        Icon::Save => {
            box_at(at(0.12, 0.1), at(0.88, 0.9));
            box_at(at(0.3, 0.1), at(0.7, 0.4));
            box_at(at(0.3, 0.6), at(0.7, 0.9));
        }
        Icon::Run => {
            p.add(egui::Shape::convex_polygon(
                vec![at(0.25, 0.1), at(0.9, 0.5), at(0.25, 0.9)],
                c,
                Stroke::NONE,
            ));
        }
        Icon::Stop => {
            p.rect_filled(Rect::from_min_max(at(0.2, 0.2), at(0.8, 0.8)), 1.0, c);
        }
        Icon::Compile => {
            line(at(0.1, 0.5), at(0.38, 0.8));
            line(at(0.38, 0.8), at(0.9, 0.15));
        }
        Icon::Restart => {
            p.circle_stroke(r.center(), r.width() * 0.35, s);
            p.rect_filled(
                Rect::from_min_max(at(0.55, 0.0), at(1.0, 0.4)),
                0.0,
                crate::theme::BG_PANEL,
            );
            line(at(0.8, 0.05), at(0.8, 0.4));
            line(at(0.8, 0.4), at(0.45, 0.4));
        }
        Icon::Debug => {
            box_at(at(0.28, 0.25), at(0.72, 0.85));
            for y in [0.35, 0.55, 0.75] {
                line(at(0.05, y), at(0.28, y));
                line(at(0.72, y), at(0.95, y));
            }
            line(at(0.35, 0.25), at(0.25, 0.05));
            line(at(0.65, 0.25), at(0.75, 0.05));
        }
        Icon::Explorer => {
            box_at(at(0.25, 0.1), at(0.9, 0.8));
            line(at(0.1, 0.25), at(0.1, 0.95));
            line(at(0.1, 0.95), at(0.7, 0.95));
        }
        Icon::Calculator => {
            box_at(at(0.18, 0.02), at(0.82, 0.98));
            box_at(at(0.28, 0.12), at(0.72, 0.4));
            for y in [0.55, 0.72, 0.87] {
                for x in [0.32, 0.5, 0.68] {
                    p.circle_filled(at(x, y), 0.8, c);
                }
            }
        }
        Icon::Layout => {
            box_at(at(0.05, 0.1), at(0.95, 0.9));
            line(at(0.65, 0.1), at(0.65, 0.9));
            line(at(0.05, 0.65), at(0.65, 0.65));
        }
        Icon::Settings => {
            p.circle_stroke(r.center(), r.width() * 0.25, s);
            p.circle_stroke(r.center(), r.width() * 0.08, s);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let v = Vec2::angled(a);
                line(
                    r.center() + v * r.width() * 0.28,
                    r.center() + v * r.width() * 0.43,
                );
            }
        }
        Icon::Close => {
            line(at(0.25, 0.25), at(0.75, 0.75));
            line(at(0.75, 0.25), at(0.25, 0.75));
        }
    }
}
