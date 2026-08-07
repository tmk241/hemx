use hemx_core::{Effect, GeneratedTarget, Payload, ResourceId, ResourceKind, ResourceRef};

fn panic_text<T>(result: std::thread::Result<T>) -> String {
    let panic = match result {
        Ok(_) => panic!("operation must panic"),
        Err(panic) => panic,
    };
    if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = panic.downcast_ref::<&str>() {
        (*message).to_owned()
    } else {
        panic!("panic payload was not text")
    }
}

#[test]
fn inspects_complete_documents_with_owned_structure() {
    let inspected = hemx_test::inspect_html_document(
        r#"<!doctype html>
        <html lang="en">
          <head><title>Todos &amp; notes</title></head>
          <body><main><h1> Todos   today </h1><p data-state="ready">two &lt; three</p></main></body>
        </html>"#,
    );

    inspected.assert_count("html", 1);
    inspected.assert_text("title", "Todos & notes");
    inspected.assert_text("main > h1", "Todos today");
    inspected.assert_attribute("p[data-state]", "data-state", "ready");

    let paragraph = inspected.select("main p").unwrap();
    assert_eq!(paragraph.elements()[0].name(), "p");
    assert_eq!(paragraph.elements()[0].text(), "two < three");
    assert_eq!(
        paragraph.elements()[0].attribute("data-state"),
        Some("ready")
    );
    assert!(paragraph.elements()[0]
        .attributes()
        .any(|attribute| attribute == ("data-state", "ready")));
}

#[test]
fn inspects_generated_targets_and_handles_without_raw_runtime_ids() {
    let target = TestTarget(ResourceKind::Slot, 42);
    let handle = hemx_core::Handle::<()>::new(7);
    let inspected = hemx_test::inspect_html_fragment(
        r#"<section data-sid="42"><button data-hid="7">Save</button></section>"#,
    );

    inspected.assert_target(target);
    inspected.assert_handle(handle);
    assert_eq!(
        inspected.select_target(target).elements()[0].name(),
        "section"
    );
    assert_eq!(inspected.select_handle(handle).elements()[0].text(), "Save");
}

#[test]
fn inspects_fragments_and_keeps_selections_owned() {
    let selection = {
        let inspected = hemx_test::inspect_html_fragment(
            r#"<ul><li class="todo">one</li><li class="todo">two</li></ul>"#,
        );
        inspected.assert_exists("ul > li.todo");
        inspected.assert_count("ul > li.todo", 2);
        inspected.select("li.todo").unwrap()
    };

    assert_eq!(selection.selector(), "li.todo");
    assert_eq!(selection.len(), 2);
    assert!(!selection.is_empty());
    assert_eq!(selection.elements()[0].text(), "one");
    assert_eq!(selection.elements()[1].text(), "two");
}

#[test]
fn inspects_generated_target_document_and_fragment_payloads() {
    let document_target = TestTarget(ResourceKind::Slot, 1);
    let document_effect = Effect::Put {
        target: ResourceRef::unscoped(document_target.__hemx_resource_id()),
        payload: Payload::Html(
            "<!doctype html><html><body><main id=app>ready</main></body></html>".into(),
        ),
    };
    let document = hemx_test::inspect(document_effect)
        .target_html_document(document_target)
        .unwrap();
    document.assert_text("main#app", "ready");
    assert!(document.origin().contains("Put HTML effect"));

    for (effect, operation) in [
        (
            Effect::Insert {
                target: ResourceRef::unscoped(ResourceId::new(ResourceKind::Slot, 2)),
                key: "later".into(),
                payload: Payload::Html("<li data-key=later>later</li>".into()),
            },
            "Insert",
        ),
        (
            Effect::Prepend {
                target: ResourceRef::unscoped(ResourceId::new(ResourceKind::Slot, 2)),
                key: "first".into(),
                payload: Payload::Html("<li data-key=first>first</li>".into()),
            },
            "Prepend",
        ),
    ] {
        let fragment = hemx_test::inspect(effect)
            .target_html_fragment(TestTarget(ResourceKind::Slot, 2))
            .unwrap();
        fragment.assert_count("li[data-key]", 1);
        assert!(fragment.origin().contains(operation));
    }
}

