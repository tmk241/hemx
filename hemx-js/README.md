# hemx-js

Browser runtime assets embedded by Hemx integrations.

## Native forms (development contract)

Eligible forms are enhanced only at a cancelable, same-origin native navigation.
The runtime requires `NavigateEvent.sourceElement` and `NavigateEvent.formData`;
it does not require `precommitHandler`. Unsupported methods, encodings, targets,
origins, or missing capabilities leave the native form submission alone.

Native constraint validation and all synchronous `submit` listeners run first.
Use `event.preventDefault()` in a submit listener to veto the action. Once Hemx
has canceled the navigation and admitted the action, a later `navigate` listener
cannot retract that admission.

Hemx reuses the browser's successful controls, submitter overrides, image
coordinates, and `formdata` additions; it does not serialize the form again.
GET enhancement performs page navigation and updates URL/history after applying
the response. Ordinary POST effects do not rewrite URL/history unless an effect
explicitly requests navigation.

Native HTTP remains the baseline: endpoints must return useful HTML or redirects
without enhancement, sharing validation and mutation logic with their effect
responses. See `hemx-axum` for transport integration. A failed enhanced POST is
reported, never automatically replayed as a native submission.

## Request deadlines


Enhanced action requests have a **ten-second default**. A form or standalone
interaction trigger can set a longer or shorter client deadline:

```html
<form action="/export" method="post"
      data-hemx-form="export" data-hemx-handle="export"
      data-hemx-timeout="120s">
  <button>Export</button>
</form>
```

Use whole milliseconds (`250` or `250ms`) or whole seconds (`120s`). Values must
be positive and at most 2,147,483,647 milliseconds; surrounding whitespace follows
JavaScript's `String.trim`, consistently in the builder and browser. Zero, empty values, fractions, unknown units and overflow are errors,
not an unlimited-wait mode. Invalid literal template values fail the build;
invalid dynamic/DOM values report an error before sending or replacing a request.

For a form interaction the setting belongs to the **form**, not its submitter.
For a non-form interaction it belongs to the triggering element. There is no
inheritance from a root or ancestor and no global configuration. Hemx captures
the value when it admits the interaction, alongside its submitted data. Queued
requests keep that value even if the attribute later changes.

The timer starts when the request executes, excluding queue wait, and remains
active through upload and reading the complete response body. Expiry aborts
client waiting, clears pending indicators/disabled controls and reports one
`HEMX_REQUEST_FAILED` event through the normal localizable error path. Inputs
remain intact and failed POSTs are never automatically replayed.

This is not a server execution deadline or a rollback guarantee. A proxy/server
may impose a shorter limit; a server operation may still finish after the client
stops waiting. Applications own recovery and idempotency. Work that must survive
navigation or expose resumable progress belongs in application-owned jobs, with
SSE or another existing read path for progress. This attribute does not change
SSE connections or ordinary link navigation.

## Error messages (development contract)

Hemx writes its default English message into the owning root's
`[data-hemx-error]` outlets **before** emitting `hemx:error` on that root.
A synchronous listener can replace that text; the request's outer error handler
will not overwrite it or emit the failure again. The next request may clear it.
Use `textContent`, not `innerHTML`, for translated messages:

```js
const root = document.querySelector("[data-hemx-root]");
const messages = {
  HEMX_REQUEST_FAILED: "Die Anfrage ist fehlgeschlagen. Bitte den Stand prüfen.",
  HEMX_STALE_BUILD: "Diese Seite ist veraltet. Bitte neu laden.",
  HEMX_BATCH_FAILED: "Die Anzeige konnte nicht aktualisiert werden. Bitte den Stand prüfen.",
  HEMX_ROWS_DRIFT: "Die Anzeige ist nicht mehr aktuell. Bitte neu laden.",
  HEMX_UNSUPPORTED_ABI: "Diese Version wird nicht unterstützt. Bitte neu laden.",
  HEMX_RUNTIME_FAILED: "Die Anzeige konnte nicht verarbeitet werden.",
};
root.addEventListener("hemx:error", event => {
  // A nested root's events bubble; do not translate another root's outlets.
  if (event.target !== root) return;
  for (const outlet of root.querySelectorAll("[data-hemx-error]")) {
    if (window.hemx.rootOf(outlet) === root) {
      outlet.textContent = messages[event.detail.code];
    }
  }
});
```

The event's `detail` contains a stable `code`, diagnostic `message`, and HTTP
`status` when available (`null` otherwise). Do not parse the English diagnostic
for application decisions. `HemxErrorCode` and `HemxErrorDetail` are exported in
`hemx.d.ts`; no translation registry or extra markup is needed.

