use super::{AUTO_COLOR, Block};
use ppl_graphics::{Grob, text::draw_text};

pub(super) const COLUMNS: usize = 4;
const ROWS: usize = 5;
pub(super) const CUSTOM: usize = COLUMNS * ROWS - 1;
const COLORS: [(&str, u32); CUSTOM] = [
    ("Auto", AUTO_COLOR),
    ("Black", 0x000000),
    ("White", 0xffffff),
    ("Silver", 0xc0c0c0),
    ("Gray", 0x808080),
    ("Dark gray", 0x555555),
    ("Red", 0xff0000),
    ("Orange", 0xffa500),
    ("Yellow", 0xffff00),
    ("Gold", 0xffd700),
    ("Lime", 0x00ff00),
    ("Cyan", 0x00ffff),
    ("Blue", 0x0000ff),
    ("Magenta", 0xff00ff),
    ("Maroon", 0x800000),
    ("Olive", 0x808000),
    ("Green", 0x008000),
    ("Teal", 0x008080),
    ("Navy", 0x000080),
];

const LEFT: i64 = 139;
const TOP: i64 = 67;
const CELL_WIDTH: i64 = 40;
const CELL_HEIGHT: i64 = 25;
const PLANE_X: i64 = 76;
const PLANE_Y: i64 = 57;
const PLANE_WIDTH: i64 = 169;
const PLANE_HEIGHT: i64 = 129;
const STEPS: i64 = 128;

pub(super) fn color(index: usize) -> Option<u32> {
    COLORS.get(index).map(|(_, value)| *value)
}

pub(super) fn item_at(x: i64, y: i64) -> Option<usize> {
    if !(LEFT..LEFT + CELL_WIDTH * COLUMNS as i64).contains(&x)
        || !(TOP..TOP + CELL_HEIGHT * ROWS as i64).contains(&y)
    {
        return None;
    }
    Some(((y - TOP) / CELL_HEIGHT) as usize * COLUMNS + ((x - LEFT) / CELL_WIDTH) as usize)
}

pub(super) fn moved(index: usize, dx: i64, dy: i64) -> usize {
    let col = (index as i64 % COLUMNS as i64 + dx).clamp(0, COLUMNS as i64 - 1);
    let row = (index as i64 / COLUMNS as i64 + dy).clamp(0, ROWS as i64 - 1);
    row as usize * COLUMNS + col as usize
}

pub(super) fn paint(g: &mut Grob, selected: usize, fill: bool) {
    g.rect(133, 43, 304, 211, 0x4b6070, 0xf7fafc);
    let title = if fill { "Fill color" } else { "Text color" };
    draw_text(g, title, LEFT, 47, 14., 0x263744, None, None);
    for index in 0..=CUSTOM {
        let x = LEFT + (index % COLUMNS) as i64 * CELL_WIDTH;
        let y = TOP + (index / COLUMNS) as i64 * CELL_HEIGHT;
        let border = if index == selected {
            0x2878b8
        } else {
            0x647582
        };
        g.rect(
            x + 2,
            y + 1,
            x + CELL_WIDTH - 3,
            y + CELL_HEIGHT - 3,
            border,
            0xffffff,
        );
        let x1 = x + 4;
        let y1 = y + 3;
        let x2 = x + CELL_WIDTH - 5;
        let y2 = y + CELL_HEIGHT - 5;
        match color(index) {
            Some(AUTO_COLOR) => {
                for py in y1..=y2 {
                    for px in x1..=x2 {
                        if ((px - x1) / 5 + (py - y1) / 5) % 2 == 0 {
                            g.set(px, py, 0xd8e1e8);
                        }
                    }
                }
            }
            Some(rgb) => g.fill_rect(x1, y1, x2, y2, rgb),
            None => {
                for px in x1..=x2 {
                    g.line(
                        px,
                        y1,
                        px,
                        y2,
                        sample((px - x1) * STEPS / (x2 - x1), STEPS / 2),
                    );
                }
            }
        }
    }
    let label = COLORS
        .get(selected)
        .map_or("Custom color", |(name, _)| name);
    draw_text(g, label, LEFT, 193, 14., 0x263744, None, None);
}

fn sample(x: i64, y: i64) -> u32 {
    let hue = (STEPS - x).rem_euclid(STEPS) * 6;
    let rising = ((hue % STEPS) * 255 + STEPS / 2) / STEPS;
    let falling = 255 - rising;
    let channels = match hue / STEPS {
        0 => [255, rising, 0],
        1 => [falling, 255, 0],
        2 => [0, 255, rising],
        3 => [0, falling, 255],
        4 => [rising, 0, 255],
        _ => [255, 0, falling],
    };
    channels.into_iter().fold(0, |rgb, channel| {
        let half = STEPS / 2;
        let channel = if y < half {
            channel + ((255 - channel) * (half - y) + half / 2) / half
        } else {
            (channel * (STEPS - y) + half / 2) / half
        };
        (rgb << 8) | channel as u32
    })
}

pub(super) struct CustomColor {
    pub block: Block,
    pub fill: bool,
    point: [i64; 2],
}

impl CustomColor {
    pub fn new(block: Block, fill: bool) -> Self {
        Self {
            block,
            fill,
            point: [STEPS / 2, STEPS / 2],
        }
    }
    pub fn pick(&mut self, x: i64, y: i64) {
        if (PLANE_X..PLANE_X + PLANE_WIDTH).contains(&x)
            && (PLANE_Y..PLANE_Y + PLANE_HEIGHT).contains(&y)
        {
            self.point = [
                (x - PLANE_X) * STEPS / (PLANE_WIDTH - 1),
                (y - PLANE_Y) * STEPS / (PLANE_HEIGHT - 1),
            ];
        }
    }
    pub fn color(&self) -> u32 {
        sample(self.point[0], self.point[1])
    }
    pub fn paint(&self, g: &mut Grob) {
        g.rect(63, 34, 257, 215, 0x4b6070, 0xf7fafc);
        g.fill_rect(64, 35, 256, 53, 0x3b5266);
        draw_text(g, "Custom color", 72, 37, 14., 0xffffff, None, None);
        for y in 0..PLANE_HEIGHT {
            for x in 0..PLANE_WIDTH {
                g.set(
                    PLANE_X + x,
                    PLANE_Y + y,
                    sample(
                        x * STEPS / (PLANE_WIDTH - 1),
                        y * STEPS / (PLANE_HEIGHT - 1),
                    ),
                );
            }
        }
        let x = PLANE_X + self.point[0] * (PLANE_WIDTH - 1) / STEPS;
        let y = PLANE_Y + self.point[1] * (PLANE_HEIGHT - 1) / STEPS;
        g.line(x - 3, y, x + 3, y, 0x111111);
        g.line(x, y - 3, x, y + 3, 0x111111);
        g.rect(76, 192, 138, 208, 0x647582, self.color());
        draw_text(
            g,
            &format!("#{:06X}", self.color()),
            146,
            194,
            14.,
            0x263744,
            None,
            None,
        );
    }
}
