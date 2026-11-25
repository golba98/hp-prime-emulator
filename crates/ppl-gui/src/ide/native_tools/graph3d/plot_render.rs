//! Rasterize the evaluated surface triangles with a depth value for each pixel.
use super::surface_colors::rgb;
use crate::ide::native_graph3d::{Book, CameraModel, Mesh, SurfaceColors};
use egui::{Color32, ColorImage, Painter, Pos2, Rect, Stroke, TextureHandle, TextureOptions, pos2};

const WIDTH: usize = 316;
type Point = [f64; 3];

#[derive(Default)]
pub(super) struct SurfaceLayer {
    inputs: Option<Inputs>,
    texture: Option<TextureHandle>,
}

#[derive(PartialEq)]
struct Inputs {
    height: usize,
    rotation: [f64; 2],
    camera: CameraModel,
    colors: SurfaceColors,
}

impl SurfaceLayer {
    pub(super) fn invalidate(&mut self) {
        self.inputs = None;
    }

    pub(super) fn paint(
        &mut self,
        painter: &Painter,
        area: Rect,
        height: usize,
        book: &Book,
        mesh: Option<&Mesh>,
        point: Option<Point>,
    ) {
        painter.rect_filled(area, 0., Color32::from_rgb(16, 21, 29));
        if book.validate_viewport().is_err() {
            return;
        }
        let projection = Projection::new(height, book.rotation, book.camera);
        let screen = |point: Point| {
            let normalized = std::array::from_fn(|i| normalize(point[i], book.bounds[i]));
            let [x, y, _] = projection.project(normalized);
            area.min
                + egui::vec2(
                    x as f32 * area.width() / WIDTH as f32,
                    y as f32 * area.height() / height as f32,
                )
        };
        paint_axes(painter, book, screen);
        let Some(mesh) = mesh else { return };
        let inputs = Inputs {
            height,
            rotation: book.rotation,
            camera: book.camera,
            colors: book.colors,
        };
        if self.inputs.as_ref() != Some(&inputs) {
            let mut raster = Raster::new(height);
            raster.fill(&projection, book, mesh);
            let image = ColorImage::new([WIDTH, height], raster.colors);
            if let Some(texture) = &mut self.texture {
                texture.set(image, TextureOptions::NEAREST);
            } else {
                self.texture = Some(painter.ctx().load_texture(
                    "graph_3d_surfaces",
                    image,
                    TextureOptions::NEAREST,
                ));
            }
            self.inputs = Some(inputs);
        }
        if let Some(texture) = &self.texture {
            painter.image(
                texture.id(),
                area,
                Rect::from_min_max(Pos2::ZERO, pos2(1., 1.)),
                Color32::WHITE,
            );
        }
        painter.text(
            pos2(area.left() + 5., area.top() + 5.),
            egui::Align2::LEFT_TOP,
            "X / Y / Z",
            egui::FontId::proportional(11.),
            Color32::LIGHT_GRAY,
        );
        if let Some(point) = point
            && point
                .iter()
                .zip(book.bounds)
                .all(|(coordinate, [low, high])| (low..=high).contains(coordinate))
        {
            let radius = (area.width() * 0.008).clamp(3., 8.);
            painter.circle_stroke(screen(point), radius, Stroke::new(1.5_f32, Color32::WHITE));
        }
    }
}

fn paint_axes(painter: &Painter, book: &Book, screen: impl Fn(Point) -> Pos2) {
    let axis = Stroke::new(1.0_f32, Color32::from_rgb(100, 114, 132));
    for axis_index in 0..3 {
        let others = [(axis_index + 1) % 3, (axis_index + 2) % 3];
        for a in book.bounds[others[0]] {
            for b in book.bounds[others[1]] {
                let mut low = [0.; 3];
                low[others[0]] = a;
                low[others[1]] = b;
                low[axis_index] = book.bounds[axis_index][0];
                let mut high = low;
                high[axis_index] = book.bounds[axis_index][1];
                painter.line_segment([screen(low), screen(high)], axis);
            }
        }
        let [low, high] = book.bounds[axis_index];
        let tick = book.ticks[axis_index];
        for index in 1..32 {
            let coordinate = low + f64::from(index) * tick;
            if coordinate >= high {
                break;
            }
            let mut start = book.bounds.map(|bounds| bounds[0]);
            start[axis_index] = coordinate;
            let mut end = start;
            let [from, to] = book.bounds[others[0]];
            end[others[0]] = from * 0.965 + to * 0.035;
            painter.line_segment([screen(start), screen(end)], axis);
        }
    }
}

fn normalize(value: f64, [low, high]: [f64; 2]) -> f64 {
    2. * ((value - low) / (high - low)) - 1.
}