- `HEMX_REQUEST_FAILED`: HTTP, network, timeout, or response-handling failure.
- `HEMX_STALE_BUILD`: the response or batch belongs to a different build. Reload
  to continue; Hemx never replays the rejected interaction.
- `HEMX_BATCH_FAILED`: a batch could not be decoded or applied. Earlier effects
  may already have applied; later effects in that batch stop.
- `HEMX_ROWS_DRIFT`: rows still disagree after the existing single page refetch.
- `HEMX_UNSUPPORTED_ABI`: the batch uses an unsupported wire ABI version.
- `HEMX_RUNTIME_FAILED`: another runtime operation, such as HTML reconciliation,
  failed.

These codes describe the client failure, **not whether a write committed**.
A failed request can have an unknown server outcome; a failed batch can follow a
successful server mutation. Do not automatically retry writes. Public
`window.hemx.applyBatch` still returns `false` on failure, and an SSE batch failure
still stops only that batch, not the next independently received one. Missing
targets retain their separate `hemx:missing-target` diagnostic.

## Live connections (development contract)

Hemx has one live channel: `data-hemx-sse` on a root, using the browser's
`EventSource`, which reconnects by itself. Interactions go to the server as HTTP
requests, so the live channel only carries server updates. There is no
WebSocket adapter; `data-hemx-ws` fails the build. An application that needs a
bidirectional socket owns it and passes each canonical batch to
`window.hemx.applyBatch(bytes, root)`. SSE does not replay updates missed while
disconnected; an application that needs continuity sends the current state when
a connection (re)opens; `hemx_axum::Live` provides cursor replay and snapshot
recovery for this. SSE batches arriving within one animation frame are drained
in order (every 100 ms while the document is hidden). Every intermediate effect
is applied: final HTML alone cannot describe changes to unsent values or nested
resources. A failed batch does not swallow the next received batch.

## Rows

A `Rows` declaration names a collection's complete key order and supplies new
or changed rows. Before changing anything, the runtime checks that every child
of the collection is a keyed row, every ordered key has a live or supplied row,
that each supplied row can be reconciled with the live one, and that moving is
possible both before and after the supplied HTML is applied. Then it reconciles supplied rows
in place, removes rows absent from the order, and moves only rows outside the
longest already-ordered run, so focus, selection, and unsent edits survive. A
failed check changes nothing and reports one diagnostic.

A `RowsDelta` names the order it starts from and the order it produces. The
runtime hashes a collection's row keys once (the same FNV-1a 64 as
`hemx_core::order_hash`) and carries the hash forward with each applied delta;
a Patch around the collection, reconciliation of an enclosing row, a `Rows`
declaration or a page swap makes it
hash the rows again. A delta at its result is a duplicate and is skipped. A
delta from any other order changes nothing: the client missed an update, so it
fetches its page again once (`hemx:resync`) and continues from the rows the
server renders. If the next delta still finds another order before any delta
applied, refetching cannot help; that one is reported as a diagnostic instead
of refetched again. Every step is checked against the live rows and the
supplied HTML before the first change.

Rows move with `moveBefore`, which keeps all of their browser state. Safari
does not provide it yet (WebKit bug 281223). There a row is removed and
inserted again instead: it stays the same node with its control values, but
would lose focus, a text selection, frame documents, playing media, open
dialogs and popovers, and fullscreen. A row holding any of those is never
reinserted; the whole change fails visibly and changes nothing. Morphing may
still reach the same order by moving the rows around it. A missing
`moveBefore` is not drift and never refetches.
Collections are server-owned: scripts that reorder their rows make every
later delta fail.

Live connections resume where they left off: `hemx_axum::Live` sends each
batch with an SSE `id`, and `EventSource` returns the last one as
`Last-Event-ID` when it reconnects. A page renders the cursor it shows into its
`data-hemx-sse` URL (`?after=`) for the first connection.

## Request fields and history (development contract)

A request carries the native successful form controls, the hidden `__h` and
`__b` fields, and explicit `data-hemx-param-NAME` attributes on the handled
element (sent as `NAME`, dashes become underscores). Other `data-*` attributes
are DOM metadata and are never sent. A drop sends the dragged row's key as the
reserved `__drag` field.

Back and forward navigation refetch the page from the server. The runtime stores
no page HTML in `history.state`, so a returning user sees current server state.

Each applied batch resolves generated targets with native attribute queries
scoped to the owning root, so update cost stays small for large pages. Pages
containing radio inputs additionally reconcile radio groups on each Morph.

These changes are unreleased; do not assume the published 0.4.0 runtime already
implements this contract.

## License

MIT
