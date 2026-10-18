# hemx

Application-facing facade for checked hypermedia with Rust and Hemplate.

Use this crate for handler macros, generated resource APIs, and effect types.
Axum applications normally pair it with `hemx-build` and `hemx-axum`.

Generated collection targets declare their complete content (`set`) or a
transition between two key orders (`transition`); generated row targets replace
one existing row. At the advanced boundary, `Slot::rows_html(order, rows)` and
`Slot::rows_delta_html(before, after, rows)` build the same declarations from
rendered HTML.

Advanced integrations encode batches with the fallible
`EffectBatch::to_wire() -> Result<Vec<u8>, WireError>`. Wire ABI 3 carries
idempotent `Patch`, `Rows` and `RowsDelta` declarations plus `Focus`, `Scroll`,
`Visit`, and `Dispatch` side effects; `fold_batches` concatenates declaration-only
batches without dropping intermediate operations. Batches
are limited to 1,048,576 bytes, individual strings/byte fields to 262,144 bytes,
and effect vectors to 1,024 entries. `hemx-axum` handles encoding failures as
HTTP 500 or SSE stream errors instead of sending invalid batches.

## License

MIT
