use super::*;
use crate::ide::native::expression::EditCommand;
use crate::ide::native_app_models::spreadsheet::format::AUTO_COLOR;

fn tools() -> NativeTools {
    let mut tools = NativeTools::default();
    tools.memory.selected_app = "Spreadsheet".into();
    tools.open(Page::Numeric);
    tools
}

#[test]
fn reset_clear_requires_confirmation_and_blocks_grid_edits() {
    let mut tools = tools();
    tools.memory.models.spreadsheet.set("A1", "2").unwrap();
    assert!(
        tools
            .spreadsheet_action(A::Edit(EditCommand::Clear))
            .unwrap()
            .is_empty()
    );
    assert!(tools.spreadsheet_panel.confirm_reset);
    assert!(tools.spreadsheet_text("9").unwrap().is_empty());
    assert!(
        tools
            .spreadsheet_paste_text("{{8,9},{10,11}}")
            .unwrap()
            .is_empty()
    );
    assert!(
        tools
            .spreadsheet_action(A::Edit(EditCommand::Backspace))
            .unwrap()
            .is_empty()
    );
    assert_eq!(tools.memory.models.spreadsheet.cells["A1"], "2");
    assert!(tools.spreadsheet_action(A::Back).unwrap().is_empty());
    assert!(!tools.spreadsheet_panel.confirm_reset);
    assert_eq!(tools.memory.models.spreadsheet.cells["A1"], "2");

    assert!(
        tools
            .spreadsheet_action(A::Edit(EditCommand::Clear))
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        tools.spreadsheet_action(A::Evaluate).unwrap().as_slice(),
        [Request::ResetSpreadsheet]
    ));
    assert!(!tools.spreadsheet_panel.confirm_reset);
    assert_eq!(tools.memory.models.spreadsheet.cells["A1"], "2");
}

#[test]
fn requested_dimensions_include_grid_lines_and_visible_row_labels_size_the_gutter() {
    let mut sheet = Spreadsheet::default();
    assert_eq!(view::axes(&sheet, 1, true, 1)[0], (1, 18, 83));
    assert_eq!(view::axes(&sheet, 1, false, 1)[0], (1, 37, 54));
    for row in 1..=5 {
        sheet.row_heights.insert(row, 50);
    }
    for col in 1..=3 {
        sheet.column_widths.insert(col, 100);
    }
    assert_eq!(
        &view::axes(&sheet, 1, true, 1)[..3],
        &[(1, 10, 111), (2, 111, 212), (3, 212, 313)]
    );
    assert_eq!(
        &view::axes(&sheet, 1, false, 1)[..3],
        &[(1, 37, 88), (2, 88, 139), (3, 139, 190)]
    );
    assert_eq!(view::axes(&sheet, 1, true, 98)[0].1, 26);
}

#[test]
fn textbook_menu_preserves_typed_matrix_and_auto_restores_sheet_default() {
    let mut tools = tools();
    let value = ppl_vm::PortableValue::Matrix {
        rows: 2,
        cols: 2,
        vector: false,
        data: (1..=4)
            .map(|n| ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(n)))
            .collect(),
    };
    tools
        .memory
        .models
        .spreadsheet
        .set("A1", "[[1,2],[3,4]]")
        .unwrap();
    tools.memory.models.results.insert(
        "Spreadsheet".into(),
        serde_json::json!({"A1":{"Object":{
        "display":"[[1,2],[3,4]]", "typed":crate::ide::native::portable_json(&value)}}}),
    );
    let settings = ppl_vm::Settings::default();
    assert!(
        !tools.spreadsheet_cell_layout(1, 1, &settings).1,
        "Matrices use the original left Auto alignment"
    );
    let inline = tools.spreadsheet_cell_layout(1, 1, &settings).0;
    tools.spreadsheet_panel.menu = Some(Menu {
        category: Some(10),
        index: 1,
        first: 0,
    });
    tools.spreadsheet_action(A::Evaluate).unwrap();
    let stacked = tools.spreadsheet_cell_layout(1, 1, &settings).0;
    assert!(stacked.height > inline.height * 2);
    assert!(stacked.width < inline.width);
    assert_eq!(
        tools.spreadsheet_cell_text(1, 1, &settings).0,
        "[[1,2],[3,4]]"
    );
    tools.spreadsheet_panel.menu = Some(Menu {
        category: Some(10),
        index: 0,
        first: 0,
    });
    tools.spreadsheet_action(A::Evaluate).unwrap();
    assert_eq!(
        tools.spreadsheet_cell_layout(1, 1, &settings).0.height,
        inline.height
    );
    tools
        .memory
        .models
        .spreadsheet
        .apply_format(
            Block {
                left: 1,
                right: MAX_COLUMNS,
                top: 1,
                bottom: MAX_ROWS,
            },
            CellFormat {
                textbook: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        tools.spreadsheet_cell_layout(1, 1, &settings).0.height,
        stacked.height
    );
    assert!(
        tools
            .spreadsheet_root_items()
            .iter()
            .any(|(code, _)| *code == 10)
    );
}

