//! Compiled, cancellable surface evaluation for Graph 3D.
use super::native_apps::{Evaluator, decimal_sample, numeric};
use ppl_math::{MathError, Real};
use ppl_vm::{AppNumericSettings, SessionSnapshot, Value};
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub const NAME: &str = "Graph 3D";
pub const FUNCTIONS: [&str; 10] = [
    "FZ1", "FZ2", "FZ3", "FZ4", "FZ5", "FZ6", "FZ7", "FZ8", "FZ9", "FZ0",
];
pub(super) fn plot_context(snapshot: &SessionSnapshot) -> SessionSnapshot {
    let mut context = snapshot.clone();
    context.active_app = NAME.into();
    // Legacy flat plot aliases do not seed 3-D formulas. The VM resolves
    // measured axis values from the active app's typed Plot Setup.
    context
        .globals
        .retain(|name, _| ppl_vm::AppPlotSettings::canonical_name(name).is_none());
    context
}

const RESOLUTION: usize = 25;
pub const NUMERIC_ROWS: usize = 1000;
pub type Definitions = [Definition; FUNCTIONS.len()];
pub type SurfaceColors = [[u32; 2]; FUNCTIONS.len()];

const DEFAULT_COLORS: SurfaceColors = [
    [0x0000ff, 0x00ffff],
    [0xff0000, 0xff00ff],
    [0x008000, 0xffff00],
    [0xffa500, 0x008080],
    [0x800080, 0x808000],
    [0xff00ff, 0x00ffff],
    [0x00ff00, 0x0000ff],
    [0x000080, 0x800080],
    [0x800000, 0xee00ee],
    [0xffd700, 0x0099d7],
];

