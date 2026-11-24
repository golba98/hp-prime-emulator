//! Genuine G1 controls exercise local records independently of inherited paint.
use super::*;
use format::{AUTO_COLOR, Block, CellFormat};
use ppl_vm::{ErrorKind, host::AppVariableRequest};
fn snapshot() -> SessionSnapshot {
    ppl_vm::SessionContext::default().snapshot()
}
fn cancel() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}
fn n(value: i64) -> PortableValue {
    PortableValue::Real(Real::from_i64(value))
}
fn text(value: &str) -> PortableValue {
    PortableValue::String(value.into())
}
fn get(sheet: &Spreadsheet, name: &str, field: i64) -> PortableValue {
    sheet
        .read_attribute(name, field, &snapshot(), cancel())
        .unwrap()
}
fn put(sheet: &mut Spreadsheet, name: &str, field: i64, value: PortableValue) {
    *sheet = sheet.write_attribute(name, field, &value).unwrap();
}
fn control_value(value: &serde_json::Value) -> PortableValue {
    match value {
        serde_json::Value::String(value) => text(value),
        serde_json::Value::Number(value) => n(value.as_i64().unwrap()),
        serde_json::Value::Array(values) => {
            PortableValue::List(values.iter().map(control_value).collect())
        }
        _ => panic!("unsupported control {value}"),
    }
}
fn controls() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../../../../tests/fixtures/native-spreadsheet-attribute-controls.json"
    ))
    .unwrap()
}
#[test]
fn original_empty_and_metadata_only_records_remain_visually_distinct_after_reopen() {
    let mut sheet = Spreadsheet::default();
    assert_eq!(get(&sheet, "A1", 0), n(0));
    for field in -1..=12 {
        if field != 0 {
            assert_eq!(get(&sheet, "A1", field), n(-1), "{field}");
        }
    }
    // A rejected format value succeeds and creates only a metadata record.
    put(&mut sheet, "A1", 5, n(99));
    assert!(!sheet.cells.contains_key("A1"));
    assert_eq!(get(&sheet, "A1", 1), n(0));
    assert_eq!(get(&sheet, "A1", 2), text(""));
    assert_eq!(get(&sheet, "A1", 5), n(-1));
    assert_eq!(get(&sheet, "A1", 12), n(0));
    let PortableValue::List(record) = get(&sheet, "A1", -1) else {
        panic!()
    };
    assert_eq!(record.len(), 11);
    assert_eq!(record[0], n(0));
    assert_eq!(record[1], text(""));
    assert!(record[2..].iter().all(|v| *v == n(-1)));
    let values = sheet.calculate(&snapshot(), cancel()).unwrap();
    assert!(
        values.get("A1").is_none(),
        "Metadata is not entered zero content"
    );
    let restored: Spreadsheet =
        serde_json::from_value(serde_json::to_value(&sheet).unwrap()).unwrap();
    assert_eq!(get(&restored, "A1", -1), get(&sheet, "A1", -1));
    // A column default does not materialize its absent child cells.
    sheet
        .apply_format(
            Block {
                left: 2,
                right: 2,
                top: 1,
                bottom: MAX_ROWS,
            },
            CellFormat {
                font: Some(22),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(sheet.format(2, 3).font, Some(22));
    assert_eq!(get(&sheet, "B3", -1), n(-1));
    assert_eq!(get(&sheet, "B3", 1), n(-1));
    // Supported read-only/record setters also materialize, unknown indices do not.
    put(&mut sheet, "C1", 12, n(1));
    put(&mut sheet, "D1", -1, n(9));
    assert_eq!(get(&sheet, "C1", 12), n(0));
    assert_eq!(get(&sheet, "D1", 1), n(0));
    let before = serde_json::to_value(&sheet).unwrap();
    assert_eq!(
        sheet.write_attribute("E1", 99, &n(1)).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    let request = AppVariableRequest {
        app: "Spreadsheet".into(),
        name: "E1".into(),
        indices: vec![PortableValue::Real(Real::parse("3.5").unwrap())],
        value: Some(n(1)),
    };
    assert_eq!(
        sheet
            .access(&request, &snapshot(), cancel())
            .unwrap_err()
            .kind,
        ErrorKind::BadArgType
    );
    assert_eq!(serde_json::to_value(&sheet).unwrap(), before);
}
#[test]
fn original_format_domains_ignore_invalid_values_and_preserve_typed_data() {
    let fixture = controls();
    for case in fixture["format_bounds"].as_array().unwrap() {
        let field = case["field"].as_i64().unwrap();
        let maximum = case["maximum"].as_i64().unwrap();
        let mut sheet = Spreadsheet::default();
        put(&mut sheet, "A1", 0, n(7));
        for value in -1..=maximum {
            put(&mut sheet, "A1", field, n(value));
            assert_eq!(
                get(&sheet, "A1", field),
                n(value),
                "field{field} value{value}"
            );
        }
        put(&mut sheet, "A1", field, n(1));
        for invalid in [
            n(-2),
            n(maximum + 1),
            PortableValue::Real(Real::parse("1.5").unwrap()),
            text("bad"),
            PortableValue::List(vec![n(1)]),
        ] {
            put(&mut sheet, "A1", field, invalid);
            assert_eq!(get(&sheet, "A1", field), n(1));
            assert_eq!(get(&sheet, "A1", 0), n(7));
        }
        let word = ppl_math::HpInt::parse_literal("0:24h", ppl_math::IntBase::Hex).unwrap();
        put(&mut sheet, "A1", field, PortableValue::Integer(word));
        assert_eq!(get(&sheet, "A1", field), n(0));
    }
    for field in [6, 7] {
        let mut sheet = Spreadsheet::default();
        for case in fixture["colors"].as_array().unwrap() {
            put(&mut sheet, "A1", field, n(case["raw"].as_i64().unwrap()));
            assert_eq!(
                get(&sheet, "A1", field),
                n(case["getter"].as_i64().unwrap())
            );
        }
        put(&mut sheet, "A1", field, n(255));
        for invalid in [n(-2), n(4_294_967_296), text("bad")] {
            put(&mut sheet, "A1", field, invalid);
            assert_eq!(get(&sheet, "A1", field), n(255));
        }
    }
}
#[test]
fn original_record_prefixes_stop_after_rejected_member_but_keep_earlier_updates() {
    for case in controls()["prefix_cases"].as_array().unwrap() {
        let mut sheet = Spreadsheet::default();
        put(&mut sheet, "A1", -1, control_value(&case["baseline"]));
        put(&mut sheet, "A1", -1, control_value(&case["prefix"]));
        assert_eq!(
            get(&sheet, "A1", -1),
            control_value(&case["expected"]),
            "{}",
            case["name"]
        );
    }
    let mut sheet = Spreadsheet::default();
    put(
        &mut sheet,
        "A1",
        -1,
        PortableValue::List(vec![n(7), text("TEST"), n(1), n(2)]),
    );
    put(&mut sheet, "A1", -1, PortableValue::List(vec![n(9)]));
    assert_eq!(get(&sheet, "A1", 0), n(9));
    assert_eq!(get(&sheet, "A1", 2), text("TEST"));
    assert_eq!(get(&sheet, "A1", 4), n(2));
    let before = get(&sheet, "A1", -1);
    put(&mut sheet, "A1", -1, PortableValue::List(vec![]));
    assert_eq!(get(&sheet, "A1", -1), before);
    // Extra items beyond the documented eleven local record fields are ignored.
    let mut all = match before {
        PortableValue::List(values) => values,
        _ => panic!(),
    };
    all.push(n(123));
    put(&mut sheet, "A1", -1, PortableValue::List(all));
    assert_eq!(get(&sheet, "A1", 12), n(0));
}
#[test]
fn original_single_name_empty_is_noop_minus_one_clears_and_new_names_allow_eight_ascii() {
    let mut sheet = Spreadsheet::default();
    put(&mut sheet, "A1", 0, n(23));
    let fixture = controls();
    for alias in fixture["name_controls"]["accepted"].as_array().unwrap() {
        let alias = alias.as_str().unwrap();
        put(&mut sheet, "A1", 2, text(alias));
        assert_eq!(get(&sheet, "A1", 2), text(alias));
        assert_eq!(get(&sheet, alias, 0), n(23));
    }
    for alias in ["SHEETNAME", "bad name", "A1", "Row", ""] {
        put(&mut sheet, "A1", 2, text(alias));
        assert_eq!(get(&sheet, "A1", 2), text("foo_bar"));
    }
    put(&mut sheet, "A1", 2, n(-1));
    assert_eq!(get(&sheet, "A1", 2), text(""));
    // Previously saved/UI-created long names stay readable and valid.
    sheet.set_name("LegacyLongName", Block::cell(1, 1)).unwrap();
    let restored: Spreadsheet =
        serde_json::from_value(serde_json::to_value(&sheet).unwrap()).unwrap();
    restored.validate().unwrap();
    assert_eq!(get(&restored, "LegacyLongName", 0), n(23));
}
#[test]
fn original_local_getters_and_effective_defaults_resolve_each_scope_before_inheriting() {
    let mut sheet = Spreadsheet::default();
    sheet
        .apply_format(
            Block::cell(2, 1),
            CellFormat {
                font: Some(10),
                ..Default::default()
            },
        )
        .unwrap();
    sheet
        .apply_format(
            Block {
                left: 2,
                right: 2,
                top: 1,
                bottom: MAX_ROWS,
            },
            CellFormat {
                font: Some(22),
                color: Some(255),
                ..Default::default()
            },
        )
        .unwrap();
    sheet
        .apply_format(
            Block {
                left: 1,
                right: MAX_COLUMNS,
                top: 2,
                bottom: 2,
            },
            CellFormat {
                font: Some(18),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(sheet.format(2, 1).font, Some(10));
    assert_eq!(sheet.format(2, 2).font, Some(22));
    assert_eq!(sheet.format(1, 2).font, Some(18));
    assert_eq!(get(&sheet, "B2", 5), n(-1));
    put(&mut sheet, "B1", 5, n(-1));
    assert_eq!(sheet.local_format(2, 1).font, Some(0));
    assert_eq!(sheet.format(2, 1).font, Some(22));
    // A newer column Auto clears the old column override; row default wins.
    sheet
        .apply_format(
            Block {
                left: 2,
                right: 2,
                top: 1,
                bottom: MAX_ROWS,
            },
            CellFormat {
                font: Some(0),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(sheet.format(2, 2).font, Some(18));
    assert_eq!(sheet.format(2, 1).font, Some(0));
    let whole = Block {
        left: 1,
        right: MAX_COLUMNS,
        top: 1,
        bottom: MAX_ROWS,
    };
    assert_eq!(sheet.format(3, 3).textbook, Some(2));
    sheet
        .apply_format(
            whole,
            CellFormat {
                textbook: Some(1),
                number: Some(1),
                digits: Some(6),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(sheet.format(3, 3).textbook, Some(1));
    assert_eq!(get(&sheet, "C3", 11), n(-1));
    put(&mut sheet, "C3", 11, n(1));
    assert_eq!(sheet.format(3, 3).textbook, Some(2));
    put(&mut sheet, "C3", 11, n(-1));
    assert_eq!(sheet.local_format(3, 3).textbook, Some(0));
    assert_eq!(sheet.format(3, 3).textbook, Some(1));
    put(&mut sheet, "C3", 3, n(-1));
    assert_eq!(get(&sheet, "C3", 3), n(-1));
    assert_eq!(sheet.format(3, 3).number, Some(1));
    assert_eq!(sheet.format(3, 3).digits, Some(6));
    put(&mut sheet, "B1", 6, n(-1));
    assert_eq!(sheet.local_format(2, 1).color, Some(AUTO_COLOR));
    assert_eq!(sheet.format(2, 1).color, Some(255));
}
#[test]
fn legacy_auto_color_survives_read_roundtrip_and_lazy_upgrade_without_hiding_new_black() {
    let mut sheet:Spreadsheet=serde_json::from_value(serde_json::json!({"cells":{"A1":"7"},"formats":[{"block":{"left":1,"right":1,"top":1,"bottom":1},"style":{"color":16777216,"fill":16777216}},{"block":{"left":2,"right":2,"top":1,"bottom":1},"style":{"color":16711680}}]})).unwrap();
    sheet.validate().unwrap();
    let untouched = serde_json::to_value(&sheet).unwrap();
    assert_eq!(untouched["formats"][0]["style"]["color"], 16777216);
    assert!(untouched.get("format_encoding").is_none());
    assert_eq!(sheet.local_format(1, 1).color, Some(AUTO_COLOR));
    assert_eq!(get(&sheet, "A1", 6), n(-1));
    assert_eq!(sheet.format(1, 1).color, Some(0));
    assert_eq!(sheet.format(1, 1).fill, Some(0xffffff));
    assert_eq!(sheet.format(2, 1).color, Some(0xff0000));
    assert_eq!(
        serde_json::to_value(&sheet).unwrap(),
        untouched,
        "Reads never migrate user data"
    );
    // A new legitimate high-byte black is applied only after old rules migrate.
    sheet
        .apply_format(
            Block::cell(3, 1),
            CellFormat {
                fill: Some(0x01000000),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(get(&sheet, "C1", 7), n(0x01000000));
    assert_eq!(sheet.format(3, 1).fill, Some(0x01000000));
    assert_eq!(get(&sheet, "A1", 6), n(-1));
    assert_eq!(get(&sheet, "A1", 0), n(7));
    assert_eq!(sheet.format(2, 1).color, Some(0xff0000));
    let image = serde_json::to_value(&sheet).unwrap();
    assert_eq!(image["format_encoding"], 1);
    assert_eq!(image["formats"][0]["style"]["color"], u64::from(AUTO_COLOR));
    let restored: Spreadsheet = serde_json::from_value(image).unwrap();
    assert_eq!(restored.format(3, 1).fill, Some(0x01000000));
    assert_eq!(get(&restored, "A1", 6), n(-1));
}
#[test]
fn worksheet_attribute_formulas_read_the_same_local_records_without_value_cycles() {
    let mut sheet = Spreadsheet::default();
    put(&mut sheet, "A1", 0, n(7));
    put(&mut sheet, "A1", 5, n(2));
    put(&mut sheet, "A1", 2, text("NAMED"));
    for name in ["B1", "B2"] {
        sheet.set(name, "=A1(5)").unwrap();
    }
    sheet.set("B3", "=NAMED(3+2)").unwrap();
    sheet.set("B4", "=A1(Row+1)").unwrap();
    for name in ["B1", "B2", "B3", "B4"] {
        assert_eq!(
            sheet
                .evaluate_in(name, &snapshot(), cancel())
                .unwrap()
                .portable_value(),
            Some(n(2))
        );
    }
    put(&mut sheet, "A1", 5, n(-1));
    assert_eq!(
        sheet
            .evaluate_in("B1", &snapshot(), cancel())
            .unwrap()
            .portable_value(),
        Some(n(-1))
    );
    sheet.set("C1", "=A1(-1)").unwrap();
    let record = sheet
        .evaluate_in("C1", &snapshot(), cancel())
        .unwrap()
        .portable_value()
        .unwrap();
    assert_eq!(record, get(&sheet, "A1", -1));
    sheet.set("C2", "=C2(3)").unwrap();
    assert_eq!(
        sheet
            .evaluate_in("C2", &snapshot(), cancel())
            .unwrap()
            .portable_value(),
        Some(n(-1))
    );
    sheet.set("C3", "=D5(1)").unwrap();
    assert_eq!(
        sheet
            .evaluate_in("C3", &snapshot(), cancel())
            .unwrap()
            .portable_value(),
        Some(n(-1))
    );
    sheet.set("C4", "=A1(3.5)").unwrap();
    let before = serde_json::to_value(&sheet).unwrap();
    assert!(sheet.evaluate_in("C4", &snapshot(), cancel()).is_err());
    assert_eq!(serde_json::to_value(&sheet).unwrap(), before);
}
#[test]
fn qualified_assignment_preserves_worksheet_ranges_when_copying_formula_content() {
    let mut sheet = Spreadsheet::default();
    sheet.set("A3", "=SUM(B1:B2)").unwrap();
    put(&mut sheet, "B1", 0, n(2));
    put(&mut sheet, "B2", 0, n(4));
    let formula = get(&sheet, "A3", 1);
    put(&mut sheet, "D1", 1, formula.clone());
    assert_eq!(get(&sheet, "D1", 1), formula);
    assert_eq!(get(&sheet, "D1", 0), n(6));
    put(&mut sheet, "B1", 0, n(5));
    assert_eq!(get(&sheet, "D1", 0), n(9));
}

#[test]
fn symbolic_setters_retain_formulas_while_entered_matrices_keep_typed_structure() {
    let mut sheet = Spreadsheet::default();
    let formula = PortableValue::Symbolic(ppl_cas::CasValue::Symbolic("1/3".into()));
    put(&mut sheet, "A1", 0, formula.clone());
    put(&mut sheet, "B1", 1, formula.clone());
    assert_eq!(get(&sheet, "A1", 1), formula);
    assert_eq!(get(&sheet, "B1", 1), formula);
    assert!(!sheet.entered_values.contains_key("A1"));
    put(
        &mut sheet,
        "C1",
        0,
        PortableValue::Symbolic(ppl_cas::CasValue::Rational {
            numerator: "1".into(),
            denominator: "3".into(),
        }),
    );
    assert!(!sheet.entered_values.contains_key("C1"));
    assert_eq!(get(&sheet, "C1", 0), get(&sheet, "A1", 0));
    assert!(matches!(get(&sheet, "C1", 1), PortableValue::Symbolic(_)));
    let value = Spreadsheet::default()
        .enter("A1", "[[1,2],[3,4]]", &snapshot(), cancel())
        .unwrap();
    let original = get(&value, "A1", 0);
    let cell = value.evaluate_in("A1", &snapshot(), cancel()).unwrap();
    assert!(matches!(cell, CellValue::Object { .. }));
    assert_eq!(cell.portable_value(), Some(original));
    let restored: Spreadsheet =
        serde_json::from_value(serde_json::to_value(&value).unwrap()).unwrap();
    assert_eq!(
        restored
            .evaluate_in("A1", &snapshot(), cancel())
            .unwrap()
            .portable_value(),
        cell.portable_value()
    );
}

#[test]
fn worksheet_functions_read_scoped_geometry_settings_but_cannot_write_them() {
    let mut context = ppl_vm::SessionContext::default();
    let mut settings = context.geometry_plot_settings();
    settings.pix_size = Real::parse("0.2").unwrap();
    context.set_geometry_plot_settings(settings).unwrap();
    context
        .store_program(
            "GREAD",
            "EXPORT GREAD() BEGIN RETURN Geometry.PixSize; END;".into(),
        )
        .unwrap();
    context
        .store_program(
            "GWRITE",
            "EXPORT GWRITE() BEGIN Geometry.PixSize:=0.3; RETURN 9; END;".into(),
        )
        .unwrap();
    let snapshot = context.snapshot();
    let mut sheet = Spreadsheet::default();
    sheet.set("A1", "=GREAD()").unwrap();
    assert_eq!(
        sheet
            .evaluate_in("A1", &snapshot, cancel())
            .unwrap()
            .portable_value(),
        Some(PortableValue::Real(Real::parse("0.2").unwrap()))
    );
    sheet.set("B1", "=GWRITE()").unwrap();
    let error = sheet.evaluate_in("B1", &snapshot, cancel()).unwrap_err();
    assert!(
        error.contains("attempted to change calculator memory"),
        "{error}"
    );
    assert_eq!(
        context.snapshot().geometry_plot_settings,
        snapshot.geometry_plot_settings
    );
}

#[test]
fn rejected_format_write_does_not_upgrade_or_modify_legacy_profile() {
    let rule = serde_json::json!({"block":{"left":1,"right":1,"top":1,"bottom":1},"style":{"color":16777216}});
    let mut sheet: Spreadsheet =
        serde_json::from_value(serde_json::json!({"cells":{},"formats":vec![rule;1024]})).unwrap();
    let before = serde_json::to_value(&sheet).unwrap();
    assert!(
        sheet
            .apply_format(
                Block::cell(2, 2),
                CellFormat {
                    font: Some(14),
                    ..Default::default()
                }
            )
            .is_err()
    );
    assert_eq!(serde_json::to_value(&sheet).unwrap(), before);
    assert!(
        sheet
            .apply_format(
                Block::cell(1, 1),
                CellFormat {
                    font: Some(15),
                    ..Default::default()
                }
            )
            .is_err()
    );
    assert_eq!(serde_json::to_value(&sheet).unwrap(), before);
}
