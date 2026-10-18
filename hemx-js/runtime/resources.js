

// Resource references are exact metadata, not selectors or first-match queries.
function createResources(dom, { ROOT, ISLAND, RESOURCE, COLLECTION, KEY, ACTION }) {
  function rootOf(node) {
    for (let el = node; el; el = dom.parentElement(el)) {
      if (dom.has(el, ROOT)) return el;
      if (dom.has(el, ISLAND)) return null;
    }
    return null;
  }

  function walk(scope, visit, owned) {
    if (!scope || (owned && dom.has(scope, ISLAND) && !dom.has(scope, ROOT))) return;
    const stack = [scope];
    while (stack.length) {
      const node = stack.pop();
      if (owned && node !== scope && (dom.has(node, ROOT) || dom.has(node, ISLAND))) continue;
      if (dom.element(node)) visit(node);
      const children = dom.childNodes(node);
      for (let index = children.length - 1; index >= 0; index--) stack.push(children[index]);
    }
  }

  const forEachElement = (scope, visit) => walk(scope, visit, false);
  const forEachOwned = (scope, visit) => walk(scope, visit, true);

  // The same owned set as forEachOwned for one selector, in document order. The
  // engine finds candidates; explicit ancestry keeps nested roots and islands out.
  function select(scope, selector) {
    if (!scope || (dom.has(scope, ISLAND) && !dom.has(scope, ROOT))) return [];
    const found = dom.element(scope) && dom.matches(scope, selector) ? [scope] : [];
    for (const node of dom.query(scope, selector)) {
      let owned = true;
      for (let el = node; el && el !== scope; el = dom.parentElement(el)) {
        if (dom.has(el, ROOT) || dom.has(el, ISLAND)) { owned = false; break; }
      }
      if (owned) found.push(node);
    }
    return found;
  }
  function elements(scope, predicate) {
    const found = [];
    forEachOwned(scope, (node) => { if (predicate(node)) found.push(node); });
    return found;
  }
  function firstElement(scope, predicate) {
    let found = null;
    forEachOwned(scope, (node) => { if (!found && node !== scope && predicate(node)) found = node; });
    return found;
  }
  function unique(nodes, description) {
    const found = [...new Set(nodes)];
    if (found.length > 1) throw new Error(`hemx ambiguous ${description}`);
    return found[0] || null;
  }
  const attrEquals = (node, name, value) => dom.attr(node, name) === String(value);
  const marker = (resource) => resource.kind === "handle" ? ACTION : RESOURCE;
  function bindings(scope, resource) {
    return select(scope, dom.attrSelector(marker(resource), resource.id));
  }
  function nearestKey(node, scope) {
    for (let current = node; current; current = dom.parentElement(current)) {
      const key = dom.attr(current, KEY);
      if (key !== null) return key;
      if (current === scope || dom.has(current, ROOT) || dom.has(current, ISLAND)) break;
    }
    return null;
  }
  function collectionOwner(node, scope) {
    for (let current = dom.parentElement(node); current; current = dom.parentElement(current)) {
      if (dom.has(current, COLLECTION)) return current;
      if (current === scope || dom.has(current, KEY) || dom.has(current, ROOT) || dom.has(current, ISLAND)) break;
    }
    return null;
  }
  function collectionRows(scope, containers, key) {
    const owners = new Set(containers);
    return select(scope, dom.attrSelector(KEY, key)).filter((node) => owners.has(collectionOwner(node, scope)));
  }

  // A key on a Rows target scopes one collection instance; row keys never
  // participate in finding the collection itself.

  function collectionTarget(scope, ref) {
    if (ref.resource.kind !== "slot" || (ref.scope && ref.scope.kind !== "key")) {
      throw new Error("hemx Rows require a slot destination");
    }
    const containers = bindings(scope, ref.resource).filter((node) => dom.has(node, COLLECTION));
    if (!containers.length) throw new Error("hemx Rows require data-hemx-collection");
    const matches = ref.scope ? containers.filter((node) => nearestKey(node, scope) === ref.scope.value) : containers;
    return unique(matches, `collection destination ${ref.resource.id}`);
  }

  function fieldTargets(scope, ref) {
    if (ref.resource.kind !== "form" || !ref.scope || ref.scope.kind !== "field") {
      throw new Error("hemx field scope requires a form resource");
    }
    const form = unique(bindings(scope, ref.resource), `form resource ${ref.resource.id}`);
    if (!form) return [];
    const named = dom.attrSelector("name", ref.scope.value);
    if (form instanceof HTMLFormElement) {
      const controls = new Set(dom.formControls(form));
      return select(scope, named).filter((node) =>
        controls.has(node) || (node instanceof HTMLInputElement && node.type === "image" && node.form === form));
    }
    return select(form, named);
  }

  function targetFor(scope, ref) {
    if (ref.scope && ref.scope.kind === "field") {
      return unique(fieldTargets(scope, ref), `form field ${ref.scope.value}`);
    }
    const all = bindings(scope, ref.resource);
    const containers = all.filter((node) => dom.has(node, COLLECTION));
    if (ref.scope && ref.scope.kind === "key") {
      const matches = containers.length && ref.resource.kind === "slot"
        ? collectionRows(scope, containers, ref.scope.value)
        : all.filter((node) => nearestKey(node, scope) === ref.scope.value);
      return unique(matches, `keyed resource ${ref.resource.id}/${ref.scope.value}`);
    }
    return unique(containers.length ? containers : all, `resource ${ref.resource.id}`);
  }

  return { rootOf, forEachElement, forEachOwned, select, firstElement, elements, unique,
    attrEquals, targetFor, collectionTarget, nearestKey, collectionOwner, collectionRows, fieldTargets };
}