#[test]
fn target_html_errors_explain_missing_non_html_and_ambiguous_effects() {
    let missing = hemx_test::inspect(Effect::Put {
        target: ResourceRef::unscoped(ResourceId::new(ResourceKind::Slot, 9)),
        payload: Payload::Html("<p>other</p>".into()),
    })
    .target_html_fragment(TestTarget(ResourceKind::Slot, 1))
    .unwrap_err()
    .to_string();
    assert!(missing.contains("generated target"), "{missing}");
    assert!(missing.contains("found none"), "{missing}");
    assert!(missing.contains("all effects"), "{missing}");
    assert!(missing.contains("other"), "{missing}");

    let non_html = hemx_test::inspect(Effect::Put {
        target: ResourceRef::unscoped(ResourceId::new(ResourceKind::Slot, 1)),
        payload: Payload::Text("plain text".into()),
    })
    .target_html_fragment(TestTarget(ResourceKind::Slot, 1))
    .unwrap_err()
    .to_string();
    assert!(non_html.contains("found none"), "{non_html}");
    assert!(non_html.contains("plain text"), "{non_html}");

    let ambiguous = hemx_test::inspect(vec![
        Effect::Put {
            target: ResourceRef::unscoped(ResourceId::new(ResourceKind::Slot, 1)),
            payload: Payload::Html("<p>one</p>".into()),
        },
        Effect::Insert {
            target: ResourceRef::unscoped(ResourceId::new(ResourceKind::Slot, 1)),
            key: "two".into(),
            payload: Payload::Html("<p>two</p>".into()),
        },
    ])
    .target_html_fragment(TestTarget(ResourceKind::Slot, 1))
    .unwrap_err()
    .to_string();
    assert!(ambiguous.contains("found 2"), "{ambiguous}");
    assert!(ambiguous.contains("cannot choose"), "{ambiguous}");
    assert!(ambiguous.contains("one"), "{ambiguous}");
    assert!(ambiguous.contains("two"), "{ambiguous}");
}

#[test]
fn selector_and_assertion_failures_are_actionable() {
    let inspected = hemx_test::inspect_html_fragment(
        r#"<section><p class="actual">first</p><p class="actual">second</p></section>"#,
    );

    let invalid = inspected.select("section[").unwrap_err().to_string();
    assert!(invalid.contains("invalid CSS selector"), "{invalid}");
    assert!(invalid.contains("section["), "{invalid}");
    assert!(invalid.contains("HTML fragment"), "{invalid}");

    let missing = panic_text(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || inspected.assert_exists("article.missing"),
    )));
    assert!(missing.contains("article.missing"), "{missing}");
    assert!(missing.contains("found none"), "{missing}");
    assert!(missing.contains("class=\"actual\""), "{missing}");

    let duplicate = panic_text(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || inspected.assert_text("p.actual", "first"),
    )));
    assert!(duplicate.contains("exactly one"), "{duplicate}");
    assert!(duplicate.contains("found 2"), "{duplicate}");
    assert!(duplicate.contains("first"), "{duplicate}");
    assert!(duplicate.contains("second"), "{duplicate}");

    let wrong_attribute = panic_text(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || inspected.assert_attribute("p:first-child", "class", "expected"),
    )));
    assert!(wrong_attribute.contains("data") || wrong_attribute.contains("class"));
    assert!(wrong_attribute.contains("expected"), "{wrong_attribute}");
    assert!(wrong_attribute.contains("actual"), "{wrong_attribute}");
}

#[derive(Clone, Copy)]
struct TestTarget(ResourceKind, u32);

impl GeneratedTarget for TestTarget {
    fn __hemx_resource_id(self) -> ResourceId {
        ResourceId::new(self.0, self.1)
    }
}
