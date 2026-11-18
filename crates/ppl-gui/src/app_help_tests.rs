//! Installed Help integration. Without an explicitly selected archive this
//! resource-dependent test returns; cloud tests cover the public reference UI.
use super::*;

fn rendered(app: &mut IdeDesktopApp, ctx: &egui::Context, label: &str) -> bool {
    frame(app, ctx, vec![])
        .shapes
        .iter()
        .any(|shape| text_rect(&shape.shape, label).is_some())
}
#[test]
fn installed_help_search_mouse_keyboard_history_and_restored_selection_work_together() {
    let Some(path) = std::env::var_os("PPL_PRIME_HELP_RESOURCE") else {
        return;
    };
    let book = ppl_engine::help::Book::load_resource(
        std::path::Path::new(&path),
        ppl_engine::help::Language::English,
    )
    .unwrap();
    let entry = |name: &str| book.topics.iter().find(|entry| entry.name == name).unwrap();
    let title = |name: &str| String::from_utf16_lossy(&entry(name).spans[0]);
    let sin_label = format!("SIN — {}", title("SIN"));
    let cos_label = format!("COS — {}", title("COS"));
    let (mut app, ctx) = fixture(Page::Help);
    app.native_tools.memory.help_query = "SIN".into();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !rendered(&mut app, &ctx, &sin_label) {
        assert!(Instant::now() < deadline, "installed Help did not load");
        std::thread::sleep(Duration::from_millis(2));
    }
    click(&mut app, &ctx, &sin_label);
    assert_eq!(app.native_tools.memory.help_topic, "SIN");
    click(&mut app, &ctx, "SIN");
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            },
        }],
    );
    frame(&mut app, &ctx, vec![egui::Event::Text("COS".into())]);
    assert_eq!(app.native_tools.memory.help_query, "COS");
    click(&mut app, &ctx, &cos_label);
    assert_eq!(app.native_tools.memory.help_topic, "COS");
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![picker_key(egui::Key::ArrowLeft)]);
    assert_eq!(app.native_tools.memory.help_topic, "SIN");
    frame(&mut app, &ctx, vec![picker_key(egui::Key::ArrowRight)]);
    assert_eq!(app.native_tools.memory.help_topic, "COS");
    for _ in 0..5 {
        frame(&mut app, &ctx, vec![picker_key(egui::Key::ArrowDown)]);
    }
    for _ in 0..5 {
        frame(&mut app, &ctx, vec![]);
    }
    assert!(
        app.native_tools.help_snapshot()["scroll_y"]
            .as_f64()
            .unwrap()
            > 0.,
        "keyboard scrolling did not move the Help document"
    );
    assert!(
        app.state.native.home_input.is_empty(),
        "Help navigation edited Home"
    );
    let saved = serde_json::to_value(&app.native_tools.memory).unwrap();
    assert_eq!(saved["help"]["current"]["id"], entry("COS").id);
    let (mut reopened, reopened_ctx) = fixture(Page::Help);
    reopened.native_tools.memory = serde_json::from_value(saved).unwrap();
    reopened.native_tools.memory.validate().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !rendered(&mut reopened, &reopened_ctx, &title("COS")) {
        assert!(
            Instant::now() < deadline,
            "restored installed topic did not render"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(reopened.native_tools.memory.help_query, "COS");
    assert_eq!(reopened.native_tools.memory.help_topic, "COS");
    assert_eq!(
        serde_json::to_value(&reopened.native_tools.memory).unwrap()["help"]["current"]["id"],
        entry("COS").id
    );
}
