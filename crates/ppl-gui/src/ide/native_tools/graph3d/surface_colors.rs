use egui::{
    Button, Color32, Frame, Popup, PopupCloseBehavior, Vec2, color_picker, emath::RectAlign,
};

const PRESETS: [(&str, u32); 8] = [
    ("Blue", 0x0000ff),
    ("Red", 0xff0000),
    ("Green", 0x008000),
    ("Orange", 0xffa500),
    ("Purple", 0x800080),
    ("Magenta", 0xff00ff),
    ("Lime", 0x00ff00),
    ("Navy", 0x000080),
];

pub(super) fn rgb(value: u32) -> [u8; 3] {
    [(value >> 16) as u8, (value >> 8) as u8, value as u8]
}

fn color(value: u32) -> Color32 {
    let [r, g, b] = rgb(value);
    Color32::from_rgb(r, g, b)
}

pub(super) fn edit(ui: &mut egui::Ui, index: usize, colors: &mut [u32; 2]) -> bool {
    let before = *colors;
    for (side, (caption, value)) in ["Upper color", "Lower color"]
        .into_iter()
        .zip(colors.iter_mut())
        .enumerate()
    {
        ui.push_id((index, side), |ui| {
            let button = ui
                .add(
                    Button::new("")
                        .fill(color(*value))
                        .min_size(Vec2::splat(14.)),
                )
                .on_hover_text(caption);
            let style = ui.style().clone();
            let alignment = if button.rect.center().y > ui.clip_rect().center().y {
                RectAlign::TOP_START
            } else {
                RectAlign::BOTTOM_START
            };
            Popup::menu(&button)
                .frame(Frame::popup(&style))
                .align(alignment)
                .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    ui.set_style(style);
                    ui.label(caption);
                    let size = Vec2::splat(ui.spacing().interact_size.y);
                    for row in PRESETS.chunks(4) {
                        ui.horizontal(|ui| {
                            for &(name, preset) in row {
                                let swatch = Button::new("").fill(color(preset)).min_size(size);
                                if ui.add(swatch).on_hover_text(name).clicked() {
                                    *value = preset;
                                    ui.close();
                                }
                            }
                        });
                    }
                    ui.collapsing("Custom color", |ui| {
                        let mut custom = color(*value);
                        if color_picker::color_picker_color32(
                            ui,
                            &mut custom,
                            color_picker::Alpha::Opaque,
                        ) {
                            let [r, g, b, _] = custom.to_array().map(u32::from);
                            *value = r << 16 | g << 8 | b;
                        }
                    });
                });
        });
    }
    before != *colors
}