struct Projection {
    center: [f64; 2],
    scale: f64,
    yaw: (f64, f64),
    pitch: (f64, f64),
    camera: CameraModel,
}

impl Projection {
    fn new(height: usize, [yaw, pitch]: [f64; 2], camera: CameraModel) -> Self {
        // On a fresh plot the flat plane covers 227 LCD columns and is
        // centred at (159,109). The native controls occupy the top 44 rows.
        let (center, scale) = match camera {
            CameraModel::G1 => (
                [WIDTH as f64 * 0.5 + 1., height as f64 * 0.5 - 22.],
                height as f64 * 0.47,
            ),
            CameraModel::Legacy => (
                [WIDTH as f64 * 0.5, height as f64 * 0.5],
                WIDTH.min(height) as f64 * 0.25,
            ),
        };
        Self {
            center,
            scale,
            yaw: yaw.to_radians().sin_cos(),
            pitch: pitch.to_radians().sin_cos(),
            camera,
        }
    }

    fn orient(&self, [x, y, z]: Point) -> Point {
        let (sin_yaw, cos_yaw) = self.yaw;
        let (sin_pitch, cos_pitch) = self.pitch;
        let depth = sin_yaw * x + cos_yaw * y;
        let u = cos_yaw * x - sin_yaw * y;
        match self.camera {
            CameraModel::Legacy => [
                u,
                cos_pitch * z - sin_pitch * depth,
                cos_pitch * depth + sin_pitch * z,
            ],
            CameraModel::G1 => {
                // The default G1 horizontal edge has a small screen roll.
                let (sin_roll, cos_roll) = (-3_f64).to_radians().sin_cos();
                let v = cos_pitch * z + sin_pitch * depth;
                [
                    cos_roll * u + sin_roll * v,
                    cos_roll * v - sin_roll * u,
                    sin_pitch * z - cos_pitch * depth,
                ]
            }
        }
    }

    fn project(&self, point: Point) -> Point {
        let [u, v, depth] = self.orient(point);
        [
            self.center[0] + self.scale * u,
            self.center[1] - self.scale * v,
            depth,
        ]
    }

    fn shade(&self, [a, b, c]: [Point; 3], colors: [u32; 2]) -> [Color32; 2] {
        if self.camera == CameraModel::Legacy {
            return colors.map(|color| face_color(color, 215));
        }
        let [ux, uy, uz] = std::array::from_fn(|axis| b[axis] - a[axis]);
        let [vx, vy, vz] = std::array::from_fn(|axis| c[axis] - a[axis]);
        let normal = [uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx];
        let length = normal[0].hypot(normal[1]).hypot(normal[2]);
        let [right, up, depth] =
            self.orient(normal.map(|axis| axis / length.max(f64::MIN_POSITIVE)));
        // Planar controls at the default and rotated G1 views follow
        // separate camera-space light directions for the two face colors.
        let front = light_level(0.99 * (-right + up + depth) / 3_f64.sqrt());
        let back = light_level(0.194 * up);
        // Positive screen Y points down, so the G1 front has negative area.
        [face_color(colors[1], back), face_color(colors[0], front)]
    }
}

struct Raster {
    height: usize,
    colors: Vec<Color32>,
    depths: Vec<f64>,
}

impl Raster {
    fn new(height: usize) -> Self {
        Self {
            height,
            colors: vec![Color32::TRANSPARENT; WIDTH * height],
            depths: vec![f64::NEG_INFINITY; WIDTH * height],
        }
    }

    fn fill(&mut self, projection: &Projection, book: &Book, mesh: &Mesh) {
        let n = mesh.resolution;
        let axis = |index| 2. * (index as f64 / (n - 1) as f64) - 1.;
        for grid in &mesh.grids {
            let colors = book.colors[grid.index];
            let point = |x: usize, y: usize| grid.heights[y * n + x].map(|z| [axis(x), axis(y), z]);
            for y in 0..n - 1 {
                for x in 0..n - 1 {
                    let a = point(x, y);
                    let b = point(x + 1, y);
                    let c = point(x + 1, y + 1);
                    let d = point(x, y + 1);
                    for [a, b, c] in [[a, b, c], [a, c, d]] {
                        if let [Some(a), Some(b), Some(c)] = [a, b, c] {
                            self.clip_triangle(projection, book.bounds[2], [a, b, c], colors);
                        }
                    }
                }
            }
        }
    }

