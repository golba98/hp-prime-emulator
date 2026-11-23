//! The inverting Plot cursor complements the already composited LCD pixels.
use eframe::{
    egui_glow::{CallbackFn, ShaderVersion},
    glow::{self, HasContext as _},
};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(super) struct CursorOverlay(Arc<Mutex<Option<Result<Invert, String>>>>);

impl CursorOverlay {
    pub(super) fn paint(&self, painter: &egui::Painter, arms: [egui::Rect; 4]) {
        let state = Arc::clone(&self.0);
        let viewport = arms[0].union(arms[1]).union(arms[2]).union(arms[3]);
        painter.add(egui::PaintCallback {
            rect: viewport,
            callback: Arc::new(CallbackFn::new(move |info, renderer| {
                let gl = renderer.gl();
                let mut state = state.lock().unwrap();
                let resource = state.get_or_insert_with(|| {
                    Invert::new(gl).inspect_err(|error| eprintln!("Plot cursor: {error}"))
                });
                if let Ok(resource) = resource {
                    resource.draw(gl, info, arms);
                }
            })),
        });
    }

    pub(super) fn release(&self, gl: &glow::Context) {
        if let Some(Ok(resource)) = self.0.lock().unwrap().take() {
            resource.release(gl);
        }
    }
}

struct Invert {
    program: glow::Program,
    triangle: glow::Buffer,
    array: Option<glow::VertexArray>,
}

impl Invert {
    fn new(gl: &glow::Context) -> Result<Self, String> {
        let version = ShaderVersion::get(gl);
        let new_shader = version.is_new_shader_interface();
        let vertex = format!(
            "{}{} vec2 p; void main() {{ gl_Position = vec4(p, 0.0, 1.0); }}",
            version.version_declaration(),
            if new_shader { "in" } else { "attribute" },
        );
        let fragment = format!(
            "{}{}{}void main() {{ {} = vec4(1.0); }}",
            version.version_declaration(),
            if version.is_embedded() {
                "precision mediump float;\n"
            } else {
                ""
            },
            if new_shader { "out vec4 color;\n" } else { "" },
            if new_shader { "color" } else { "gl_FragColor" },
        );
        // The egui render callback owns the current GL context. All retained
        // objects are destroyed in the IDE window's on_exit callback.
        unsafe {
            let program = gl.create_program()?;
            let link = (|| {
                for (kind, source) in [
                    (glow::VERTEX_SHADER, vertex),
                    (glow::FRAGMENT_SHADER, fragment),
                ] {
                    let shader = gl.create_shader(kind)?;
                    gl.shader_source(shader, &source);
                    gl.compile_shader(shader);
                    let result = if gl.get_shader_compile_status(shader) {
                        gl.attach_shader(program, shader);
                        Ok(())
                    } else {
                        Err(gl.get_shader_info_log(shader))
                    };
                    gl.delete_shader(shader);
                    result?;
                }
                gl.bind_attrib_location(program, 0, "p");
                gl.link_program(program);
                if !gl.get_program_link_status(program) {
                    return Err(gl.get_program_info_log(program));
                }
                let array = (gl.version().major >= 3)
                    .then(|| gl.create_vertex_array())
                    .transpose()?;
                let triangle = gl.create_buffer().inspect_err(|_| {
                    if let Some(array) = array {
                        gl.delete_vertex_array(array);
                    }
                })?;
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(triangle));
                let vertices = [-1.0_f32, -1.0, 3.0, -1.0, -1.0, 3.0];
                let bytes = vertices.map(f32::to_ne_bytes).concat();
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::STATIC_DRAW);
                Ok(Self {
                    program,
                    triangle,
                    array,
                })
            })();
            if link.is_err() {
                gl.delete_program(program);
            }
            link
        }
    }

    fn draw(&self, gl: &glow::Context, info: egui::PaintCallbackInfo, arms: [egui::Rect; 4]) {
        // Preserve destination alpha and compute white × (1 − destination).
        // The Glow painter restores its blend state, viewport and shader after
        // the callback. Its scissor remains active throughout these four draws.
        unsafe {
            gl.use_program(Some(self.program));
            gl.blend_func_separate(glow::ONE_MINUS_DST_COLOR, glow::ZERO, glow::ZERO, glow::ONE);
            if let Some(array) = self.array {
                gl.bind_vertex_array(Some(array));
            }
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.triangle));
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 0, 0);
            for arm in arms {
                let pixels = egui::epaint::ViewportInPixels::from_points(
                    &arm,
                    info.pixels_per_point,
                    info.screen_size_px,
                );
                gl.viewport(
                    pixels.left_px,
                    pixels.from_bottom_px,
                    pixels.width_px,
                    pixels.height_px,
                );
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
            }
            gl.disable_vertex_attrib_array(0);
        }
    }

    fn release(self, gl: &glow::Context) {
        // eframe provides the active renderer context during window cleanup.
        unsafe {
            if let Some(array) = self.array {
                gl.delete_vertex_array(array);
            }
            gl.delete_buffer(self.triangle);
            gl.delete_program(self.program);
        }
    }
}
