//! Reuse the calculator's structured expression editor for tool math fields.
use super::*;
use crate::ide::native_app_models::{cell_coordinates, spreadsheet::worksheet_cas};
use crate::ide::native_tools::{MathTarget, Page};
use std::sync::Arc;
pub(super) struct MathEntry {
    target: MathTarget,
    editor: crate::ide::native::expression::ExpressionEditor,
    home_input: String,
    lower: bool,
    previous_page: Page,
    spreadsheet_cas: Option<bool>,
}
impl IdeDesktopApp {
    pub(super) fn graph_math_entry(&self) -> bool {
        self.math_entry.as_ref().is_some_and(|e| {
            matches!(&e.target, MathTarget::Curve {app,..} if app=="Function")
                && e.previous_page == Page::Symbolic
        })
    }
    pub(super) fn function_numeric_math_entry(
        &self,
    ) -> Option<crate::ide::native_tools::function_numeric::Field> {
        match &self.math_entry.as_ref()?.target {
            MathTarget::FunctionNumeric { field, .. } => Some(*field),
            _ => None,
        }
    }
    fn restore_inline_page(&mut self) {
        if self.function_numeric_math_entry().is_some() {
            self.native_tools
                .open(self.math_entry.as_ref().unwrap().previous_page);
            return;
        }
        if self.spreadsheet_math_entry() {
            self.native_tools.open(Page::Numeric);
        } else if self.graph_math_entry() {
            self.native_tools.open(Page::Symbolic);
        }
    }
    pub(super) fn spreadsheet_math_entry(&self) -> bool {
        self.math_entry.as_ref().is_some_and(|e| {
            matches!(
                e.target,
                MathTarget::SpreadsheetCell { .. }
                    | MathTarget::SpreadsheetGoto
                    | MathTarget::SpreadsheetName { .. }
                    | MathTarget::SpreadsheetDimension { .. }
            )
        })
    }
    pub(super) fn spreadsheet_cas_mode(&self) -> Option<bool> {
        self.math_entry.as_ref()?.spreadsheet_cas
    }

    pub(super) fn toggle_spreadsheet_cas(&mut self) {
        if let Some(mode) = self
            .math_entry
            .as_mut()
            .and_then(|entry| entry.spreadsheet_cas.as_mut())
        {
            *mode = !*mode;
            self.native_tools.memory.spreadsheet_cas_mode = *mode;
            self.state.native.alpha_default_lower = *mode;
        }
    }