#[test]
fn font_and_color_auto_restore_inherited_formats_without_erasing_other_fields() {
    let mut tools = tools();
    let sheet = &mut tools.memory.models.spreadsheet;
    sheet
        .apply_format(
            Block {
                left: 1,
                right: 1,
                top: 1,
                bottom: MAX_ROWS,
            },
            CellFormat {
                font: Some(22),
                color: Some(0xff0000),
                ..Default::default()
            },
        )
        .unwrap();
    sheet
        .apply_format(
            Block::cell(1, 1),
            CellFormat {
                font: Some(14),
                color: Some(0x01000000),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(sheet.format(1, 1).color, Some(0x01000000));
    tools.spreadsheet_panel.menu = Some(Menu {
        category: Some(3),
        index: 0,
        first: 0,
    });
    tools.spreadsheet_action(A::Evaluate).unwrap();
    assert_eq!(
        tools.memory.models.spreadsheet.local_format(1, 1).color,
        Some(AUTO_COLOR)
    );
    assert_eq!(
        tools.memory.models.spreadsheet.format(1, 1).color,
        Some(0xff0000)
    );
    assert_eq!(tools.memory.models.spreadsheet.format(1, 1).font, Some(14));
    tools.spreadsheet_panel.menu = Some(Menu {
        category: Some(2),
        index: 0,
        first: 0,
    });
    tools.spreadsheet_action(A::Evaluate).unwrap();
    assert_eq!(tools.memory.models.spreadsheet.format(1, 1).font, Some(22));
    // Navigate to the new final root entry through the production key handler.
    tools.spreadsheet_panel.menu = Some(Menu {
        category: None,
        index: 0,
        first: 0,
    });
    for _ in 0..8 {
        tools.spreadsheet_action(A::History(1)).unwrap();
    }
    tools.spreadsheet_action(A::Evaluate).unwrap();
    assert_eq!(tools.spreadsheet_panel.menu.unwrap().category, Some(10));
    tools
        .spreadsheet_action(A::Edit(EditCommand::Left(false)))
        .unwrap();
}

#[test]
fn full_argb_ink_composites_without_reusing_legitimate_black_as_auto() {
    let opaque = view::composited_color(0x000000ff, 0xffffff);
    let partial = view::composited_color(0x800000ff, 0xffffff);
    let faint = view::composited_color(0xfe0000ff, 0xffffff);
    assert_eq!(opaque, 0x0000ff);
    assert_ne!(partial, opaque);
    assert!(partial > opaque && partial < faint && faint < 0xffffff);
    assert_eq!(view::composited_color(0x01000000, 0), 0);
    assert_eq!(view::composited_color(AUTO_COLOR, 0xff0000), 0xff0000);
}

#[test]
fn textbook_modes_do_not_force_exact_fraction_display_for_numeric_results() {
    let mut tools = tools();
    let mut session = ppl_vm::SessionContext::default();
    session.settings.numeric_version = ppl_math::NumericVersion::Current15751;
    tools.memory.models.spreadsheet = tools
        .memory
        .models
        .spreadsheet
        .enter(
            "A1",
            "1/3",
            &session.snapshot(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    let values = tools
        .memory
        .models
        .spreadsheet
        .calculate(&session.snapshot(), Arc::new(AtomicBool::new(false)))
        .unwrap();
    tools
        .memory
        .models
        .results
        .insert("Spreadsheet".into(), values);
    let mut first_height = None;
    for mode in [0, 1, 2] {
        tools
            .memory
            .models
            .spreadsheet
            .apply_format(
                Block::cell(1, 1),
                CellFormat {
                    textbook: Some(mode),
                    ..Default::default()
                },
            )
            .unwrap();
        let display = tools.spreadsheet_cell_text(1, 1, &session.settings).0;
        assert!(display.starts_with("0.333333"), "{display}");
        assert!(!display.contains('/'));
        let height = tools
            .spreadsheet_cell_layout(1, 1, &session.settings)
            .0
            .height;
        if let Some(first) = first_height {
            assert_eq!(height, first, "Textbook mode{mode} changed decimal layout");
        } else {
            first_height = Some(height);
        }
    }
}

#[test]
fn unpopulated_and_metadata_only_cells_do_not_draw_math_slots() {
    let mut tools = tools();
    for metadata in [false, true] {
        if metadata {
            tools
                .memory
                .models
                .spreadsheet
                .apply_format(
                    Block::cell(1, 1),
                    CellFormat {
                        textbook: Some(1),
                        font: Some(18),
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        let layout = tools
            .spreadsheet_cell_layout(1, 1, &ppl_vm::Settings::default())
            .0;
        assert_eq!(layout.width, 0);
        let mut pixels = ppl_graphics::Grob::new(80, 40, 0xffffff);
        layout.paint_static(&mut pixels, 1, 1, 0);
        assert!(pixels.pixels().iter().all(|&p| p == 0xffffff));
    }
}

#[test]
fn far_selection_scrolls_for_the_actual_row_gutter_and_requested_column_widths() {
    let mut tools = tools();
    for col in MAX_COLUMNS - 2..=MAX_COLUMNS {
        tools
            .memory
            .models
            .spreadsheet
            .column_widths
            .insert(col, 100);
    }
    tools
        .spreadsheet_goto(&cell_name(MAX_COLUMNS, 9999))
        .unwrap();
    let cursor = &tools.memory.spreadsheet_cursor;
    let columns = view::axes(
        &tools.memory.models.spreadsheet,
        cursor.first_column,
        true,
        cursor.first_row,
    );
    let selected = columns.iter().find(|entry| entry.0 == MAX_COLUMNS).unwrap();
    assert!(
        selected.2 <= 320,
        "selected cell extends past the visible grid: {selected:?}"
    );
}

#[test]
fn single_headers_dispatch_to_their_formula_editor_and_restore_the_first_cell() {
    let mut tools = tools();
    tools.spreadsheet_select(2, 1, false);
    tools.memory.spreadsheet_cursor.whole_columns = true;
    let mut requests = Vec::new();
    tools.spreadsheet_softkey(0, &mut requests);
    assert!(matches!(
        requests.as_slice(),
        [Request::EditMath(MathTarget::SpreadsheetCell { address, clear: false, .. })] if address == "B"
    ));
    assert_eq!(tools.spreadsheet_source("B", false), "");
    let snapshot = ppl_vm::SessionContext::default().snapshot();
    tools.memory.models.spreadsheet = tools
        .memory
        .models
        .spreadsheet
        .enter("B", "=Row+2", &snapshot, Arc::new(AtomicBool::new(false)))
        .unwrap();
    tools.spreadsheet_entry_changed("B");
    assert_eq!(tools.memory.models.selected_cell, "B1");
    assert!(!tools.memory.spreadsheet_cursor.whole_columns);
    assert_eq!(tools.spreadsheet_source("B", false), "=Row+2");
    assert_eq!(tools.spreadsheet_source("B1", false), "");

    tools.spreadsheet_select(1, 3, false);
    tools.memory.spreadsheet_cursor.whole_rows = true;
    let requests = tools.spreadsheet_action(A::Evaluate).unwrap();
    assert!(matches!(
        requests.as_slice(),
        [Request::EditMath(MathTarget::SpreadsheetCell { address, clear: false, .. })] if address == "3"
    ));
}