    fn clip_triangle(
        &mut self,
        projection: &Projection,
        bounds: [f64; 2],
        triangle: [Point; 3],
        colors: [u32; 2],
    ) {
        let [lo, hi] = bounds;
        if triangle.iter().all(|point| point[2] < lo) || triangle.iter().all(|point| point[2] > hi)
        {
            return;
        }
        let project = |raster: &mut Self, triangle: [Point; 3]| {
            let triangle = triangle.map(|[x, y, z]| [x, y, normalize(z, bounds)]);
            let faces = projection.shade(triangle, colors);
            raster.triangle(triangle.map(|point| projection.project(point)), faces);
        };
        if triangle.iter().all(|point| (lo..=hi).contains(&point[2])) {
            project(self, triangle);
            return;
        }
        let mut bottom = [[0.; 3]; 4];
        let mut volume = [[0.; 3]; 5];
        let count = clip_z(&triangle, &mut bottom, lo, |z| z >= lo);
        let count = clip_z(&bottom[..count], &mut volume, hi, |z| z <= hi);
        for index in 1..count.saturating_sub(1) {
            let triangle = [volume[0], volume[index], volume[index + 1]];
            project(self, triangle);
        }
    }

    fn triangle(&mut self, [a, b, c]: [Point; 3], face_colors: [Color32; 2]) {
        let area = edge(a, b, c);
        if area.abs() < 1e-9 {
            return;
        }
        let color = face_colors[usize::from(area < 0.)];
        let limits = |i: usize, end: usize| {
            let low = a[i].min(b[i]).min(c[i]).floor() as usize;
            let high = (a[i].max(b[i]).max(c[i]).ceil() as usize).min(end);
            low..high
        };
        for y in limits(1, self.height) {
            for x in limits(0, WIDTH) {
                let pixel = [x as f64 + 0.5, y as f64 + 0.5, 0.];
                let w = [edge(b, c, pixel), edge(c, a, pixel), edge(a, b, pixel)]
                    .map(|weight| weight / area);
                if w.iter().all(|&weight| weight >= -1e-12) {
                    let depth = w[0] * a[2] + w[1] * b[2] + w[2] * c[2];
                    let i = y * WIDTH + x;
                    if depth > self.depths[i] {
                        self.depths[i] = depth;
                        self.colors[i] = color;
                    }
                }
            }
        }
    }
}

fn edge(from: Point, to: Point, point: Point) -> f64 {
    (to[0] - from[0]) * (point[1] - from[1]) - (to[1] - from[1]) * (point[0] - from[0])
}

fn light_level(direction: f64) -> u8 {
    (127.5 * (1. + direction)).clamp(0., 255.) as u8
}

fn face_color(color: u32, shade: u8) -> Color32 {
    let [r, g, b] =
        rgb(color).map(|value| ((u16::from(value) * u16::from(shade) + 127) / 255) as u8);
    Color32::from_rgb(r, g, b)
}

