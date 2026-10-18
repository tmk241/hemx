# hemx-js

Browser runtime assets embedded by Hemx integrations.

## Native forms

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

## Live connections

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

## Request fields and history

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

These request-field and history behaviors were added in Hemx 0.5.

## License

MIT
