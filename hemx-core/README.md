# hemx-core

Core effect types, protocol values, validation, and runtime contracts for Hemx.
Most applications should depend on the `hemx` facade instead.

## Wire contract

`EffectBatch::to_wire()` returns `Result<Vec<u8>, WireError>`. ABI version 3
declares state. `Patch` sets one resource's HTML. `Rows` declares a
collection's complete key order followed by the new or changed rows as
(key, HTML) pairs; keys absent from the order are removed, retained rows keep
their DOM nodes. Order keys are unique (at most `MAX_ROW_KEYS`, 65,536), and
every supplied row appears once in the order. `Focus`, `Scroll`, `Visit`, and
`Dispatch` are side effects. Earlier ABIs are rejected, not guessed or
translated.

`RowsDelta` changes a collection from one key order to another. Orders are
named by `order_hash`: FNV-1a 64 over each key's UTF-8 bytes followed by
`0xFF`, which the browser runtime computes identically. A delta carries the
`from` and `to` hashes, a sequence of `RowStep`s (remove a row, or place a row
before another or at the end), and the HTML of new or changed rows. A client
applies it only when its rows are in the `from` order, skips it when they are
already in the `to` order, and otherwise rejects it without changing
anything, so a missed update is detected rather than silently compounded. It
is as idempotent as `Rows` while its size follows the change, not the
collection. `row_steps(before, after)` computes the steps: a shared prefix and
suffix cost only comparisons, removals come first, and only rows outside a
longest increasing run move. At most `MAX_ROW_KEYS` steps and supplied rows.
`order_hash` detects divergence; it is not a cryptographic commitment, and a
missed change to a row's content alone is not detected.

Both encoding and decoding enforce `MAX_BATCH_BYTES` (1,048,576 bytes including
the envelope), `MAX_FIELD_BYTES` (262,144 bytes per string or byte field), and
`MAX_EFFECTS` (1,024). Every nested scope, row key, HTML, URL, event name, and
payload is bounded. Encoding validates before allocating
its output; decoding preflights the complete input before allocating owned
effects. `encoded_len()` only computes the exact size; it does not validate.
Errors identify the failed contract without including field contents.

## Folding


`fold_batches` coalesces consecutive declaration batches by concatenating
**every operation in order**. `folds_into` permits the same run only when both
batches have the same ABI and fingerprint and contain only `Patch`, `Rows`, or
`RowsDelta`. `Focus`, `Scroll`, `Visit`, and `Dispatch` are barriers on either
side, as are differing ABIs or fingerprints.

Coalescing never drops a declaration or rewrites its HTML, patch mode, row
order, supplied rows, delta hashes, or steps. Repeated targets and keys remain
intact, including deltas with mismatched order hashes: the client still checks
each original transition. Even repeated morphs can affect retained DOM and
unsent form values, so replacing intermediate declarations with their final
state is not safe. No HTML parsing or patch-mode shortcut is used.

`fold_run` accepts a borrowed run, including shared `Arc<EffectBatch>` values.
It checks the total `MAX_EFFECTS` and `MAX_BATCH_BYTES` (including one envelope)
before cloning operations. A run exceeding either limit stays entirely
unfolded, rather than being partially coalesced. Encoding still validates each
declaration's fields and structure.

This reduces batch envelopes and transport frames, not the operations or HTML
sent or applied. Slow clients still perform every intermediate DOM update;
coalescing is not a way to discard a backlog or reduce rendering work.

Every `Rows` carries the complete key order, so producing, encoding and
applying one costs time proportional to the collection size; use it for a
first or recovering state and `RowsDelta` for changes.

`Slot::rows_html(order, rows)` builds a `Rows` declaration and
`Slot::rows_delta_html(before, after, rows)` a `RowsDelta`; `KeyedSlot`
replaces one existing row. Raw HTML still requires `SafeHtml`. The core supports
`no_std` with `alloc`.

## License

MIT
