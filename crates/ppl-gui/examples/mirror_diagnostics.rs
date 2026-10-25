//! Read-only, bounded capture/GUI measurements; no keys or program operations.
use std::path::PathBuf;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = ppl_gui::MirrorConfig::default();
    let mut frames = 20;
    let mut fixture = None;
    let mut size = (853, 640);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = args.next().ok_or("each option requires a value")?;
        match arg.as_str() {
            "--quality" => config.quality = value.parse()?,
            "--filter" => config.filter = value.parse()?,
            "--scale" => config.scale = value.parse()?,
            "--frames" => frames = value.parse()?,
            "--fixture" => fixture = Some(PathBuf::from(value)),
            "--size" => {
                let (w, h) = value.split_once('x').ok_or("size must be WIDTHxHEIGHT")?;
                size = (w.parse()?, h.parse()?);
            }
            "--gui-seconds" => config.exit_after = Some(Duration::from_secs(value.parse()?)),
            "--out-dir" => config.diagnostics = Some(PathBuf::from(value)),
            _ => return Err(format!("unknown option {arg}").into()),
        }
    }
    let dir = config
        .diagnostics
        .as_deref()
        .ok_or("--out-dir is required (use a new directory)")?;
    if !(1..=4).contains(&config.scale) {
        return Err("scale must be 1–4".into());
    }
    if let Some(path) = fixture {
        let image = ppl_graphics::Grob::from_prime_screenshot(&std::fs::read(path)?)?;
        if config.exit_after.is_some() {
            return ppl_gui::mirror::run_mirror(config, StaticSource(image));
        }
        if size.0 == 0 || size.1 == 0 || size.0 > 8192 || size.1 > 8192 {
            return Err("comparison size must be 1–8192 in each dimension".into());
        }
        std::fs::create_dir_all(dir)?;
        image.save_png(&dir.join("decoded.png"))?;
        for (name, viewport) in [
            (
                "before.png",
                ppl_gui::Viewport::calculate_aspect_fit(size.0, size.1),
            ),
            (
                "after.png",
                ppl_gui::Viewport::calculate_integer(size.0, size.1),
            ),
        ] {
            let mut output = ppl_graphics::Grob::new(size.0 as usize, size.1 as usize, 0);
            ppl_gui::blit_filtered(
                image.pixels(),
                image.width(),
                image.height(),
                output.pixels_mut(),
                size.0 as usize,
                size.1 as usize,
                &viewport,
                ppl_gui::mirror::DEFAULT_BG_COLOR,
                config.filter,
            );
            output.save_png(&dir.join(name))?;
        }
        return Ok(());
    }
    let prime = ppl_device::Prime::open_first()?;
    if config.exit_after.is_some() {
        ppl_gui::run_mirror_usb(config, prime)?;
    } else {
        let metrics = ppl_gui::mirror::capture_diagnostics(prime, config.quality, frames, dir)?;
        println!(
            "{} captures; {:.3} capture FPS; {} errors; headless (no displayed FPS)",
            metrics.total_captured, metrics.rolling_fps, metrics.errors
        );
    }
    Ok(())
}

struct StaticSource(ppl_graphics::Grob);
impl ppl_gui::MirrorSource for StaticSource {
    fn capture_frame(&mut self) -> Result<ppl_graphics::Grob, ppl_device::DeviceError> {
        Ok(self.0.clone())
    }
    fn device_name(&self) -> &str {
        "offline fixture"
    }
    fn device_serial(&self) -> Option<&str> {
        None
    }
    fn try_reconnect(&mut self) -> Result<bool, ppl_device::DeviceError> {
        Ok(false)
    }
}
