//! The VM owns C/D lists. GUI analyses retain definitions and a read cache.
use super::*;
use crate::ide::native_statistics::Columns;
impl IdeDesktopApp {
    pub(super) fn sync_native_statistics_memory(&mut self) {
        let snapshot = self.controller.session_snapshot();
        if self.controller.operation_active()
            && self
                .native_tools
                .statistics_context_pinned(snapshot.generation)
        {
            return;
        }
        let seeds = match self.native_tools.statistics_seed_columns(&snapshot) {
            Ok(seeds) => seeds,
            Err(error) => {
                self.native_tools.statistics_sync_error(Some(error));
                return;
            }
        };
        if !seeds.is_empty() {
            if self.controller.operation_active() {
                self.native_tools.statistics_sync_error(Some(
                    "Stop the running calculation to import older statistics data".into(),
                ));
                return;
            }
            let result = self.controller.mutate_memory(|memory| {
                let present = memory.snapshot().globals;
                for (name, value) in seeds {
                    if !present.contains_key(&name) {
                        memory
                            .set_variable(&name, value)
                            .map_err(|e| e.to_string())?;
                    }
                }
                Ok(())
            });
            if let Err(error) = result {
                self.native_tools.statistics_sync_error(Some(error));
                return;
            }
        }
        let snapshot = self.controller.session_snapshot();
        match self.native_tools.import_statistics_columns(&snapshot) {
            Ok(_) => self.native_tools.statistics_sync_error(None),
            Err(error) => self.native_tools.statistics_sync_error(Some(error)),
        }
    }
    pub(super) fn write_statistics_columns(
        &mut self,
        two: bool,
        updates: Vec<(usize, Vec<ppl_vm::PortableValue>)>,
        owner: Option<&crate::ide::execution::worker::SessionLease>,
    ) -> Result<(), String> {
        let mut values = Vec::new();
        let mut names = std::collections::BTreeSet::new();
        for (index, data) in updates {
            if index >= 10 || data.len() > crate::ide::native_statistics::MAX_ROWS {
                return Err("Invalid statistics column dimensions".into());
            }
            let name = Columns::label(index, two);
            if !names.insert(name.clone()) {
                return Err("Duplicate statistics column update".into());
            }
            let value = data
                .into_iter()
                .map(|value| {
                    value
                        .to_value()
                        .expect_real("statistics data")
                        .map(ppl_vm::PortableValue::Real)
                        .map_err(|error| error.to_string())
                })
                .collect::<Result<Vec<_>, String>>()?;
            values.push((name, ppl_vm::PortableValue::List(value)));
        }
        let write = |memory: &mut ppl_vm::SessionContext| {
            for (name, value) in values {
                memory
                    .set_variable(&name, value)
                    .map_err(|e| e.to_string())?;
            }
            Ok(())
        };
        if let Some(owner) = owner {
            self.controller.commit_native_operation(owner, write)?;
        } else {
            self.controller.mutate_memory(write)?;
        }
        self.native_tools
            .import_statistics_columns(&self.controller.session_snapshot())?;
        self.native_tools.statistics_sync_error(None);
        Ok(())
    }
    pub(super) fn write_statistics_typed_column(
        &mut self,
        two: bool,
        column: usize,
        value: ppl_vm::PortableValue,
        owner: Option<&crate::ide::execution::worker::SessionLease>,
    ) -> Result<(), String> {
        let ppl_vm::PortableValue::List(values) = value else {
            return Err("A statistics column requires a list of real numbers".into());
        };
        self.write_statistics_columns(two, vec![(column, values)], owner)
    }
    pub(super) fn write_statistics_typed_cell(
        &mut self,
        two: bool,
        column: usize,
        row: usize,
        value: ppl_vm::PortableValue,
        owner: Option<&crate::ide::execution::worker::SessionLease>,
    ) -> Result<(), String> {
        if column >= crate::ide::native_statistics::COLUMN_COUNT
            || row >= crate::ide::native_statistics::MAX_ROWS
        {
            return Err("Invalid statistics cell address".into());
        }
        let snapshot = self.controller.session_snapshot();
        let name = Columns::label(column, two);
        let mut values = match snapshot.globals.get(&name) {
            Some(ppl_vm::PortableValue::List(values)) => values.clone(),
            Some(_) => return Err(format!("{name} is not a statistics data list")),
            None => Vec::new(),
        };
        if row > values.len() {
            return Err("Enter statistics cells without gaps".into());
        }
        let value = value
            .to_value()
            .expect_real("statistics data")
            .map(ppl_vm::PortableValue::Real)
            .map_err(|error| error.to_string())?;
        if row == values.len() {
            values.push(value);
        } else {
            values[row] = value;
        }
        self.write_statistics_typed_column(
            two,
            column,
            ppl_vm::PortableValue::List(values),
            owner,
        )?;
        self.native_tools
            .statistics_cell_committed(two, column, row);
        Ok(())
    }
    pub(super) fn write_statistics_cell(
        &mut self,
        two: bool,
        column: usize,
        row: usize,
        value: f64,
        owner: Option<&crate::ide::execution::worker::SessionLease>,
    ) -> Result<(), String> {
        let value = ppl_math::Real::from_f64(value).map_err(|error| error.to_string())?;
        self.write_statistics_typed_cell(
            two,
            column,
            row,
            ppl_vm::PortableValue::Real(value),
            owner,
        )
    }
    pub(super) fn delete_statistics_cell(
        &mut self,
        two: bool,
        column: usize,
        row: usize,
    ) -> Result<(), String> {
        self.native_tools
            .memory
            .models
            .ensure_statistics_book(two)?;
        if column >= crate::ide::native_statistics::COLUMN_COUNT {
            return Err("Invalid statistics column".into());
        }
        let name = Columns::label(column, two);
        let snapshot = self.controller.session_snapshot();
        let mut values = match snapshot.globals.get(&name) {
            Some(ppl_vm::PortableValue::List(values)) => values.clone(),
            Some(_) => return Err(format!("{name} is not a statistics data list")),
            None => Vec::new(),
        };
        if row >= values.len() {
            return Err("Invalid statistics row".into());
        }
        values.remove(row);
        self.write_statistics_typed_column(two, column, ppl_vm::PortableValue::List(values), None)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ide::native_tools::{Page, Request};
    use ppl_vm::PortableValue as P;
    fn list(values: &[i64]) -> P {
        P::List(
            values
                .iter()
                .map(|&n| P::Real(ppl_math::Real::from_i64(n)))
                .collect(),
        )
    }
    #[test]
    fn typed_cell_edits_and_deletions_preserve_other_exact_values_and_valid_extreme_exponents() {
        let mut app = IdeDesktopApp::default();
        app.native_tools.memory.selected_app = "Statistics 1Var".into();
        app.native_tools.open(Page::Numeric);
        let version = ppl_math::NumericVersion::Current15751;
        let parse = |source| ppl_math::Real::parse_versioned(source, version).unwrap();
        let original = vec![
            P::Real(parse("1234567890123456789")),
            P::Real(
                ppl_math::Real::ONE
                    .div(ppl_math::Real::from_i64(3))
                    .unwrap(),
            ),
            P::Real(parse("1E400")),
            P::Real(parse("1E-400")),
        ];
        app.controller
            .mutate_memory(|memory| {
                memory.settings.numeric_version = version;
                memory
                    .set_variable("D1", P::List(original.clone()))
                    .map_err(|error| error.to_string())
            })
            .unwrap();
        app.sync_native_statistics_memory();
        assert_eq!(
            app.native_tools.statistics_cell_source(false, 0, 2, false),
            parse("1E400").ppl_source()
        );
        assert_eq!(
            app.native_tools.statistics_cell_source(false, 0, 3, false),
            parse("1E-400").ppl_source()
        );
        app.write_statistics_typed_cell(false, 0, 1, P::Real(parse("7")), None)
            .unwrap();
        let mut expected = original;
        expected[1] = P::Real(parse("7"));
        let snapshot = app.controller.session_snapshot();
        assert_eq!(
            ppl_vm::session::encode_value(&snapshot.globals["D1"]),
            ppl_vm::session::encode_value(&P::List(expected.clone()))
        );
        app.delete_statistics_cell(false, 0, 1).unwrap();
        expected.remove(1);
        assert_eq!(
            ppl_vm::session::encode_value(&app.controller.session_snapshot().globals["D1"]),
            ppl_vm::session::encode_value(&P::List(expected))
        );
        let before = app.controller.session_snapshot();
        assert!(
            app.write_statistics_typed_column(
                false,
                0,
                P::List(vec![P::String("invalid".into())]),
                None
            )
            .is_err()
        );
        assert_eq!(app.controller.session_snapshot().globals, before.globals);
    }
    #[test]
    fn migration_seeds_absent_only_and_never_replaces_explicit_ppl_data_or_empty_lists() {
        let mut app = IdeDesktopApp::default();
        app.native_tools.memory.selected_app = "Statistics 1Var".into();
        app.native_tools.memory.models.statistics1.samples = vec![1., 2.];
        app.native_tools.memory.models.statistics1.frequencies = vec![2, 1];
        app.sync_native_statistics_memory();
        let snapshot = app.controller.session_snapshot();
        assert_eq!(snapshot.globals["D1"], list(&[1, 2]));
        assert_eq!(snapshot.globals["D2"], list(&[2, 1]));
        app.controller
            .mutate_memory(|memory| {
                memory
                    .set_variable("D1", list(&[4, 8]))
                    .map_err(|e| e.to_string())
            })
            .unwrap();
        app.sync_native_statistics_memory();
        assert_eq!(
            app.native_tools
                .memory
                .models
                .statistics1_book
                .as_ref()
                .unwrap()
                .columns
                .0[0],
            vec![4., 8.]
        );
        app.controller
            .mutate_memory(|memory| {
                memory
                    .set_variable("D1", list(&[]))
                    .map_err(|e| e.to_string())
            })
            .unwrap();
        app.native_tools.memory.models.statistics1_book = None;
        app.sync_native_statistics_memory();
        assert!(
            app.native_tools
                .memory
                .models
                .statistics1_book
                .as_ref()
                .unwrap()
                .columns
                .0[0]
                .is_empty()
        );
        assert_eq!(app.controller.session_snapshot().globals["D1"], list(&[]));
    }
    #[test]
    fn real_calculation_worker_commits_a_cell_with_app_angle_and_does_not_promote_ans() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();
        app.native_tools.memory.selected_app = "Statistics 2Var".into();
        app.native_tools.open(Page::Numeric);
        app.controller
            .mutate_memory(|memory| {
                memory.app_settings.insert(
                    "Statistics 2Var".into(),
                    ppl_vm::AppSettings {
                        angle: 2,
                        ..Default::default()
                    },
                );
                Ok(())
            })
            .unwrap();
        app.sync_native_statistics_memory();
        let before = app.controller.session_snapshot();
        let request = app
            .native_tools
            .statistics_cell_request(true, 0, 0, "SIN(30)".into())
            .unwrap();
        assert!(matches!(request, Request::Calculate(..)));
        app.handle_native_tool_request(request, &ctx);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while app.model_task.is_some() {
            app.poll_native_tools(&ctx);
            assert!(
                std::time::Instant::now() < deadline,
                "{:?}",
                app.native_tools.error
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            app.native_tools.error.is_none(),
            "{:?}",
            app.native_tools.error
        );
        let snapshot = app.controller.session_snapshot();
        assert_eq!(
            snapshot.globals["C1"],
            P::List(vec![P::Real(ppl_math::Real::parse("0.5").unwrap())])
        );
        assert_eq!(snapshot.last_value, before.last_value);
        assert_eq!(snapshot.globals.get("Ans"), before.globals.get("Ans"));
        assert_eq!(
            app.native_tools
                .memory
                .models
                .statistics2_book
                .as_ref()
                .unwrap()
                .columns
                .0[0],
            vec![0.5]
        );
        assert_eq!(app.native_tools.memory.statistics_cursor[1].row, 1);
    }
    #[test]
    fn fast_numeric_keyboard_entry_advances_rows_without_touching_the_home_draft() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();
        app.state.layout = crate::ide::layout::WorkspaceLayout::Calculator;
        app.state.calc_input_focus = true;
        app.state
            .native
            .set_view(crate::ide::native::NativeView::Home);
        app.state.native.edit(|editor| editor.set_text("7+"));
        app.native_tools.memory.selected_app = "Statistics 2Var".into();
        app.native_tools.open(Page::Numeric);
        app.sync_native_statistics_memory();
        let key = |pressed| egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let _ = ctx.run(
            egui::RawInput {
                focused: true,
                events: vec![
                    egui::Event::Text("1".into()),
                    key(true),
                    key(false),
                    egui::Event::Text("2".into()),
                    key(true),
                    key(false),
                    egui::Event::Text("3".into()),
                    egui::Event::Key {
                        key: egui::Key::A,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::COMMAND,
                    },
                    egui::Event::Text("4".into()),
                    egui::Event::Key {
                        key: egui::Key::C,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::COMMAND,
                    },
                    key(true),
                    key(false),
                ],
                ..Default::default()
            },
            |ctx| app.route_native_keyboard(ctx),
        );
        assert_eq!(
            app.controller.session_snapshot().globals["C1"],
            list(&[1, 2, 4])
        );
        assert_eq!(app.native_tools.memory.statistics_cursor[1].row, 3);
        assert_eq!(app.state.native.home_input, "7+");
        assert!(app.math_entry.is_none());
        assert!(app.model_task.is_none());
    }
}