fn default_colors(colors: &SurfaceColors) -> bool {
    *colors == DEFAULT_COLORS
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Definition {
    pub source: String,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub literal: Option<(String, serde_json::Value)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Book {
    pub definitions: Definitions,
    pub bounds: [[f64; 2]; 3],
    pub ticks: [f64; 3],
    pub rotation: [f64; 2],
    #[serde(skip_serializing_if = "default_colors")]
    pub colors: SurfaceColors,
}

impl Default for Book {
    fn default() -> Self {
        Self {
            definitions: Default::default(),
            bounds: [[-4., 4.]; 3],
            ticks: [1.; 3],
            rotation: [-45., 25.],
            colors: DEFAULT_COLORS,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SurfaceGrid {
    pub index: usize,
    pub heights: Vec<Option<f64>>,
}

#[derive(Clone, Copy)]
pub(super) struct SurfacePoint {
    pub index: usize,
    pub coordinates: [Real; 2],
    pub height: Option<Real>,
}

impl SurfacePoint {
    pub(super) fn projectable(self) -> Option<[f64; 3]> {
        let [x, y] = self.coordinates.map(Real::to_f64);
        let z = self.height?.to_f64();
        [x, y, z]
            .into_iter()
            .all(f64::is_finite)
            .then_some([x, y, z])
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Mesh {
    pub resolution: usize,
    pub grids: Vec<SurfaceGrid>,
}

impl Book {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_plot_fields(&self.bounds, &self.ticks)?;
        if self
            .rotation
            .iter()
            .any(|angle| !angle.is_finite() || angle.abs() > 360.)
        {
            return Err("Invalid Graph 3D rotation".into());
        }
        if self.colors.iter().flatten().any(|&color| color > 0xffffff) {
            return Err("Invalid Graph 3D surface color".into());
        }
        for definition in &self.definitions {
            if definition.source.len() > 65_536 {
                return Err("Surface definition exceeds 64 KiB".into());
            }
            if let Some((source, value)) = &definition.literal
                && (source != &definition.source
                    || !matches!(
                        ppl_vm::session::decode_value(value),
                        Ok(ppl_vm::PortableValue::Real(_) | ppl_vm::PortableValue::Integer(_))
                    ))
            {
                return Err("Invalid Graph 3D scalar definition".into());
            }
        }
        Ok(())
    }

    pub fn range_error(&self) -> Option<String> {
        ["X", "Y", "Z"]
            .into_iter()
            .zip(self.bounds)
            .find(|(_, [low, high])| low >= high)
            .map(|(axis, _)| format!("{axis} range minimum must be less than {axis} range maximum"))
    }

    pub fn validate_viewport(&self) -> Result<(), String> {
        self.validate()?;
        self.range_error().map_or(Ok(()), Err)
    }

    pub fn apply_plot_settings(
        &mut self,
        settings: &ppl_vm::AppPlotSettings,
    ) -> Result<(), String> {
        let fields = ppl_vm::AppPlotSettings::fields(NAME);
        let values = fields
            .iter()
            .map(|field| {
                settings
                    .get(field)
                    .and_then(|value| value.as_real())
                    .map(|value| value.to_f64())
                    .ok_or("Missing Graph 3D Plot Setup value")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let bounds = [
            [values[0], values[1]],
            [values[2], values[3]],
            [values[4], values[5]],
        ];
        let ticks = [values[6], values[7], values[8]];
        validate_plot_fields(&bounds, &ticks)?;
        self.bounds = bounds;
        self.ticks = ticks;
        Ok(())
    }

    pub fn plot_settings(&self) -> Result<ppl_vm::AppPlotSettings, String> {
        let mut settings = ppl_vm::AppPlotSettings::for_app(NAME).expect("Graph 3D Plot Setup");
        let numbers = self.bounds.into_iter().flatten().chain(self.ticks);
        for (&field, number) in ppl_vm::AppPlotSettings::fields(NAME).iter().zip(numbers) {
            settings
                .set(
                    field,
                    &Value::Real(Real::from_f64(number).map_err(|error| error.to_string())?),
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(settings)
    }

    fn compile(
        &self,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<Vec<(usize, Evaluator)>, String> {
        self.validate()?;
        let sources: Vec<_> = self
            .definitions
            .iter()
            .enumerate()
            .filter(|(_, definition)| definition.enabled && !definition.source.trim().is_empty())
            .map(|(index, definition)| (index, definition.source.as_str()))
            .collect();
        let expressions: Vec<_> = sources.iter().map(|(_, source)| *source).collect();
        let memory = plot_context(snapshot);
        let context = super::native_graph_context::resolve(
            &memory,
            &expressions,
            &["X", "Y", "x", "y"],
            &cancel,
        )?;
        sources
            .into_iter()
            .map(|(index, source)| {
                stopped(&cancel)?;
                Ok((
                    index,
                    Evaluator::implicit(source, &context, cancel.clone())?,
                ))
            })
            .collect()
    }

    pub(super) fn mesh(
        &self,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
        selection: Option<SurfacePoint>,
    ) -> Result<(Mesh, Option<SurfacePoint>), String> {
        self.validate_viewport()?;
        let definitions = self.compile(snapshot, cancel.clone())?;
        let selection =
            selection.filter(|point| definitions.iter().any(|(index, _)| *index == point.index));
        let index = selection
            .map(|point| point.index)
            .or_else(|| definitions.first().map(|entry| entry.0));
        let mut grids = Vec::new();
        let mut point = None;
        for (surface, mut evaluator) in definitions {
            let mut heights = Vec::with_capacity(RESOLUTION * RESOLUTION);
            for row in 0..RESOLUTION {
                let y = position(self.bounds[1], row, RESOLUTION);
                for column in 0..RESOLUTION {
                    stopped(&cancel)?;
                    let x = position(self.bounds[0], column, RESOLUTION);
                    heights.push(numeric(evaluator.at_xy_value(x, y))?);
                }
            }
            if Some(surface) == index {
                stopped(&cancel)?;
                let coordinates = selection.map_or([Real::ZERO; 2], |point| point.coordinates);
                let initial =
                    surface_point(&mut evaluator, surface, coordinates).unwrap_or(SurfacePoint {
                        index: surface,
                        coordinates,
                        height: None,
                    });
                point = Some(initial);
            }
            grids.push(SurfaceGrid {
                index: surface,
                heights,
            });
        }
        Ok((
            Mesh {
                resolution: RESOLUTION,
                grids,
            },
            point,
        ))
    }

    pub(super) fn trace_point(
        &self,
        snapshot: &SessionSnapshot,
        index: usize,
        coordinates: [Real; 2],
        cancel: Arc<AtomicBool>,
    ) -> Result<SurfacePoint, String> {
        self.validate_viewport()?;
        let mut evaluator = self
            .compile(snapshot, cancel)?
            .into_iter()
            .find(|(number, _)| *number == index)
            .ok_or("The selected surface is unavailable")?
            .1;
        surface_point(&mut evaluator, index, coordinates)
    }

    pub(super) fn sort_numeric_points(
        &self,
        settings: &mut AppNumericSettings,
        column: usize,
        ascending: bool,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<(), String> {
        let axis = settings.graph3d.as_mut().expect("Graph 3D Numeric axes");
        let mut surface = if column < 2 {
            None
        } else {
            Some(
                self.compile(snapshot, cancel.clone())?
                    .into_iter()
                    .nth(column - 2)
                    .ok_or("The selected surface is unavailable")?
                    .1,
            )
        };
        let mut records = Vec::with_capacity(axis.points.len());
        for point in &axis.points {
            stopped(&cancel)?;
            let [x, y] = *point;
            let value = match &mut surface {
                Some(evaluator) => {
                    decimal_sample(evaluator.at_coordinates(Value::Real(x), Value::Real(y)))?
                }
                None => Some(point[column]),
            };
            records.push((value, *point));
        }
        // The original keeps undefined cells first in both sort directions.
        records.sort_by(|(left, _), (right, _)| match (left, right) {
            (Some(left), Some(right)) if !ascending => right.partial_cmp(left).unwrap(),
            _ => left.partial_cmp(right).unwrap(),
        });
        stopped(&cancel)?;
        axis.points = records.into_iter().map(|(_, point)| point).collect();
        Ok(())
    }

    pub fn numeric(
        &self,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<Vec<Vec<Option<f64>>>, String> {
        let mut definitions = self.compile(snapshot, cancel.clone())?;
        let settings = snapshot
            .app_numeric_settings
            .get(NAME)
            .cloned()
            .unwrap_or_else(|| ppl_vm::AppNumericSettings::for_app(NAME));
        let points = if settings.kind == 1 {
            settings
                .graph3d
                .as_ref()
                .expect("Graph 3D Numeric axes")
                .points
                .iter()
                .copied()
                .take(NUMERIC_ROWS)
                .collect::<Vec<_>>()
        } else {
            (0..50)
                .map(|row| numeric_coordinates(&settings, row))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
        };
        let mut rows = Vec::with_capacity(points.len());
        for [x, y] in points {
            stopped(&cancel)?;
            let mut row = vec![Some(x.to_f64()), Some(y.to_f64())];
            for (_, evaluator) in &mut definitions {
                row.push(numeric(
                    evaluator.at_coordinates(Value::Real(x), Value::Real(y)),
                )?);
            }
            rows.push(row);
        }
        Ok(rows)
    }
}

fn surface_point(
    evaluator: &mut Evaluator,
    index: usize,
    coordinates: [Real; 2],
) -> Result<SurfacePoint, String> {
    let [x, y] = coordinates.map(Value::Real);
    let height = decimal_sample(evaluator.at_coordinates(x, y))?;
    Ok(SurfacePoint {
        index,
        coordinates,
        height,
    })
}

pub(super) fn numeric_coordinates(
    settings: &AppNumericSettings,
    row: usize,
) -> Result<[Real; 2], MathError> {
    let y = settings.graph3d.as_ref().expect("Graph 3D Numeric axes");
    let offset = Real::from_i64(row as i64);
    let x = settings.start.add(settings.step.mul(offset)?)?;
    let y = y.y_start.add(y.y_step.mul(offset)?)?;
    Ok([x, y])
}

fn validate_plot_fields(bounds: &[[f64; 2]; 3], ticks: &[f64; 3]) -> Result<(), String> {
    if bounds.iter().any(|[low, high]| !(high - low).is_finite())
        || ticks.iter().any(|tick| !tick.is_finite() || *tick <= 0.)
    {
        return Err("Graph 3D Plot Setup is outside the renderable range".into());
    }
    Ok(())
}

pub fn position([low, high]: [f64; 2], index: usize, count: usize) -> f64 {
    low + (high - low) * (index as f64 / (count - 1) as f64)
}

fn stopped(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Graph 3D calculation cancelled".into());
    }
    Ok(())
}
