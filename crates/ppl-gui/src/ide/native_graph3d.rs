//! Compiled, cancellable surface evaluation for Graph 3D.
use super::native_apps::{Evaluator, numeric};
use ppl_math::Real;
use ppl_vm::{SessionSnapshot, Value};
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub const NAME: &str = "Graph 3D";
pub const FUNCTIONS: [&str; 10] = [
    "FZ1", "FZ2", "FZ3", "FZ4", "FZ5", "FZ6", "FZ7", "FZ8", "FZ9", "FZ0",
];
const RESOLUTION: usize = 25;
pub type Definitions = [Definition; FUNCTIONS.len()];

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
}

impl Default for Book {
    fn default() -> Self {
        Self {
            definitions: Default::default(),
            bounds: [[-4., 4.]; 3],
            ticks: [1.; 3],
            rotation: [-45., 25.],
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SurfaceGrid {
    pub index: usize,
    pub heights: Vec<Option<f64>>,
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
        if self
            .bounds
            .iter()
            .any(|[low, high]| low >= high || !(high - low).is_finite())
            || self
                .ticks
                .iter()
                .any(|tick| !tick.is_finite() || *tick <= 0.)
            || self
                .rotation
                .iter()
                .any(|angle| !angle.is_finite() || angle.abs() > 360.)
        {
            return Err("Invalid Graph 3D viewport".into());
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
        let mut memory = snapshot.clone();
        memory.active_app = NAME.into();
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

    pub fn mesh(
        &self,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<Mesh, String> {
        let mut grids = Vec::new();
        for (index, mut evaluator) in self.compile(snapshot, cancel.clone())? {
            let mut heights = Vec::with_capacity(RESOLUTION * RESOLUTION);
            for row in 0..RESOLUTION {
                let y = position(self.bounds[1], row, RESOLUTION);
                for column in 0..RESOLUTION {
                    stopped(&cancel)?;
                    let x = position(self.bounds[0], column, RESOLUTION);
                    heights.push(numeric(evaluator.at_xy_value(x, y))?);
                }
            }
            grids.push(SurfaceGrid { index, heights });
        }
        Ok(Mesh {
            resolution: RESOLUTION,
            grids,
        })
    }

    pub fn numeric(
        &self,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<Vec<Vec<Option<f64>>>, String> {
        let mut definitions = self.compile(snapshot, cancel.clone())?;
        let mut rows = Vec::with_capacity(50);
        let step = Real::parse("0.1").expect("valid decimal step");
        for index in 0..50 {
            stopped(&cancel)?;
            let coordinate = Real::from_i64(index).mul(step).map_err(|e| e.to_string())?;
            let argument = Value::Real(coordinate);
            let mut row = vec![Some(coordinate.to_f64()); 2];
            for (_, evaluator) in &mut definitions {
                row.push(numeric(
                    evaluator.at_coordinates(argument.clone(), argument.clone()),
                )?);
            }
            rows.push(row);
        }
        Ok(rows)
    }
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
