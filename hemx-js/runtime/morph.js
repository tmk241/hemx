// The correspondence and control snapshots live for one operation only. The DOM
// remains the source of both server defaults and browser-owned control state.
function createMorpher(dom, { ROOT, ISLAND, RESOURCE, KEY }, rootOf, updatePendingAttributes) {
  const HTML = "http://www.w3.org/1999/xhtml";
  const resetForm = HTMLFormElement.prototype.reset;
  const is = (node, name) => dom.element(node) && dom.namespaceURI(node) === HTML && dom.localName(node) === name;
  const content = (node) => is(node, "template") ? node.content : node;
  const children = (node) => dom.childNodes(content(node));
  const opaque = (node) => dom.has(node, ROOT) || dom.has(node, ISLAND);
  const fail = (message) => { throw new Error(`hemx Morph: ${message}`); };

  function inertDocument(context) {
    return dom.ownerDocument(context).implementation.createHTMLDocument("");
  }

  function copyAttributes(target, source) {
    const wanted = dom.attributes(source);
    for (const attr of dom.attributes(target)) {
      if (!wanted.some((next) => next.namespaceURI === attr.namespaceURI && next.localName === attr.localName)) {
        dom.removeAttributeNS(target, attr.namespaceURI, attr.localName);
      }
    }
    for (const attr of wanted) {
      const previous = dom.attributes(target).find((old) => old.namespaceURI === attr.namespaceURI && old.localName === attr.localName);
      if (!previous || previous.value !== attr.value || previous.name !== attr.name) {
        dom.setAttributeNS(target, attr.namespaceURI, attr.name, attr.value);
      }
    }
  }

  function parseBox(context, html) {
    if (!dom.element(context)) fail("HTML requires an element parsing context");
    // A separate document has no custom-element registry. Unlike a contextual
    // fragment, innerHTML also gives scripts the non-executing parsing path.
    const doc = inertDocument(context);
    const box = doc.createElementNS(dom.namespaceURI(context), dom.localName(context));
    copyAttributes(box, context);
    dom.setHTML(box, html);
    return box;
  }

  function parse(context, html) {
    const nodes = children(parseBox(context, html));
    for (const node of nodes) dom.remove(node);
    return nodes;
  }

  function move(parent, node, reference = null) {
    if (reference !== null && dom.parentNode(reference) !== parent) fail("move reference is not a child of its destination");
    if (node === reference || (dom.parentNode(node) === parent && dom.nextSibling(node) === reference)) return;
    if (dom.contains(node, parent)) fail("move would create a cycle");
    if (dom.isConnected(node)) {
      const destinationDocument = dom.nodeType(parent) === 9 ? parent : dom.ownerDocument(parent);
      if (!dom.isConnected(parent) || destinationDocument !== dom.ownerDocument(node) || !dom.canRelocate(parent, node)) {
        fail("moving a node that holds browser state requires native moveBefore in the same connected document");
      }
      dom.relocate(parent, node, reference);
    } else dom.insertBefore(parent, node, reference);
  }

  function compatible(a, b, ownRoot = false) {
    if (dom.nodeType(a) !== dom.nodeType(b)) return false;
    if (!dom.element(a)) return true;
    if (dom.namespaceURI(a) !== dom.namespaceURI(b) || dom.localName(a) !== dom.localName(b)) return false;
    if (is(a, "input") || is(a, "select")) {
      if (a.type !== b.type) return false;
    }
    // A boundary cannot become an ordinary element, or a differently owned
    // boundary, just because it occupies the same position.
    return ownRoot || (dom.attr(a, ROOT) === dom.attr(b, ROOT) && dom.attr(a, ISLAND) === dom.attr(b, ISLAND));
  }

  function ownership(node) {
    const resource = dom.attr(node, RESOURCE);
    const key = dom.attr(node, KEY);
    return resource !== null || key !== null ? ["resource", resource, key] : null;
  }

  function indexTree(nodes, transparentRoot = null) {
    const all = [];
    const byNode = new Map();
    const identities = new Map();
    function visit(node, parent, scope, path) {
      const own = ownership(node);
      const ownedScope = own ? [...scope, own] : scope;
      const ids = [];
      if (own) ids.push(JSON.stringify([scope, own]));
      const id = dom.attr(node, "id");
      if (id) ids.push(JSON.stringify([ownedScope, ["id", id]]));
      const island = dom.attr(node, ISLAND);
      if (island) ids.push(JSON.stringify([ownedScope, ["island", island]]));
      const record = { node, parent, ids, own, scope, children: [], opaque: opaque(node) && node !== transparentRoot, match: null, live: null };
      all.push(record);
      byNode.set(node, record);
      for (const identity of ids) {
        if (identities.has(identity)) fail(`duplicate explicit identity ${identity}`);
        identities.set(identity, record);
      }
      if (!record.opaque) {
        // Template contents form a different tree, not an empty element and not
        // a pool of identities available to the containing live document.
        const nextScope = is(node, "template") ? [...ownedScope, ["template", ids[0] || path]] : ownedScope;
        record.children = children(node).map((child, i) => visit(child, record, nextScope, own ? [i] : [...path, i]));
      }
      return record;
    }
    const roots = nodes.map((node, i) => visit(node, null, [], [i]));
    return { roots, all, byNode, identities };
  }

  function correspond(old, next, target, box, wholeElement) {
    const used = new Set();
    function pair(record, previous) {
      if (record.match || used.has(previous)) fail("conflicting explicit correspondence");
      record.match = previous;
      record.live = previous.node;
      used.add(previous);
    }
    if (wholeElement && compatible(target, box, dom.has(target, ROOT) || dom.has(box, ROOT))) {
      // The caller supplied the corresponding host. Its changing attributes
      // (including id/defaults) are updates, not a request to match a sibling.
      pair(next.roots[0], old.roots[0]);
    }
    for (const record of next.all) {
      if (wholeElement && record === next.roots[0]) continue;
      const candidates = new Set(record.ids.map((id) => old.identities.get(id)).filter(Boolean));
      if (candidates.size > 1) fail("explicit identities refer to different old nodes");
      const previous = candidates.values().next().value;
      if (previous && compatible(previous.node, record.node) && JSON.stringify(previous.own) === JSON.stringify(record.own)) pair(record, previous);
    }

    function options(record) {
      const found = [];
      function visit(child) {
        if (is(child.node, "option")) found.push(child);
        if (!child.opaque && !is(child.node, "select")) child.children.forEach(visit);
      }
      record.children.forEach(visit);
      return found;
    }
    function uniqueValues(records) {
      const values = new Map();
      for (const record of records) {
        const value = record.node.value;
        values.set(value, values.has(value) ? null : record);
      }
      return values;
    }
    function matchOptions(record) {
      if (!record.match || !is(record.node, "select")) return;
      const previous = uniqueValues(options(record.match));
      const incoming = uniqueValues(options(record));
      for (const [value, option] of incoming) {
        const before = previous.get(value);
        if (option && before && !option.ids.length && !before.ids.length && !option.match && !used.has(before) && compatible(before.node, option.node) && JSON.stringify(before.scope) === JSON.stringify(option.scope)) pair(option, before);
      }
    }
    if (!wholeElement) matchOptions({ node: box, children: next.roots, match: { node: target, children: old.roots } });
    next.all.forEach(matchOptions);

    function local(incoming, previous) {
      const available = previous.filter((record) => !used.has(record) && !record.ids.length);
      let position = 0;
      for (const record of incoming) {
        if (!record.ids.length && !record.match) {
          const candidate = available[position++];
          if (candidate && compatible(candidate.node, record.node)) {
            // Duplicate option values are not a value-based identity. Only an
            // unchanged local position/text can supply anonymous correspondence.
            const optionFits = !is(record.node, "option") || (candidate.node.value === record.node.value && dom.textContent(candidate.node) === dom.textContent(record.node));
            if (optionFits) pair(record, candidate);
          }
        }
        if (!record.opaque) {
          matchOptions(record);
          local(record.children, record.match ? record.match.children : []);
        }
      }
    }
    local(next.roots, old.roots);
    return used;
  }

  function planMoves(target, old, next, host, oldNodes, endReference) {
    const rootToken = {};
    const lists = new Map([[rootToken, [...oldNodes]]]);
    const parents = new Map();
    const containers = new Map([[rootToken, host]]);
    const connected = new Map([[rootToken, dom.isConnected(host)]]);
    const operations = [];
    const token = (record) => record.match ? record.match.node : record;
    for (const record of old.all) {
      if (!record.opaque) {
        const container = content(record.node);
        lists.set(record.node, dom.childNodes(container));
        containers.set(record.node, container);
      }
    }
    for (const [parent, list] of lists) for (const node of list) parents.set(node, parent);

    function visit(parent, records) {
      const list = lists.get(parent);
      const desired = new Set(records.map(token));
      // Ignore obsolete siblings, but keep their wrappers connected until every
      // retained descendant has escaped. Deleting a sibling needs no native move.
      const anchors = list.filter((node) => desired.has(node));
      for (const record of records) {
        const node = token(record);
        const reference = anchors[0] || (parent === rootToken ? endReference : null);
        if (node === reference) anchors.shift();
        else {
          if (record.match && dom.isConnected(record.match.node)) {
            const destination = containers.get(parent);
            if (!connected.get(parent) || dom.ownerDocument(record.match.node) !== dom.ownerDocument(target) || !dom.canRelocate(destination, record.match.node)) {
              fail("relocating a node that holds browser state requires a native moveBefore destination");
            }
          }
          for (let ancestor = parent; ancestor; ancestor = parents.get(ancestor)) {
            if (ancestor === node) fail("correspondence would create an ancestor cycle");
          }
          const before = parents.get(node);
          if (before !== undefined) {
            const source = lists.get(before);
            source.splice(source.indexOf(node), 1);
          }
          const at = reference === null || reference === endReference ? list.length : list.indexOf(reference);
          list.splice(at, 0, node);
          const anchor = anchors.indexOf(node);
          if (anchor !== -1) anchors.splice(anchor, 1);
          parents.set(node, parent);
          operations.push({ parent, record, reference });
        }
        if (!record.opaque) {
          if (!lists.has(node)) lists.set(node, []);
          if (!containers.has(node)) containers.set(node, content(record.node));
          connected.set(node, connected.get(parent) && !is(record.node, "template"));
          visit(node, record.children);
        }
      }
    }
    visit(rootToken, next.roots);
    return { operations, rootToken };
  }

  function pristineSelection(select, doc) {
    // importNode into the inert document avoids constructing registered custom
    // descendants while preflight is still in progress.
    const clone = doc.importNode(select, true);
    dom.removeAttribute(clone, "form");
    const form = doc.createElement("form");
    dom.insertBefore(form, clone, null);
    resetForm.call(form);
    return Array.from(clone.options, (option, i) => option.selected ? i : -1).filter((i) => i !== -1);
  }

  function snapshotControls(target, box, next, wholeElement) {
    const doc = dom.ownerDocument(box);
    const controls = [];
    const entries = wholeElement ? next.all : [{ node: box, live: target, match: { node: target }, children: next.roots, opaque: false }, ...next.all];
    for (const record of entries) {
      if (record.opaque) continue;
      const source = record.node;
      const before = record.match && record.match.node;
      if (is(source, "input") && before) {
        controls.push({ record, kind: "input", value: before.value, defaultValue: before.defaultValue, checked: before.checked, defaultChecked: before.defaultChecked, desiredDefaultValue: source.defaultValue, desiredValue: source.value, desiredChecked: source.defaultChecked });
      } else if (is(source, "textarea") && before) {
        controls.push({ record, kind: "textarea", value: before.value, defaultValue: before.defaultValue, desiredValue: source.defaultValue });
      } else if (is(source, "select")) {
        function hasBoundary(children) { return children.some((child) => child.opaque || hasBoundary(child.children)); }
        if (hasBoundary(record.children)) fail("select options cross an opaque ownership boundary");
        const incoming = Array.from(source.options);
        const previous = before ? Array.from(before.options) : [];
        const oldDefaults = before ? pristineSelection(before, doc).map((i) => ({ node: previous[i], value: previous[i].value })) : [];
        const newDefaults = pristineSelection(source, doc).map((i) => incoming[i]);
        const mapped = (option) => {
          const entry = next.byNode.get(option);
          return entry && entry.match ? entry.match.node : null;
        };
        const unchanged = before && before.multiple === source.multiple && oldDefaults.length === newDefaults.length && oldDefaults.every((option) => newDefaults.some((incomingOption) => mapped(incomingOption) === option.node && incomingOption.value === option.value));
        const liveSelection = previous.filter((option) => option.selected).map((node) => ({ node, value: node.value }));
        const desired = unchanged ? incoming.filter((option) => liveSelection.some((oldOption) => oldOption.node === mapped(option) && oldOption.value === option.value)) : newDefaults;
        controls.push({ record, kind: "select", desired: desired.map((option) => next.byNode.get(option)) });
      }
    }
    return controls;
  }

  function radioPlan(target, old, next, incomingRoots = next.roots, wholeElement = false) {
    const host = wholeElement ? dom.parentNode(target) : content(target);
    const tree = dom.getRootNode(host);
    // Groups can only change when a radio exists before or after the update.
    // Skip the whole-tree plan when none does; the native query is cheap.
    const radio = "input[type=radio i]";
    const hasRadio = (node) => (dom.element(node) && dom.matches(node, radio)) || dom.query(node, radio).length > 0;
    if (!hasRadio(tree) && !incomingRoots.some((record) => hasRadio(record.node))) return [];
    const owner = rootOf(target);
    const newHost = wholeElement ? incomingRoots[0] : null;
    const finalOwner = newHost && dom.has(newHost.node, ROOT) ? (newHost.match ? newHost.match.node : newHost) : owner;
    const oldGroups = new Map();
    const newGroups = new Map();
    const touched = new Set();
    const groups = [];
    const oldRadios = new Map();
    const finalRadios = new Map();
    const finalElements = [];
    const groupKeys = new Map();
    function groupKey(form, name, token) {
      // Empty names do not form radio groups.
      const association = name ? form : token;
      let names = groupKeys.get(association);
      if (!names) groupKeys.set(association, names = new Map());
      const key = name || "";
      if (!names.has(key)) names.set(key, {});
      return names.get(key);
    }
    function add(map, key, radio) {
      if (!map.has(key)) map.set(key, []);
      map.get(key).push(radio);
    }
    function owned(ancestors, expectedOwner = owner) {
      for (let i = ancestors.length - 1; i >= 0; --i) {
        const view = ancestors[i];
        if (dom.has(view.source, ROOT)) return view.token === expectedOwner;
        if (dom.has(view.source, ISLAND)) return false;
      }
      return expectedOwner === null || (!wholeElement && host !== target);
    }
    function oldWalk(node, ancestors) {
      const view = { token: node, source: node };
      const path = [...ancestors, view];
      if (is(node, "input") && node.type === "radio") {
        const key = groupKey(node.form, node.name, node);
        const radio = { token: node, source: node, value: node.value, checked: node.checked, defaultChecked: node.defaultChecked, owned: owned(path), key };
        oldRadios.set(node, radio);
        add(oldGroups, key, radio);
        const record = old.byNode.get(node);
        if (record && !record.opaque) touched.add(key);
      }
      // Template contents belong to a separate tree and have no connected peers.
      for (const child of dom.childNodes(node)) oldWalk(child, path);
    }
    oldWalk(tree, []);

    function finalWalk(node, record, ancestors) {
      if (wholeElement && node === target && !record) {
        incomingRoots.forEach((child) => finalWalk(child.match ? child.match.node : child.node, child, ancestors));
        return;
      }
      const source = record && !record.opaque ? record.node : node;
      const token = record ? (record.match ? record.match.node : record) : node;
      const view = { token, source, record, ancestors };
      const path = [...ancestors, view];
      if (dom.element(source)) finalElements.push(view);
      if (!wholeElement && node === host && !record) {
        incomingRoots.forEach((child) => finalWalk(child.match ? child.match.node : child.node, child, path));
      } else if (record && !record.opaque) {
        // The template element is present, but its content is not in this tree.
        if (!is(source, "template")) record.children.forEach((child) => finalWalk(child.match ? child.match.node : child.node, child, path));
      } else {
        for (const child of dom.childNodes(node)) finalWalk(child, null, path);
      }
    }
    finalWalk(tree, null, []);
    const firstId = new Map();
    for (const view of finalElements) {
      const id = dom.attr(view.source, "id");
      if (id && !firstId.has(id)) firstId.set(id, view);
    }
    for (const view of finalElements) {
      const source = view.source;
      if (!is(source, "input") || source.type !== "radio") continue;
      let form = null;
      const explicitForm = dom.attr(source, "form");
      if (explicitForm !== null) {
        const candidate = firstId.get(explicitForm);
        if (candidate && is(candidate.source, "form")) form = candidate.token;
      } else {
        for (let i = view.ancestors.length - 1; i >= 0; --i) {
          if (is(view.ancestors[i].source, "form")) { form = view.ancestors[i].token; break; }
        }
      }
      const key = groupKey(form, source.name, view.token);
      const radio = { token: view.token, record: view.record, source, value: source.value, defaultChecked: source.defaultChecked, owned: owned([...view.ancestors, view], finalOwner), key };
      add(newGroups, key, radio);
      finalRadios.set(view.token, radio);
      if (view.record && !view.record.opaque) touched.add(key);
      const before = oldRadios.get(view.token);
      if (before && before.key !== key) { touched.add(before.key); touched.add(key); }
    }
    // A fresh opaque subtree may introduce a checked radio that unchecks an
    // existing peer even though no owned input was itself reconciled. Likewise,
    // deleting an opaque member changes a surviving group's defaults. Neither
    // operation may silently change a group spanning ownership boundaries.
    for (const radio of finalRadios.values()) {
      if (!oldRadios.has(radio.token) && oldGroups.has(radio.key)) touched.add(radio.key);
    }
    for (const radio of oldRadios.values()) {
      if (!finalRadios.has(radio.token) && newGroups.has(radio.key)) touched.add(radio.key);
    }
    for (const key of touched) {
      const before = oldGroups.get(key) || [];
      const after = newGroups.get(key) || [];
      if ([...before, ...after].some((radio) => !radio.owned)) fail("radio group crosses another root or island");
      const defaultsBefore = before.filter((radio) => radio.defaultChecked);
      const defaultsAfter = after.filter((radio) => radio.defaultChecked);
      const unchanged = before.length > 0 && defaultsBefore.length === defaultsAfter.length && defaultsBefore.every((radio, i) => radio.token === defaultsAfter[i].token && radio.value === defaultsAfter[i].value);
      const selected = unchanged ? before.find((radio) => radio.checked) : defaultsAfter[defaultsAfter.length - 1];
      const choice = selected && after.find((radio) => radio.token === selected.token && radio.value === selected.value);
      groups.push({ radios: after, choice });
    }
    return groups;
  }

  function finishControls(controls, radios) {
    for (const state of controls) {
      const node = state.record.live;
      if (state.kind === "input") {
        if (node.type !== "file") {
          const value = state.defaultValue === state.desiredDefaultValue ? state.value : state.desiredValue;
          if (node.value !== value) node.value = value;
        }
        // Named radios are reconciled together after all associations settle.
        if (node.type === "checkbox" || (node.type === "radio" && !radios.some((group) => group.radios.some((radio) => radio.token === node)))) {
          const checked = state.defaultChecked === state.desiredChecked ? state.checked : state.desiredChecked;
          if (node.checked !== checked) node.checked = checked;
        }
      } else if (state.kind === "textarea") {
        // defaultValue is text content, so this must follow child reconciliation.
        if (node.defaultValue !== state.desiredValue) node.defaultValue = state.desiredValue;
        const value = state.defaultValue === state.desiredValue ? state.value : state.desiredValue;
        if (node.value !== value) node.value = value;
      } else {
        const desired = new Set(state.desired.map((record) => record.live));
        for (const option of Array.from(node.options)) if (!desired.has(option) && option.selected) option.selected = false;
        for (const option of desired) if (!option.selected) option.selected = true;
        if (!desired.size && node.selectedIndex !== -1) node.selectedIndex = -1;
      }
    }
    for (const group of radios) {
      const live = (radio) => radio.record ? radio.record.live : radio.token;
      for (const radio of group.radios) if (radio !== group.choice && live(radio).checked) live(radio).checked = false;
      if (group.choice && !live(group.choice).checked) live(group.choice).checked = true;
    }
  }

  // With `dryRun` only the checks run: indexing, correspondence, movement and
  // control/radio planning. Nothing live is touched, so a failure here means a
  // later real run would have failed before its first change as well.
  function reconcile(target, box, wholeElement, dryRun = false) {
    const host = wholeElement ? dom.parentNode(target) : content(target);
    if (!host) fail("element update requires an existing parent");
    const oldNodes = wholeElement ? [target] : children(target);
    const newNodes = wholeElement ? [box] : children(box);
    // An explicitly addressed root owns this operation. Only its descendant
    // roots are opaque; an island without an owning root remains opaque.
    const ownRoot = wholeElement && (dom.has(target, ROOT) || dom.has(box, ROOT));
    const old = indexTree(oldNodes, ownRoot ? target : null);
    const next = indexTree(newNodes, ownRoot ? box : null);
    correspond(old, next, target, box, wholeElement);
    if (wholeElement && dom.nodeType(host) === 9 && !next.roots[0].match) {
      // Two document elements cannot coexist while retained descendants move.
      fail("cannot connect a replacement document element before removing its predecessor");
    }
    const endReference = wholeElement ? dom.nextSibling(target) : null;
    // No live clones, attribute writes, insertions, or removals precede these
    // checks. In particular a missing atomic move leaves the subtree untouched.
    const movement = planMoves(target, old, next, host, oldNodes, endReference);
    const controls = snapshotControls(target, box, next, wholeElement);
    const radios = radioPlan(target, old, next, next.roots, wholeElement);
    // Each retained template content is a separate native tree, including its
    // radio groups. It must not share a group with the containing document.
    for (const record of next.all) {
      if (record.match && !record.opaque && is(record.node, "template")) {
        radios.push(...radioPlan(record.match.node, old, next, record.children));
      }
    }
    if (dryRun) return undefined;
    const freshRecords = new Set(next.all.filter((record) => !record.match));
    const liveToken = (token) => freshRecords.has(token) ? token.live : token;
    const resolve = (token) => token === movement.rootToken ? host : content(liveToken(token));
    for (const operation of movement.operations) {
      const record = operation.record;
      if (!record.live) record.live = dom.clone(record.node, record.opaque);
      move(resolve(operation.parent), record.live, operation.reference === null ? null : liveToken(operation.reference));
    }
    for (const record of next.all) {
      if (record.opaque) continue;
      if (dom.element(record.node)) {
        if (record.match) {
          copyAttributes(record.live, record.node);
          updatePendingAttributes(record.live, record.node);
        }
      } else if (dom.nodeValue(record.live) !== dom.nodeValue(record.node)) dom.setNodeValue(record.live, dom.nodeValue(record.node));
    }
    function prune(parent, records, candidates = dom.childNodes(parent)) {
      const desired = new Set(records.map((record) => record.live));
      for (const node of candidates) {
        if (!desired.has(node) && dom.parentNode(node) === parent) dom.removeChild(parent, node);
      }
      for (const record of records) if (!record.opaque) prune(content(record.live), record.children);
    }
    // In element mode the region is one host, not all children of its parent.
    // Its preceding/following siblings are neither indexed nor pruned.
    prune(host, next.roots, wholeElement ? oldNodes : dom.childNodes(host));
    finishControls(controls, radios);
    return wholeElement ? next.roots[0].live : target;
  }

  function morph(target, html) {
    return reconcile(target, parseBox(target, html), false);
  }

  function inertSource(source) {
    const doc = inertDocument(source);
    const clone = doc.importNode(source, true);
    function visit(node) {
      // A supplied source need not have come from our innerHTML parser. Seed
      // script nodes through that parser so even a source created by script
      // cannot introduce an executable script when its inert clone is inserted.
      if (dom.element(node) && dom.localName(node) === "script" &&
          (dom.namespaceURI(node) === HTML || dom.namespaceURI(node) === "http://www.w3.org/2000/svg")) {
        const container = doc.createElementNS(dom.namespaceURI(node), dom.namespaceURI(node) === HTML ? "div" : "svg");
        dom.setHTML(container, "<script></script>");
        const script = dom.firstChild(container);
        copyAttributes(script, node);
        dom.setText(script, dom.textContent(node));
        return script;
      }
      for (const child of children(node)) {
        const replacement = visit(child);
        if (replacement !== child) {
          dom.insertBefore(content(node), replacement, child);
          dom.remove(child);
        }
      }
      return node;
    }
    return visit(clone);
  }

  // Returns the element now standing where `target` stood: `target` itself, or
  // a new element when the tag changed. Callers that keep references must use it.
  function morphElement(target, sourceElement) {
    if (!dom.element(target) || !dom.element(sourceElement)) fail("element update requires target and source elements");
    return reconcile(target, inertSource(sourceElement), true);
  }

  // Throw exactly what `morphElement` would, before anything changes.
  function checkElement(target, sourceElement) {
    if (!dom.element(target) || !dom.element(sourceElement)) fail("element update requires target and source elements");
    reconcile(target, inertSource(sourceElement), true, true);
  }

  return { parse, morph, morphElement, checkElement, move };
}