    pub(super) fn spreadsheet_form(&self) -> Option<crate::ide::native_tools::spreadsheet::Form> {
        use crate::ide::native_tools::spreadsheet::Form;
        match &self.math_entry.as_ref()?.target {
            MathTarget::SpreadsheetGoto => Some(Form::Goto),
            MathTarget::SpreadsheetName { .. } => Some(Form::Name),
            MathTarget::SpreadsheetDimension { property, .. } => Some(if property == "width" {
                Form::Width
            } else {
                Form::Height
            }),
            _ => None,
        }
    }
    pub(super) fn open_math_entry(&mut self, target: MathTarget, ctx: &egui::Context) {
        if self.controller.running() {
            return;
        }
        if let MathTarget::Curve { app, index, .. } = &target {
            if let Err(error) = self.native_tools.graph_prepare_curve(app, *index) {
                self.native_tools.error = Some(error);
                return;
            }
        }
        let mut spreadsheet_cas = None;
        let text = match &target {
            MathTarget::FunctionNumeric { field, clear } => {
                if *clear {
                    String::new()
                } else {
                    self.native_tools.function_numeric_source(*field)
                }
            }
            MathTarget::SpreadsheetCell {
                address,
                clear,
                history,
            } => {
                let draft = self.native_tools.spreadsheet_source(address, *clear);
                if cell_coordinates(address).is_some() {
                    let expression = draft.trim_start();
                    let body = expression.strip_prefix('=').unwrap_or(expression);
                    if let Some(source) = worksheet_cas::body(body) {
                        spreadsheet_cas = Some(true);
                        if expression.starts_with('=') {
                            format!("={source}")
                        } else {
                            source
                        }
                    } else {
                        spreadsheet_cas = Some(
                            *clear
                                && history.is_none()
                                && self.native_tools.memory.spreadsheet_cas_mode,
                        );
                        draft
                    }
                } else {
                    draft
                }
            }
            MathTarget::SpreadsheetGoto => String::new(),
            MathTarget::SpreadsheetName { block } => self
                .native_tools
                .memory
                .models
                .spreadsheet
                .names
                .iter()
                .find(|(_, b)| *b == block)
                .map(|(n, _)| n.clone())
                .unwrap_or_default(),
            MathTarget::SpreadsheetDimension { block, property } => if property == "width" {
                self.native_tools
                    .memory
                    .models
                    .spreadsheet
                    .column_widths
                    .get(&block.left)
                    .map(|v| i64::from(*v))
                    .unwrap_or(-1)
            } else {
                self.native_tools
                    .memory
                    .models
                    .spreadsheet
                    .row_heights
                    .get(&block.top)
                    .map(|v| i64::from(*v))
                    .unwrap_or(-1)
            }
            .to_string(),
            MathTarget::SequenceInitial { app, index, term } => self
                .native_tools
                .memory
                .graphs
                .get(app)
                .and_then(|app| app.curves.get(*index))
                .and_then(|curve| curve.sequence.as_ref())
                .and_then(|definition| definition.initial.get(*term))
                .cloned()
                .unwrap_or_default(),
            MathTarget::Curve { clear: true, .. } => String::new(),
            MathTarget::Curve {
                app, index, second, ..
            } => self
                .native_tools
                .memory
                .graphs
                .get(app)
                .and_then(|a| a.curves.get(*index))
                .map(|c| {
                    if *second {
                        c.second.clone()
                    } else {
                        c.expression.clone()
                    }
                })
                .unwrap_or_default(),
            MathTarget::StatisticsCell {
                two,
                column,
                row,
                clear,
            } => self
                .native_tools
                .statistics_cell_source(*two, *column, *row, *clear),
            MathTarget::StatisticsColumn { two, .. } => {
                self.native_tools.memory.statistics_make_source[usize::from(*two)].clone()
            }
            MathTarget::Slopefield { index, clear } => {
                if *clear {
                    String::new()
                } else {
                    self.native_tools.slopefield_value(*index)
                }
            }
            MathTarget::GeometryPlotField(name) => {
                self.native_tools.geometry_plot_field_source(name)
            }
            MathTarget::GeometryObject { name, preset, .. } => {
                self.native_tools.geometry_object_source(name, preset)
            }
            MathTarget::GeometryMeasurement { index, preset } => self
                .native_tools
                .geometry_measurement_source(*index, preset),
            MathTarget::GeometryName { name } => format!("G{name}"),
            MathTarget::GeometryLabel { index } => self
                .native_tools
                .memory
                .models
                .geometry_book
                .as_ref()
                .and_then(|b| b.measurements.get(*index))
                .map(|m| m.label.clone())
                .unwrap_or_default(),
            MathTarget::StatisticsPlotX => self.native_tools.statistics_plot_x().to_string(),
            MathTarget::StatisticsGoto { two } => {
                (self.native_tools.memory.statistics_cursor[usize::from(*two)].row + 1).to_string()
            }
            MathTarget::StatisticsDefinition {
                analysis,
                frequency,
                clear,
            } => {
                if *clear {
                    String::new()
                } else {
                    self.native_tools
                        .memory
                        .models
                        .statistics1_book
                        .as_ref()
                        .and_then(|book| book.analyses.get(*analysis))
                        .map(|definition| {
                            if *frequency {
                                definition.frequency_expression()
                            } else {
                                definition.data_expression()
                            }
                        })
                        .unwrap_or_default()
                }
            }
            MathTarget::StatisticsTwoReference {
                analysis,
                field,
                clear,
            } => {
                if *clear {
                    String::new()
                } else {
                    self.native_tools
                        .memory
                        .models
                        .statistics2_book
                        .as_ref()
                        .and_then(|book| book.analyses.get(*analysis))
                        .map(|analysis| analysis.reference(*field))
                        .unwrap_or_default()
                }
            }
            MathTarget::StatisticsFit { analysis } => self
                .native_tools
                .memory
                .models
                .statistics2_book
                .as_ref()
                .and_then(|book| book.analyses.get(*analysis))
                .map(|analysis| analysis.user_expression.clone())
                .unwrap_or_default(),
            MathTarget::Object => self.native_tools.memory.content.clone(),
            MathTarget::FunctionBody => self.native_tools.memory.function_body.clone(),
        };
        self.math_entry = Some(MathEntry {
            target,
            editor: self.state.native.editor.clone(),
            home_input: self.state.native.home_input.clone(),
            lower: self.state.native.alpha_default_lower,
            previous_page: self.native_tools.page.unwrap_or(Page::Symbolic),
            spreadsheet_cas,
        });
        if matches!(
            &self.math_entry.as_ref().unwrap().target,
            MathTarget::FunctionNumeric { .. } | MathTarget::Slopefield { .. }
        ) {
            self.native_tools.cancel_calculations();
            self.native_tools.page = None;
        } else {
            self.native_tools.close();
        }
        self.state.native.edit(|e| e.set_text(&text));
        if matches!(
            &self.math_entry.as_ref().unwrap().target,
            MathTarget::GeometryObject {
                preset: Some(_),
                ..
            } | MathTarget::GeometryMeasurement {
                preset: Some(_),
                ..
            }
        ) && text.ends_with("()")
        {
            self.state
                .native
                .edit(|e| e.set_cursor(text.len() - 1, false));
        }

        self.state.native.edit_title = Some("Edit expression · Enter: OK · Esc: cancel".into());
        self.state.native.alpha_default_lower = spreadsheet_cas == Some(true)
            || matches!(
                self.math_entry.as_ref().map(|entry| entry.previous_page),
                Some(Page::CasVariables)
            );
        self.set_native_view(NativeView::Home, ctx);
        self.restore_inline_page();
    }
    pub(super) fn geometry_plot_edit_matches(&self, name: &str, source: &str) -> bool {
        matches!(self.math_entry.as_ref().map(|entry| &entry.target),
            Some(MathTarget::GeometryPlotField(field)) if field == name)
            && self.state.native.home_input == source
    }

