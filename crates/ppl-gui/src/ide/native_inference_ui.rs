//! Documented HP Inference app choices; preserved Native extensions are labeled.
//! Only validated JSON drafts mutate saved data.
use super::native_inference::{
    Alternative, InferenceMode, InferenceRequest, MeanSummary, ProportionSample, RegressionTarget,
};
use serde::{Serialize, de::DeserializeOwned};

const KINDS: [&str; 6] = [
    "Z: one mean",
    "Z: two means",
    "t: one mean",
    "t: two means",
    "Z: one proportion",
    "Z: two proportions",
];
fn kind(request: &InferenceRequest) -> usize {
    match request {
        InferenceRequest::MeanZ { .. } => 0,
        InferenceRequest::TwoMeansZ { .. } => 1,
        InferenceRequest::MeanT { .. } => 2,
        InferenceRequest::TwoMeansT { .. } => 3,
        InferenceRequest::OneProportion { .. } => 4,
        InferenceRequest::TwoProportions { .. } => 5,
        InferenceRequest::VarianceRatio { .. } => 6,
        InferenceRequest::GoodnessOfFit { .. } => 7,
        InferenceRequest::Contingency { .. } => 8,
        InferenceRequest::Anova { .. } => 9,
        InferenceRequest::Regression { .. } => 10,
    }
}
fn selected_label(request: &InferenceRequest) -> String {
    let index = kind(request);
    if let Some(name) = KINDS.get(index) {
        return (*name).into();
    }
    let name = match request {
        InferenceRequest::VarianceRatio { .. } => "F variance ratio",
        InferenceRequest::GoodnessOfFit { .. } => "Chi-square goodness of fit",
        InferenceRequest::Contingency { .. } => "Chi-square two-way table",
        InferenceRequest::Anova { .. } => "One-way ANOVA",
        InferenceRequest::Regression { .. } => "Regression inference",
        _ => unreachable!("documented method has a catalog label"),
    };
    format!("Native extension: {name}")
}
fn default_mode(null_value: f64) -> InferenceMode {
    InferenceMode::Test {
        null_value,
        alpha: 0.05,
        alternative: Alternative::TwoSided,
    }
}
fn summary(mean: f64) -> MeanSummary {
    MeanSummary {
        mean,
        sd: 2.,
        count: 20,
    }
}
fn default_request(index: usize) -> InferenceRequest {
    match index {
        0 => InferenceRequest::default(),
        1 => InferenceRequest::TwoMeansZ {
            first: summary(10.),
            second: summary(9.),
            mode: default_mode(0.),
        },
        2 => InferenceRequest::MeanT {
            sample: summary(10.),
            mode: default_mode(9.),
        },
        3 => InferenceRequest::TwoMeansT {
            first: summary(10.),
            second: summary(9.),
            pooled: false,
            mode: default_mode(0.),
        },
        4 => InferenceRequest::OneProportion {
            sample: ProportionSample {
                successes: 60,
                total: 100,
            },
            mode: default_mode(0.5),
        },
        5 => InferenceRequest::TwoProportions {
            first: ProportionSample {
                successes: 60,
                total: 100,
            },
            second: ProportionSample {
                successes: 45,
                total: 100,
            },
            mode: default_mode(0.),
        },
        6 => InferenceRequest::VarianceRatio {
            sd1: 2.,
            n1: 10,
            sd2: 1.,
            n2: 12,
            mode: default_mode(1.),
        },
        7 => InferenceRequest::GoodnessOfFit {
            observed: vec![10., 20., 30.],
            expected: vec![20., 20., 20.],
            estimated_parameters: 0,
            alpha: 0.05,
        },
        8 => InferenceRequest::Contingency {
            table: vec![vec![10., 20.], vec![30., 40.]],
            alpha: 0.05,
        },
        9 => InferenceRequest::Anova {
            groups: vec![vec![1., 2., 3.], vec![3., 4., 5.], vec![5., 6., 7.]],
            alpha: 0.05,
        },
        _ => InferenceRequest::Regression {
            pairs: vec![[1., 2.], [2., 4.], [3., 5.], [4., 4.], [5., 5.]],
            target: RegressionTarget::Slope,
            mode: default_mode(0.),
        },
    }
}
fn number(ui: &mut egui::Ui, label: &str, value: &mut f64) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::DragValue::new(value).speed(0.1));
    });
}
fn count(ui: &mut egui::Ui, label: &str, value: &mut u64) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::DragValue::new(value).speed(1.));
    });
}
fn summary_fields(ui: &mut egui::Ui, label: &str, sample: &mut MeanSummary, known: bool) {
    ui.group(|ui| {
        ui.label(label);
        number(ui, "Mean", &mut sample.mean);
        number(
            ui,
            if known { "Population σ" } else { "Sample s" },
            &mut sample.sd,
        );
        count(ui, "n", &mut sample.count);
    });
}
fn proportion_fields(ui: &mut egui::Ui, label: &str, sample: &mut ProportionSample) {
    ui.group(|ui| {
        ui.label(label);
        count(ui, "Successes", &mut sample.successes);
        count(ui, "n", &mut sample.total);
    });
}
fn mode_fields(ui: &mut egui::Ui, mode: &mut InferenceMode, null: f64, test_allowed: bool) {
    if !test_allowed && matches!(mode, InferenceMode::Test { .. }) {
        *mode = InferenceMode::Interval { confidence: 0.95 };
    }
    let mut interval = matches!(mode, InferenceMode::Interval { .. });
    let was = interval;
    ui.horizontal(|ui| {
        if test_allowed {
            ui.selectable_value(&mut interval, false, "Test");
        }
        ui.selectable_value(&mut interval, true, "Interval");
    });
    if interval != was {
        *mode = if interval {
            InferenceMode::Interval { confidence: 0.95 }
        } else {
            default_mode(null)
        };
    }
    match mode {
        InferenceMode::Interval { confidence } => number(ui, "Confidence", confidence),
        InferenceMode::Test {
            null_value,
            alpha,
            alternative,
        } => {
            number(ui, "Null value", null_value);
            number(ui, "α", alpha);
            let label = match alternative {
                Alternative::TwoSided => "≠",
                Alternative::Less => "<",
                Alternative::Greater => ">",
            };
            egui::ComboBox::from_id_salt("inference_alternative")
                .selected_text(format!("Alternative: {label}"))
                .show_ui(ui, |ui| {
                    ui.selectable_value(alternative, Alternative::TwoSided, "≠ null");
                    ui.selectable_value(alternative, Alternative::Less, "< null");
                    ui.selectable_value(alternative, Alternative::Greater, "> null");
                });
        }
    }
}
#[derive(Clone)]
struct JsonDraft {
    text: String,
    last_saved: String,
}
fn json_field<T: Serialize + DeserializeOwned>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut T,
) -> bool {
    let id = ui.id().with(("inference_json", label));
    let Ok(saved) = serde_json::to_string(value) else {
        ui.colored_label(egui::Color32::DARK_RED, "Cannot serialize input");
        return false;
    };
    let mut draft = ui
        .ctx()
        .data_mut(|d| d.get_temp::<JsonDraft>(id))
        .unwrap_or_else(|| JsonDraft {
            text: saved.clone(),
            last_saved: saved.clone(),
        });
    if draft.last_saved != saved {
        draft = JsonDraft {
            text: saved.clone(),
            last_saved: saved,
        };
    }
    ui.label(label);
    let response = ui.add(
        egui::TextEdit::multiline(&mut draft.text)
            .desired_rows(3)
            .id(id),
    );
    let parsed = serde_json::from_str::<T>(&draft.text);
    let valid = parsed.is_ok();
    match parsed {
        Ok(parsed) if response.changed() => {
            *value = parsed;
            draft.last_saved = serde_json::to_string(value).unwrap();
        }
        Err(error) => {
            ui.colored_label(egui::Color32::DARK_RED, format!("Invalid JSON: {error}"));
        }
        _ => {}
    }
    ui.ctx().data_mut(|d| d.insert_temp(id, draft));
    valid
}
fn method_selector(ui: &mut egui::Ui, request: &mut InferenceRequest) -> egui::Response {
    let old = kind(request);
    let mut selected = old;
    if old >= KINDS.len() {
        ui.label(selected_label(request));
        ui.label("This saved Native extension is separate from the HP Inference app methods.");
    }
    let response = egui::ComboBox::from_id_salt("inference_kind")
        .selected_text(selected_label(request))
        .show_ui(ui, |ui| {
            for (i, label) in KINDS.iter().enumerate() {
                ui.selectable_value(&mut selected, i, *label);
            }
        })
        .response;
    if old != selected {
        *request = default_request(selected);
    }
    response
}

