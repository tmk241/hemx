# hemx-build

Build-time Hemplate analysis and typed resource generation for Hemx applications.

Call `hemx_build::app().template_dir("templates").run()` from `build.rs`, then
include `hemx.generated.rs` from `OUT_DIR` through `#[hemx::surface]`. Resources
are exported under their template's generated module. `.global_exports(true)`
also exports application-wide aliases.

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
not declare a collection. A list tag or descendant loop never turns an ordinary
slot into a collection.

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





All generated rendering helpers use one complete application lowering table,
including nested child and grandchild resources and handles. The destination
slot's name does not select a component-specific table. Authoring names must
remain globally unique across templates for each resource kind; ambiguous names
are rejected, never guessed or assigned fallback IDs.

Resource IDs remain deterministic from resource kind and template/name. Slot
role is part of the semantic build fingerprint, so changing an ordinary slot to
a row or collection changes compatibility even when its resource ID is stable.
Unchanged inputs and generator contracts produce byte-identical artifacts, and
no-op builds do not rewrite them.

## License

MIT
