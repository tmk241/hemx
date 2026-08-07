# hemx-test

Test support for Hemx applications and generated resources.

`hemx-test` keeps assertions at the same typed boundary as application code. Use
an `EffectInspector` with generated targets instead of copying resource IDs or
matching raw wire operations.

```rust
use hemx_core::{GeneratedTarget, SafeHtml, Slot};
# use hemx_core::{Effect, ResourceId};
# #[derive(Clone, Copy)]
# struct GeneratedTodos(Slot<()>);
# impl GeneratedTarget for GeneratedTodos {
#     fn __hemx_resource_id(self) -> ResourceId { self.0.id() }
# }
# impl GeneratedTodos {
#     fn html(self, value: SafeHtml) -> Effect { self.0.html(value) }
# }

// Application build output supplies generated target types like this one.
let todos = GeneratedTodos(Slot::new(7));
let effect = todos.html(SafeHtml::trusted(
    r#"<ul><li class="todo" data-state="open">Write tests</li></ul>"#,
));

let inspected = hemx_test::inspect(effect);
assert!(inspected.updates_html(todos));

let html = inspected.target_html_fragment(todos)?;
html.assert_count("li.todo", 1);
html.assert_text("li.todo", "Write tests");
html.assert_attribute("li.todo", "data-state", "open");
# Ok::<(), hemx_test::HtmlInspectionError>(())
```

Complete server-rendered pages can be inspected directly:

```rust
let page = hemx_test::inspect_html_document(
    "<!doctype html><html><body><main id=app>Ready</main></body></html>",
);
page.assert_text("main#app", "Ready");
```

Structural inspection is backed internally by an HTML parser, but its types are
not part of the public API. `HtmlInspector`, `HtmlSelection`, and `HtmlElement`
own the observable source and selected data. Structural inspection does not run
JavaScript or prove browser-owned behavior such as focus, history, layout, or
runtime reconciliation.

A target inspector requires exactly one `Put`, `Insert`, or `Prepend` HTML
payload for that generated target. Missing, non-HTML, and ambiguous target
effects return `HtmlInspectionError` with the relevant effects in the
diagnostic.

## License

MIT