/// Returns true only when Calculate was clicked with all data drafts valid.
/// Statistical domain errors are reported by InferenceRequest::calculate().
pub fn show(ui: &mut egui::Ui, request: &mut InferenceRequest) -> bool {
    method_selector(ui, request);
    let selected = kind(request);
    let mut valid = true;
    ui.push_id(selected, |ui| match request {
        InferenceRequest::MeanZ {
            mean,
            sigma,
            count: n,
            mode,
        } => {
            number(ui, "Sample mean", mean);
            number(ui, "Known σ", sigma);
            count(ui, "n", n);
            mode_fields(ui, mode, 0., true);
        }
        InferenceRequest::TwoMeansZ {
            first,
            second,
            mode,
        } => {
            summary_fields(ui, "Sample 1", first, true);
            summary_fields(ui, "Sample 2", second, true);
            mode_fields(ui, mode, 0., true);
        }
        InferenceRequest::MeanT { sample, mode } => {
            summary_fields(ui, "Sample", sample, false);
            mode_fields(ui, mode, 0., true);
        }
        InferenceRequest::TwoMeansT {
            first,
            second,
            pooled,
            mode,
        } => {
            summary_fields(ui, "Sample 1", first, false);
            summary_fields(ui, "Sample 2", second, false);
            ui.checkbox(pooled, "Pooled variance");
            mode_fields(ui, mode, 0., true);
        }
        InferenceRequest::OneProportion { sample, mode } => {
            proportion_fields(ui, "Sample", sample);
            mode_fields(ui, mode, 0.5, true);
        }
        InferenceRequest::TwoProportions {
            first,
            second,
            mode,
        } => {
            proportion_fields(ui, "Sample 1", first);
            proportion_fields(ui, "Sample 2", second);
            mode_fields(ui, mode, 0., true);
        }
        InferenceRequest::VarianceRatio {
            sd1,
            n1,
            sd2,
            n2,
            mode,
        } => {
            number(ui, "Sample s1", sd1);
            count(ui, "n1", n1);
            number(ui, "Sample s2", sd2);
            count(ui, "n2", n2);
            mode_fields(ui, mode, 1., true);
        }
        InferenceRequest::GoodnessOfFit {
            observed,
            expected,
            estimated_parameters,
            alpha,
        } => {
            valid &= json_field(ui, "Observed counts", observed);
            valid &= json_field(ui, "Expected counts", expected);
            ui.horizontal(|ui| {
                ui.label("Estimated parameters");
                ui.add(egui::DragValue::new(estimated_parameters));
            });
            number(ui, "α", alpha);
        }
        InferenceRequest::Contingency { table, alpha } => {
            valid &= json_field(ui, "Observed rows", table);
            number(ui, "α", alpha);
        }
        InferenceRequest::Anova { groups, alpha } => {
            valid &= json_field(ui, "Groups", groups);
            number(ui, "α", alpha);
        }
        InferenceRequest::Regression {
            pairs,
            target,
            mode,
        } => {
            valid &= json_field(ui, "[x,y] pairs", pairs);
            let (mut index, x) = match target {
                RegressionTarget::Slope => (0, 0.),
                RegressionTarget::Intercept => (1, 0.),
                RegressionTarget::MeanResponse { x } => (2, *x),
                RegressionTarget::Prediction { x } => (3, *x),
            };
            let old = index;
            let names = ["Slope", "Intercept", "Mean response", "Future response"];
            egui::ComboBox::from_id_salt("regression_target")
                .selected_text(names[index])
                .show_ui(ui, |ui| {
                    for (i, name) in names.iter().enumerate() {
                        ui.selectable_value(&mut index, i, *name);
                    }
                });
            if index != old {
                *target = match index {
                    0 => RegressionTarget::Slope,
                    1 => RegressionTarget::Intercept,
                    2 => RegressionTarget::MeanResponse { x },
                    _ => RegressionTarget::Prediction { x },
                };
            }
            if let RegressionTarget::MeanResponse { x } | RegressionTarget::Prediction { x } =
                target
            {
                number(ui, "Response at x", x);
            }
            mode_fields(ui, mode, 0., matches!(target, RegressionTarget::Slope));
        }
    });
    ui.add_enabled(valid, egui::Button::new("Calculate"))
        .clicked()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_preserved_request_form_has_a_valid_real_calculation() {
        for kind in 0..11 {
            let request = default_request(kind);
            let name = selected_label(&request);
            let result = request
                .calculate()
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!result.method.is_empty());
        }
    }
    #[test]
    fn malformed_json_draft_preserves_saved_data_and_disables_calculate() {
        let ctx = egui::Context::default();
        let mut values = vec![1., 2.];
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let id = ui.id().with(("inference_json", "Samples"));
                ui.ctx().data_mut(|d| {
                    d.insert_temp(
                        id,
                        JsonDraft {
                            text: "[1,".into(),
                            last_saved: serde_json::to_string(&values).unwrap(),
                        },
                    )
                });
                assert!(!json_field(ui, "Samples", &mut values));
            });
        });
        assert_eq!(values, [1., 2.]);
    }
    #[test]
    fn hp_catalog_contains_only_the_six_documented_methods() {
        assert_eq!(
            KINDS,
            [
                "Z: one mean",
                "Z: two means",
                "t: one mean",
                "t: two means",
                "Z: one proportion",
                "Z: two proportions"
            ]
        );
        for index in 0..6 {
            assert!(!selected_label(&default_request(index)).starts_with("Native extension:"));
        }
    }
    #[test]
    fn rendering_saved_native_extensions_preserves_all_fields_and_labels_scope() {
        for index in 6..11 {
            let ctx = egui::Context::default();
            let mut request = default_request(index);
            let before = serde_json::to_value(&request).unwrap();
            assert!(selected_label(&request).starts_with("Native extension:"));
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    assert!(!show(ui, &mut request));
                });
            });
            assert_eq!(
                serde_json::to_value(&request).unwrap(),
                before,
                "saved extension {index} must not silently become a mean test"
            );
        }
    }
    #[test]
    fn saved_extension_changes_only_after_clicking_a_documented_method() {
        fn text_position(shape: &egui::Shape, text: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(value) if value.galley.text() == text => {
                    Some(value.pos + value.galley.size() / 2.)
                }
                egui::Shape::Vec(shapes) => shapes
                    .iter()
                    .filter_map(|shape| text_position(shape, text))
                    .next_back(),
                _ => None,
            }
        }
        fn pointer(pos: egui::Pos2) -> Vec<egui::Event> {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        }
        let ctx = egui::Context::default();
        let mut request = default_request(6);
        let before = serde_json::to_value(&request).unwrap();
        let frame = |request: &mut InferenceRequest, events| {
            let mut rect = egui::Rect::NOTHING;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(640., 700.),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        rect = method_selector(ui, request).rect;
                    });
                },
            );
            (output, rect)
        };
        let (_, rect) = frame(&mut request, vec![]);
        frame(&mut request, pointer(rect.center()));
        assert_eq!(serde_json::to_value(&request).unwrap(), before);
        let (opened, _) = frame(&mut request, vec![]);
        let select_at = opened
            .shapes
            .iter()
            .find_map(|shape| text_position(&shape.shape, "Z: one mean"))
            .expect("documented method must be an actual selectable popup option");
        frame(&mut request, pointer(select_at));
        assert!(matches!(request, InferenceRequest::MeanZ { .. }));
    }
}
