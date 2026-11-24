//! LCD geometry and colors measured from a legitimate G1 firmware screen.
//! Keep painting and pointer regions in the same 320×240 coordinate system.
use ppl_graphics::{
    Grob,
    text::{draw_text, measure},
};

pub const HEADER_BOTTOM: i64 = 20;
pub const SOFTKEY_TOP: i64 = 220;
const CAPTION_WIDTH: i64 = 314;
const CAPTION_INK: u32 = 0xebebeb;
const HEADER_ROWS: [u32; 18] = [
    0x2562a8, 0x2560a6, 0x265ca3, 0x2658a0, 0x26549e, 0x26519e, 0x26509e, 0x2550a0, 0x2451a3,
    0x2352a7, 0x2254ac, 0x2156b0, 0x2058b5, 0x1f5aba, 0x1e5cbf, 0x1d5ec3, 0x1c5fc6, 0x1b60c9,
];

pub fn header(screen: &mut Grob, title: &str) {
    for (row, color) in HEADER_ROWS.into_iter().enumerate() {
        screen.line(0, row as i64, 319, row as i64, color);
    }
    screen.line(0, 18, 319, 18, 0);
    screen.line(0, 19, 319, 19, 0x898989);
    let width = measure(title, 14.);
    draw_text(
        screen,
        title,
        ((CAPTION_WIDTH - width) / 2).max(2),
        1,
        14.,
        CAPTION_INK,
        Some(316),
        None,
    );
}

pub fn softkeys(screen: &mut Grob, labels: &[&str; 6]) {
    softkeys_with_menus(screen, labels, &[]);
}

pub fn softkeys_with_menus(screen: &mut Grob, labels: &[&str; 6], menus: &[usize]) {
    let background = screen.get(0, 219).unwrap_or(0xffffff);
    let previous_row = (0..320)
        .map(|x| screen.get(x, 219).unwrap_or(background))
        .collect::<Vec<_>>();
    if ppl_graphics::softkeys::draw(screen, &[]).is_err() {
        screen.fill_rect(0, SOFTKEY_TOP, 319, 239, 0xffffff);
        draw_text(
            screen,
            "Calculator menu unavailable",
            3,
            222,
            12.,
            0xaa0000,
            Some(314),
            None,
        );
        return;
    }
    for (x, color) in previous_row.into_iter().enumerate() {
        screen.set(x as i64, 219, color);
    }
    for y in 220..240 {
        for x in 0..320 {
            if screen.get(x, y) == Some(0xffffff) {
                screen.set(x, y, background);
            }
        }
    }
    for &index in menus {
        if index < 6 {
            let start = index as i64 * 53;
            let end = if index == 5 { 319 } else { start + 51 };
            screen.line(start, 220, end, 220, 0);
            screen.line(start, 221, end, 221, 0);
            screen.line(start + 1, 221, end - 1, 221, 0x202020);
        }
    }
    for (i, label) in labels.iter().enumerate() {
        let start = i as i64 * 53;
        let end = if i == 5 { 319 } else { start + 51 };
        let arrow = label
            .strip_suffix(" ▸")
            .or_else(|| label.strip_suffix(" ▶"));
        let text = arrow.unwrap_or(label);
        let text_width = measure(text, 14.);
        let width = text_width + if arrow.is_some() { 8 } else { 0 };
        let x = start + ((end - start + 1 - width) / 2).max(2);
        draw_text(
            screen,
            text,
            x,
            222,
            14.,
            0xefefef,
            Some(end - start - 3),
            None,
        );
        if arrow.is_some() {
            for row in 0..7i64 {
                screen.line(
                    x + text_width + 4,
                    226 + row,
                    x + text_width + 4 + 3 - (row - 3).abs(),
                    226 + row,
                    0xefefef,
                );
            }
        }
    }
}

/// The last G1 button is two pixels wider than the other five.
pub fn softkey_index(x: i64) -> usize {
    (x.clamp(0, 319) / 53).min(5) as usize
}
