# hemx-build

Build-time Hemplate analysis and typed resource generation for Hemx applications.

Call `hemx_build::app().template_dir("templates").run()` from `build.rs`, then
include the generated resources with `#[hemx::surface] pub mod heml {}`.
Resources are exported only under their template's module, such as
`heml::page::count`; they never enter the application's own scope, so a
template binding or module named like a resource cannot collide with it. The
generated file compiles only inside that surface module; a bare `include!` of
`hemx.generated.rs` fails.

A `#[hemx::form("name")]` model must match the controls inside the
`<form data-hemx-form="name">` element of the template that declares it. A model
field without such a control fails to compile with a diagnostic naming the
form, the field and that template; a control rendered by a nested view does not
belong to the outer form's contract.

## Canonical template layout

`app().run()` checks every `.heml` file recursively under the selected template
directory before generating artifacts, including when precomputed surfaces are
supplied or `OUT_DIR` is absent. Files backing supplied surfaces are checked too.
One error names all noncanonical files and the fix command:

```console
hemplate-fmt templates
```

For a custom `template_dir`, pass that directory to the formatter instead.
`hemplate-fmt --check templates` lists differences without rewriting. Hemplate's
formatter preserves rendered output, attribute order, quote style, Rust
expressions, and raw/preformatted content. Invalid templates receive source-path
and position diagnostics and are not rewritten by the formatter.

Only an explicit `.check_format(false)` disables this check:

```rust,ignore
hemx_build::app().check_format(false).run()?;
```

This opt-out affects layout checking only, not resource validation or transforms.

## Slots, rows, and collections

Declare collections explicitly, including initially empty ones:

```html
<main data-hemx-root>
  <section data-hemx-slot="summary"></section>
  <ul data-hemx-collection="rows"></ul>
</main>
```

For a template named `list.heml`, the generated capabilities are:

- `list::summary.put(&view)`, `.replace(&view)`, `.text(value)`, and `.set(value)`
  update an ordinary slot. Ordinary slots do not expose collection rows.
- `list::rows.set(&rows)` declares the collection's complete content: every
  `Hemplate + KeyedPartial` row, in order.
- `list::rows.transition(&before, &after, &changed)` changes the collection
  from the key order `before` to `after`, with the rows that are new or
  changed. Only the steps between the orders and those rows travel; retained
  rows keep their DOM nodes, focus, and unsent edits. A browser whose rows are
  not in the `before` order changes nothing and reports it.
- `list::rows.replace(row)` and `.replace_keyed(key, &view)` replace one
  existing keyed row within that collection.

Use `transition` for each change and `set` whenever a client may have missed
one: on a new or reconnected stream, or after a lagging broadcast. A delta
already at its resulting order is skipped (when its starting order differs).
Do not drop intermediate declarations: even if the final HTML returns to the
same defaults, an intervening change can affect unsent form values.

A `data-hemx-slot` inside a keyed `h-for`, or on an element carrying
`data-hemx-key` (including `+data-hemx-key`), declares a **keyed row**. A row
supports keyed replacement only; membership and order belong to its collection.
For example:

```html
<template h-for="row in &self.rows" h-key="row.id">
  <li data-hemx-slot="row_body">{+ row.title +}</li>
</template>
```

This declares `row_body.replace(row)` and `.replace_keyed(key, &view)`. It does
not declare a collection.

For a keyed row, `replace` reconciles the row itself when the view renders
one element carrying that row's `data-hemx-key`; it never nests that wrapper.
The same applies to keyed patches addressed through a collection. An explicit
row resource also accepts content-only HTML, which morphs the existing row's
content and leaves its host in place. Ordinary, unkeyed slots always morph
content. A list tag or descendant loop never turns an ordinary slot into a
collection.

An element may declare only one of `data-hemx-slot`, `data-hemx-collection`,
and `data-hemx-form`: they lower to the same `data-hemx-resource` marker.
Use separate elements when different resource capabilities are needed.
`data-hemx-handle` lowers to `data-hemx-action`, so an action and form resource
may share an element. A plain slot is not rejected merely for being on an input
or form host.

Handler parameters are explicit. `data-hemx-param-todo-id="7"` on a handled
element declares the parameter `todo_id` and sends it as a request field; other
`data-*` attributes are metadata and are neither parameters nor request fields.
Generated action forms also carry the page's build fingerprint as a hidden `__b`
field. `data-hemx-ws` is rejected: Hemx streams server updates with
`data-hemx-sse`.

Collections require non-void elements. Reusing one resource name with different
roles is an error, rather than an implicit upgrade. Repeated collection instances
may live under separate roots; the browser runtime validates destination
uniqueness within the owning root.

Collection lowering emits `data-hemx-resource="ID" data-hemx-collection=""`.
The collection marker is retained for runtime validation. A `Rows` declaration
addresses the collection resource and carries row keys separately; explicit row
resources retain their own generated IDs.

## Composed rendering and determinism

Lowering happens while templates compile. `run()` writes one Hemplate transform
per template into `OUT_DIR` and points the derive macro at it with
`cargo:rustc-env`. The transform replaces authored markers with runtime markers
and inserts the hidden `__h` / `__b` fields into handled forms, using the
complete application table, so nested child and grandchild templates are final
wherever they render. `template_dir` may select `templates/` or one of its
subdirectories under the package root. Hemplate's derive does not read Hemx
transforms for files outside that tree; `run()` rejects those files with a
source-path diagnostic. Move them under `templates/` and use Hemplate's
`#[hemplate = "subdirectory"]` attribute when needed.

Nothing rewrites rendered HTML: interpolated text and
trusted raw HTML keep any marker-like text unchanged. Resource names must be
static; `+data-hemx-slot` and the other dynamic spellings are rejected. A
handled form needs a closing tag (`<form ...></form>`) to hold its fields; a
self-closing `<form .../>` is rejected.
Authoring names must remain globally unique across templates for each resource
kind; ambiguous names are rejected, never guessed or assigned fallback IDs.

Resource IDs remain deterministic from resource kind and template/name. Slot
role is part of the semantic build fingerprint, so changing an ordinary slot to
a row or collection changes compatibility even when its resource ID is stable.
Unchanged inputs and generator contracts produce byte-identical artifacts, and
no-op builds do not rewrite them.

## License

MIT