fn clip_z(
    input: &[Point],
    output: &mut [Point],
    plane: f64,
    inside: impl Fn(f64) -> bool,
) -> usize {
    let mut length = 0;
    for (a, b) in input.iter().zip(input.iter().cycle().skip(1)) {
        if inside(a[2]) != inside(b[2]) {
            let delta = b[2] - a[2];
            let t = if delta.is_finite() {
                (plane - a[2]) / delta
            } else {
                (plane * 0.5 - a[2] * 0.5) / (b[2] * 0.5 - a[2] * 0.5)
            };
            output[length] = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, plane];
            length += 1;
        }
        if inside(b[2]) {
            output[length] = *b;
            length += 1;
        }
    }
    length
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ide::native_graph3d::SurfaceGrid;

    #[test]
    fn facing_and_depth_choose_the_visible_surface_color() {
        let book = Book::default();
        for (pitch, expected) in [(35., 0xff0000), (-35., 0x00ffff)] {
            let projection = Projection::new(174, [0., pitch], CameraModel::Legacy);
            let mut raster = Raster::new(174);
            let mesh = Mesh {
                resolution: 2,
                grids: vec![
                    SurfaceGrid {
                        index: 0,
                        heights: vec![Some(0.); 4],
                    },
                    SurfaceGrid {
                        index: 1,
                        heights: vec![Some(1.); 4],
                    },
                ],
            };
            raster.fill(&projection, &book, &mesh);
            assert_eq!(
                raster.colors[87 * WIDTH + WIDTH / 2],
                face_color(expected, 215)
            );
        }
    }

    #[test]
    fn fresh_camera_matches_the_measured_plane_faces_and_lighting() {
        let book = Book::default();
        let projection = Projection::new(174, book.rotation, book.camera);
        for (slope_x, slope_y, front, measured) in [
            (0., 0., true, 215u8),
            (0.5, 0., false, 149),
            (-0.5, 0., true, 195),
            (0., 0.5, true, 245),
            (0., -0.5, false, 151),
            (0.25, 0., true, 218),
            (0., -0.25, false, 152),
            (0.5, -0.5, false, 149),
            (0.5, 0.25, true, 234),
            (-0.5, -0.25, true, 174),
        ] {
            let mut raster = Raster::new(174);
            let heights = [(-4., -4.), (4., -4.), (-4., 4.), (4., 4.)]
                .map(|(x, y)| Some(slope_x * x + slope_y * y));
            let mesh = Mesh {
                resolution: 2,
                grids: vec![SurfaceGrid {
                    index: 0,
                    heights: heights.to_vec(),
                }],
            };
            raster.fill(&projection, &book, &mesh);
            let color = raster.colors
                [projection.center[1] as usize * WIDTH + projection.center[0] as usize];
            assert_eq!(color.r(), 0);
            assert!(color.b().abs_diff(measured) <= 1, "{slope_x} {slope_y}");
            assert_eq!(color.g(), if front { 0 } else { color.b() });
        }
    }

    #[test]
    fn old_persisted_views_keep_their_camera_and_color_convention() {
        let saved = serde_json::json!({"rotation": [315., -35.]});
        let book: Book = serde_json::from_value(saved).unwrap();
        assert_eq!(book.camera, CameraModel::Legacy);
        assert_eq!(book.rotation, [315., -35.]);
        let saved = serde_json::to_value(&book).unwrap();
        assert_eq!(serde_json::from_value::<Book>(saved).unwrap(), book);
        let empty: Book = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(empty.camera, CameraModel::Legacy);
        assert_eq!(empty.rotation, [-45., 25.]);
        assert_eq!(Book::default().camera, CameraModel::G1);
    }

    #[test]
    fn tilted_planes_match_independent_virtual_g1_face_and_intensity_controls() {
        let book = Book::default();
        let p = Projection::new(174, book.rotation, book.camera);
        for (x, y, expected) in [
            (0., 0., [0, 0, 215]),
            (0.5, 0., [0, 149, 149]),
            (-1., 0., [0, 0, 172]),
            (0., 0.5, [0, 0, 245]),
            (0., -0.5, [0, 151, 151]),
            (0.2, 0.7, [0, 0, 252]),
            (0.25, 0.5, [0, 0, 247]),
        ] {
            let mut raster = Raster::new(174);
            let mesh = Mesh {
                resolution: 2,
                grids: vec![SurfaceGrid {
                    index: 0,
                    heights: [-x - y, x - y, -x + y, x + y]
                        .map(|z| Some(z * 4.))
                        .to_vec(),
                }],
            };
            raster.fill(&p, &book, &mesh);
            let pixel = raster.colors[p.center[1] as usize * WIDTH + p.center[0] as usize];
            let actual = [pixel.r(), pixel.g(), pixel.b()];
            for (a, b) in actual.into_iter().zip(expected) {
                assert!(
                    u8::abs_diff(a, b) <= 1,
                    "{x}, {y}: {actual:?} expected {expected:?}"
                );
            }
        }
    }
    #[test]
    fn triangles_clip_at_both_height_bounds() {
        let projection = Projection::new(174, [0., 30.], CameraModel::Legacy);
        let mut raster = Raster::new(174);
        let colors = [0x0000ff, 0xff0000];
        let triangle = [[-1., -1., -2.], [1., -1., 2.], [1., 1., 2.]];
        raster.clip_triangle(&projection, [-0.25, 0.25], triangle, colors);
        let mut expected = Raster::new(174);
        let [a, b, c, d] = [
            [-0.125, -1., -1.],
            [0.125, -1., 1.],
            [0.125, 0.125, 1.],
            [-0.125, -0.125, -1.],
        ]
        .map(|point| projection.project(point));
        expected.triangle([a, b, c], colors.map(|color| face_color(color, 215)));
        expected.triangle([a, c, d], colors.map(|color| face_color(color, 215)));
        assert!(
            expected
                .colors
                .iter()
                .any(|&color| color != Color32::TRANSPARENT)
        );
        assert_eq!(raster.colors, expected.colors);
    }

    #[test]
    fn old_profiles_inherit_colors_and_only_custom_choices_need_storage() {
        let original = serde_json::to_value(Book::default()).unwrap();
        assert!(original.get("colors").is_none());
        let mut restored: Book = serde_json::from_value(original).unwrap();
        assert_eq!(restored.colors, Book::default().colors);
        restored.colors[0] = [0x800080, 0xff0000];
        let restored: Book =
            serde_json::from_value(serde_json::to_value(restored).unwrap()).unwrap();
        assert_eq!(restored.colors[0], [0x800080, 0xff0000]);
        assert_eq!(restored.colors[1..], Book::default().colors[1..]);
    }
}