    pub(super) fn finish_math_entry(&mut self, commit: bool, ctx: &egui::Context) -> bool {
        if let Some(field) = self.function_numeric_math_entry() {
            if commit {
                self.native_tools.function_numeric_submit(
                    field,
                    self.state.native.home_input.clone(),
                    &self.controller.session_snapshot(),
                );
                ctx.request_repaint();
                return true;
            }
            self.native_tools.function_numeric_cancel();
        }
        if commit
            && let Some(MathTarget::GeometryPlotField(name)) =
                self.math_entry.as_ref().map(|entry| &entry.target)
        {
            if self.model_task.is_none() {
                self.handle_native_tool_request(
                    crate::ide::native_tools::Request::Calculate(
                        crate::ide::native_app_ui::Calculation::GeometryPlotField {
                            name: name.clone(),
                            source: self.state.native.home_input.clone(),
                        },
                    ),
                    ctx,
                );
            }
            return true;
        }
        if commit && self.spreadsheet_form().is_some() {
            let source = self.state.native.home_input.clone();
            let target = &self.math_entry.as_ref().unwrap().target;
            let valid = match target {
                MathTarget::SpreadsheetGoto => {
                    self.native_tools.spreadsheet_target(&source).map(|_| ())
                }
                MathTarget::SpreadsheetName { block } => self
                    .native_tools
                    .memory
                    .models
                    .spreadsheet
                    .clone()
                    .set_name(&source, *block),
                MathTarget::SpreadsheetDimension { property, .. } => {
                    crate::ide::native_tools::spreadsheet::dimension(&source, property == "width")
                        .map(|_| ())
                }
                _ => Ok(()),
            };
            if let Err(e) = valid {
                self.native_tools.error = Some(e);
                ctx.request_repaint();
                return true;
            }
        }
        let graph_edit = self.graph_math_entry();
        if commit && graph_edit && !self.state.native.home_input.trim().is_empty() {
            let (_, d) = ppl_syntax::parse_expression(&self.state.native.home_input);
            if d.has_errors() {
                self.native_tools.error = Some(
                    d.errors()
                        .next()
                        .map_or("Invalid function", |e| e.message.as_str())
                        .into(),
                );
                ctx.request_repaint();
                return true;
            }
        }
        let Some(entry) = self.math_entry.take() else {
            return false;
        };
        let mut pending = None;
        if commit {
            let source = self.state.native.home_input.clone();
            match entry.target {
                MathTarget::FunctionNumeric { .. } => {}
                MathTarget::SpreadsheetCell {
                    address, history, ..
                } => {
                    let source = if entry.spreadsheet_cas == Some(true) && !source.trim().is_empty()
                    {
                        if let Some(formula) = source.trim_start().strip_prefix('=') {
                            format!("={}", worksheet_cas::wrap(formula))
                        } else {
                            worksheet_cas::wrap(&source)
                        }
                    } else {
                        source
                    };
                    self.native_tools.memory.spreadsheet_draft =
                        Some(crate::ide::native_tools::spreadsheet::Draft {
                            address: address.clone(),
                            source: source.clone(),
                        });
                    use crate::ide::{native_app_models, native_app_ui::Calculation};
                    let models = &self.native_tools.memory.models;
                    let sheet = models.spreadsheet.clone();
                    let geometry = models.geometry_book.clone().map(Arc::new);
                    let calculation = if history.is_some()
                        && native_app_models::Spreadsheet::is_clipboard_collection(&source)
                        && let Some((col, row)) = native_app_models::cell_coordinates(&address)
                    {
                        Calculation::SpreadsheetPasteCollection {
                            sheet,
                            text: source,
                            destination: [col, row],
                            geometry,
                        }
                    } else {
                        Calculation::SpreadsheetEntry(sheet, address, source, geometry)
                    };
                    pending = Some(crate::ide::native_tools::Request::Calculate(calculation));
                }
                MathTarget::SpreadsheetGoto => {
                    if let Err(e) = self.native_tools.spreadsheet_goto(&source) {
                        self.native_tools.error = Some(e);
                    }
                }
                MathTarget::SpreadsheetName { block } => {
                    match self
                        .native_tools
                        .memory
                        .models
                        .spreadsheet
                        .set_name(&source, block)
                    {
                        Ok(()) => self.native_tools.spreadsheet_changed(false),
                        Err(e) => self.native_tools.error = Some(e),
                    }
                }
                MathTarget::SpreadsheetDimension { block, property } => {
                    match crate::ide::native_tools::spreadsheet::dimension(
                        &source,
                        property == "width",
                    ) {
                        Ok(value) => {
                            let sheet = &mut self.native_tools.memory.models.spreadsheet;
                            if property == "width" {
                                for c in block.left..=block.right {
                                    if let Some(n) = value {
                                        sheet.column_widths.insert(c, n);
                                    } else {
                                        sheet.column_widths.remove(&c);
                                    }
                                }
                            } else {
                                for r in block.top..=block.bottom {
                                    if let Some(n) = value {
                                        sheet.row_heights.insert(r, n);
                                    } else {
                                        sheet.row_heights.remove(&r);
                                    }
                                }
                            }
                            self.native_tools.spreadsheet_changed(false);
                        }
                        Err(e) => self.native_tools.error = Some(e),
                    }
                }
                MathTarget::SequenceInitial { app, index, term } => {
                    if let Some(value) = self
                        .native_tools
                        .memory
                        .graphs
                        .get_mut(&app)
                        .and_then(|app| app.curves.get_mut(index))
                        .and_then(|curve| curve.sequence.as_mut())
                        .and_then(|definition| definition.initial.get_mut(term))
                    {
                        *value = source;
                    }
                }
                MathTarget::Curve {
                    app, index, second, ..
                } => {
                    let nonempty = !source.trim().is_empty();
                    if let Some(curve) = self
                        .native_tools
                        .memory
                        .graphs
                        .get_mut(&app)
                        .and_then(|a| a.curves.get_mut(index))
                    {
                        if second {
                            curve.second = source
                        } else {
                            curve.expression = source;
                            curve.literal = None;
                        }
                        if graph_edit {
                            curve.enabled = nonempty;
                        }
                    }
                    if graph_edit {
                        self.native_tools.graph_finished(index, nonempty);
                    }
                }
                MathTarget::StatisticsCell {
                    two, column, row, ..
                } => {
                    match self
                        .native_tools
                        .statistics_cell_request(two, column, row, source)
                    {
                        Ok(request) => pending = Some(request),
                        Err(error) => self.native_tools.error = Some(error),
                    }
                }
                MathTarget::StatisticsColumn { two, column } => {
                    self.native_tools.memory.statistics_make_source[usize::from(two)] =
                        source.clone();
                    if let Some(columns) = self.native_tools.memory.models.statistics_columns(two) {
                        pending = Some(crate::ide::native_tools::Request::Calculate(
                            crate::ide::native_app_ui::Calculation::StatisticsColumn {
                                two,
                                column,
                                source,
                                columns: columns.clone(),
                            },
                        ));
                    }
                }
                MathTarget::Slopefield { index, .. } => {
                    self.native_tools.slopefield_set(index, source)
                }
                MathTarget::GeometryPlotField(_) => {
                    unreachable!("Geometry Plot Setup uses the worker")
                }
                MathTarget::GeometryObject { name, insert, .. } => {
                    if let Some(book) = self.native_tools.memory.models.geometry_book.clone() {
                        self.native_tools.memory.geometry_draft =
                            Some(crate::ide::native_tools::geometry::Draft {
                                name: Some(name.clone()),
                                index: 0,
                                source: source.clone(),
                            });
                        pending = Some(crate::ide::native_tools::Request::Calculate(
                            crate::ide::native_app_ui::Calculation::GeometryEdit {
                                book: Box::new(book),
                                name,
                                source,
                                insert,
                            },
                        ));
                    }
                }
                MathTarget::GeometryMeasurement { index, .. } => {
                    if let Some(book) = self.native_tools.memory.models.geometry_book.clone() {
                        self.native_tools.memory.geometry_draft =
                            Some(crate::ide::native_tools::geometry::Draft {
                                name: None,
                                index,
                                source: source.clone(),
                            });
                        pending = Some(crate::ide::native_tools::Request::Calculate(
                            crate::ide::native_app_ui::Calculation::GeometryMeasurement {
                                book: Box::new(book),
                                index,
                                source,
                            },
                        ));
                    }
                }
                MathTarget::GeometryName { name } => {
                    if let Some(book) = &mut self.native_tools.memory.models.geometry_book {
                        match book.rename(&name, &source) {
                            Ok(()) => self.native_tools.geometry_changed(),
                            Err(e) => self.native_tools.error = Some(e),
                        }
                    }
                }
                MathTarget::GeometryLabel { index } => {
                    if source.len() > 256 {
                        self.native_tools.error = Some("Geometry label exceeds256bytes".into());
                    } else if let Some(m) = self
                        .native_tools
                        .memory
                        .models
                        .geometry_book
                        .as_mut()
                        .and_then(|b| b.measurements.get_mut(index))
                    {
                        m.label = source;
                        self.native_tools.geometry_changed();
                    }
                }
                MathTarget::StatisticsPlotX => {
                    match ppl_math::Real::parse(source.trim())
                        .map(|n| n.to_f64())
                        .filter(|n| n.is_finite())
                    {
                        Some(x) => self.native_tools.statistics_plot_goto(x),
                        None => {
                            self.native_tools.error = Some("Enter a finite X coordinate".into())
                        }
                    }
                }
                MathTarget::StatisticsGoto { two } => {
                    let limit = self
                        .native_tools
                        .memory
                        .models
                        .statistics_columns(two)
                        .and_then(|columns| {
                            columns.0.get(
                                self.native_tools.memory.statistics_cursor[usize::from(two)].column,
                            )
                        })
                        .map_or(1, |values| {
                            (values.len() + 1).min(crate::ide::native_statistics::MAX_ROWS)
                        });
                    match source.trim().parse::<usize>() {
                        Ok(row) if row > 0 && row <= limit => {
                            self.native_tools.memory.statistics_cursor[usize::from(two)].row =
                                row - 1
                        }
                        _ => self.native_tools.error = Some(format!("Choose row1 through{limit}")),
                    }
                }
                MathTarget::StatisticsDefinition {
                    analysis,
                    frequency,
                    ..
                } => {
                    let result = self
                        .native_tools
                        .memory
                        .models
                        .statistics1_book
                        .as_mut()
                        .and_then(|book| book.analyses.get_mut(analysis))
                        .ok_or_else(|| "Missing statistics definition".to_owned())
                        .and_then(|definition| {
                            definition.assign_field(
                                if frequency { 2 } else { 1 },
                                &ppl_vm::PortableValue::String(source),
                            )
                        });
                    match result {
                        Ok(_) => {
                            self.native_tools
                                .memory
                                .models
                                .results
                                .retain(|name, _| !name.starts_with("Statistics 1Var"));
                        }
                        Err(error) => self.native_tools.error = Some(error),
                    }
                }
                MathTarget::StatisticsFit { analysis }
                | MathTarget::StatisticsTwoReference { analysis, .. } => {
                    let field = match entry.target {
                        MathTarget::StatisticsTwoReference { field, .. } => field,
                        _ => 5,
                    };
                    let result = self
                        .native_tools
                        .memory
                        .models
                        .statistics2_book
                        .as_mut()
                        .ok_or_else(|| "Missing statistics definition".to_owned())
                        .and_then(|book| {
                            book.assign_field(
                                analysis,
                                field,
                                &ppl_vm::PortableValue::String(source),
                            )
                        });
                    match result {
                        Ok(()) => self.native_tools.statistics_invalidate(true),
                        Err(error) => self.native_tools.error = Some(error),
                    }
                }
                MathTarget::Object => self.native_tools.memory.content = source,
                MathTarget::FunctionBody => self.native_tools.memory.function_body = source,
            }
        }
        self.state.native.editor = entry.editor;
        self.state.native.home_input = entry.home_input;
        self.state.native.alpha_default_lower = entry.lower;
        self.state.native.edit_title = None;
        // Returning from an editor is not a new Plot visit. In particular,
        // Go To has just chosen a trace sample that opening Plot must retain.
        let plot_trace = (entry.previous_page == Page::Plot).then_some(self.native_tools.trace);
        self.native_tools.open(entry.previous_page);
        if let Some(trace) = plot_trace {
            self.native_tools.trace = trace;
        }
        self.release_native_input();
        if let Some(request) = pending {
            self.handle_native_tool_request(request, ctx);
        }
        self.state.native.touch_view();
        ctx.request_repaint();
        true
    }
}
