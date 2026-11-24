//! Reuse the calculator's structured expression editor for tool math fields.
use super::*;
use crate::ide::native_tools::{MathTarget, Page};
pub(super) struct MathEntry {
    target: MathTarget,
    editor: crate::ide::native::expression::ExpressionEditor,
    home_input: String,
    lower: bool,
    previous_page: Page,
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
        let text = match &target {
            MathTarget::FunctionNumeric { field, clear } => {
                if *clear {
                    String::new()
                } else {
                    self.native_tools.function_numeric_source(*field)
                }
            }
            MathTarget::SpreadsheetCell { address, clear } => {
                self.native_tools.spreadsheet_source(address, *clear)
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
        });
        if matches!(
            &self.math_entry.as_ref().unwrap().target,
            MathTarget::FunctionNumeric { .. }
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
        self.state.native.alpha_default_lower = matches!(
            self.math_entry.as_ref().map(|entry| entry.previous_page),
            Some(Page::CasVariables)
        );
        self.set_native_view(NativeView::Home, ctx);
        self.restore_inline_page();
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
                MathTarget::SpreadsheetCell { address, .. } => {
                    self.native_tools.memory.spreadsheet_draft =
                        Some(crate::ide::native_tools::spreadsheet::Draft {
                            address: address.clone(),
                            source: source.clone(),
                        });
                    pending = Some(crate::ide::native_tools::Request::Calculate(
                        crate::ide::native_app_ui::Calculation::SpreadsheetEntry(
                            self.native_tools.memory.models.spreadsheet.clone(),
                            address,
                            source,
                        ),
                    ));
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
                MathTarget::StatisticsFit { analysis } => {
                    if let Some(analysis) = self
                        .native_tools
                        .memory
                        .models
                        .statistics2_book
                        .as_mut()
                        .and_then(|book| book.analyses.get_mut(analysis))
                    {
                        analysis.user_expression = source;
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
        self.native_tools.open(entry.previous_page);
        self.release_native_input();
        if let Some(request) = pending {
            self.handle_native_tool_request(request, ctx);
        }
        self.state.native.touch_view();
        ctx.request_repaint();
        true
    }
}
