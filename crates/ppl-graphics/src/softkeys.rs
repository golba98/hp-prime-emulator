//! Calculator menu composition. Installed textures stay in the user's resource
//! archive; unconfigured installations retain the bundled font and an authored
//! gradient. Invalid selected resources are errors, never substituted assets.
use crate::{BLACK, Grob, WHITE, text};
use text::prime::FontError;

pub fn draw(screen: &mut Grob, labels: &[&str]) -> Result<(), FontError> {
    let labels = labels
        .iter()
        .map(|s| s.encode_utf16().collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let borrowed = labels.iter().map(Vec::as_slice).collect::<Vec<_>>();
    draw_units(screen, &borrowed)
}

pub fn draw_units(screen: &mut Grob, labels: &[&[u16]]) -> Result<(), FontError> {
    let texture = text::prime::active_menu_texture(false).transpose()?;
    screen.fill_rect(0, 219, 319, 239, WHITE);
    for index in 0..6 {
        let start = index * 53;
        let width = if index == 5 { 55 } else { 52 };
        for row in 0..20 {
            for x in 0..width {
                let color = match row {
                    0 | 19 if (3..width - 3).contains(&x) => BLACK,
                    0 | 19 => WHITE,
                    1 | 18 if x == 0 || x == width - 1 => WHITE,
                    1 | 18 if x <= 2 || x >= width - 3 => BLACK,
                    _ if x == 0 || x == width - 1 => BLACK,
                    _ => texture.as_ref().map_or_else(
                        || {
                            let gray = 32 + (47 * (row - 1) + 8) / 17;
                            gray as u32 * 0x010101
                        },
                        |g| g.get(x, row - 1).unwrap(),
                    ),
                };
                screen.set(start + x, 220 + row, color);
            }
        }
        let Some(label) = labels.get(index as usize) else {
            continue;
        };
        // The original menu clips ink at the button rectangle, including a
        // partial final glyph. TEXTOUT's width argument has different semantics.
        let label = label
            .iter()
            .copied()
            .filter(|u| !matches!(u, 0 | 10 | 13))
            .collect::<Vec<_>>();
        let mut area = screen.sub(start + 1, 221, start + width - 1, 239);
        let x = ((width - 2 - text::measure_utf16_units(&label, 14.0)) / 2).max(0);
        text::draw_utf16_units(&mut area, &label, x, 1, 14.0, WHITE, None, None);
        for y in 0..18 {
            for x in 0..width - 2 {
                screen.set(start + 1 + x, 221 + y, area.get(x, y).unwrap());
            }
        }
    }
    Ok(())
}
