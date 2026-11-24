//! BCD table workers use the authoritative app settings and local app resolver.
use super::*;
use crate::ide::native_application_access::NumericRows;
pub(super) struct TypedTableJob {
    generation: u64,
    source: serde_json::Value,
    rx: mpsc::Receiver<Result<(NumericRows, Option<String>), String>>,
    cancel: Arc<AtomicBool>,
}
impl Drop for TypedTableJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
impl NativeTools {
    pub fn function_numeric_calculating(&self) -> bool {
        self.numeric_panel.job.is_some() || self.numeric_panel.table_job.is_some()
    }
    pub fn function_numeric_cancel_all(&mut self) {
        self.numeric_panel.job = None;
        self.numeric_panel.table_job = None;
    }
    pub fn function_numeric_invalidate(&mut self) {
        self.numeric_panel.table_job = None;
        self.numeric_panel.table_values.clear();
    }
    pub fn numeric_typed_table(&self) -> &NumericRows {
        &self.numeric_panel.table_values
    }
    pub(in crate::ide::native_tools) fn function_numeric_start_table(
        &mut self,
        snapshot: &SessionSnapshot,
    ) {
        self.function_numeric_apply_context(snapshot);
        self.numeric_panel.table_job = None;
        let numeric = self.memory.function_numeric.settings.clone();
        let values = if numeric.kind == 1 {
            Ok(numeric
                .independent
                .iter()
                .take(1000)
                .copied()
                .collect::<Vec<_>>())
        } else {
            ppl_math::with_overflow_policy(ppl_math::OverflowPolicy::Saturate, || {
                (0..50)
                    .map(|i| numeric.start.add(numeric.step.mul(Real::from_i64(i))?))
                    .collect::<Result<Vec<_>, _>>()
            })
        };
        let values = match values {
            Ok(v) => v,
            Err(e) => {
                self.record_calculation_error("table", Some(e.to_string()));
                return;
            }
        };
        let graph = self.memory.graphs["Function"].clone();
        let source = self.function_numeric_source_key();
        let sheet = self.memory.models.spreadsheet.clone();
        let generation = snapshot.generation;
        let mut context = snapshot.clone();
        context.active_app = "Function".into();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = crate::ide::native_application_access::evaluate_numeric_rows(
                sheet, graph, &context, values, flag,
            )
            .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        self.numeric_panel.table_job = Some(TypedTableJob {
            generation,
            source,
            rx,
            cancel,
        });
        self.dirty();
    }
    pub(super) fn function_numeric_poll_table(&mut self, snapshot: &SessionSnapshot) {
        let Some(job) = &self.numeric_panel.table_job else {
            return;
        };
        let result = match job.rx.try_recv() {
            Ok(v) => v,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Numeric table calculation stopped".into())
            }
        };
        let job = self.numeric_panel.table_job.take().unwrap();
        let source = self.function_numeric_source_key();
        if job.source != source
            || (!self.presentation_pending && job.generation != snapshot.generation)
        {
            self.numeric_panel.table_values.clear();
            self.table.clear();
            self.record_calculation_error(
                "table",
                Some("Numeric table source changed during calculation".into()),
            );
            self.dirty();
            return;
        }
        match result {
            Ok((values, error)) => {
                self.table = values
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|value| {
                                value
                                    .as_ref()
                                    .and_then(|v| v.to_value().as_real())
                                    .map(|r| r.to_f64())
                                    .filter(|n| n.is_finite())
                            })
                            .collect()
                    })
                    .collect();
                self.numeric_panel.table_values = values;
                self.record_calculation_error("table", error);
            }
            Err(error) => {
                self.numeric_panel.table_values.clear();
                self.table.clear();
                self.record_calculation_error("table", Some(error));
            }
        }
        self.dirty();
    }
}
