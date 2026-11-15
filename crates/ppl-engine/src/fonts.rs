//! Font admission shared by headless Native execution and desktop consumers.
//! Unconfigured installations retain the bundled font. Selecting an installed
//! resource is explicit: unreadable or invalid selected data is an error.
use crate::firmware_options::AssetPaths;
use ppl_graphics::text::prime::PrimeFonts;
use std::{path::PathBuf, sync::Arc};

pub fn resource(paths: &AssetPaths) -> Option<PathBuf> {
    paths
        .factory_a
        .as_ref()
        .map(|p| p.join("programs/misc/armfir.dat"))
        .or_else(|| {
            paths
                .init_data
                .as_ref()
                .map(|p| p.join("A/programs/misc/armfir.dat"))
        })
}
pub fn configured() -> Result<Option<Arc<PrimeFonts>>, String> {
    let path = match std::env::var_os("PPL_PRIME_FONT_RESOURCE").filter(|p| !p.is_empty()) {
        Some(path) => Some(PathBuf::from(path)),
        None => resource(&AssetPaths::default().resolve()?),
    };
    path.map(|path| {
        PrimeFonts::load_resource(&path).map_err(|error| format!("{}: {error}", path.display()))
    })
    .transpose()
}
