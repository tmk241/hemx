// Native DOM access must not consult named properties on HTMLFormElement.
// A successful control named "getAttribute" or "ownerDocument" is still data.
function nativeDOM() {
  const dom = {};
  const call = (fn) => Function.prototype.call.bind(fn);
  const getter = (prototype, name) => call(Object.getOwnPropertyDescriptor(prototype, name).get);
  const setter = (prototype, name) => call(Object.getOwnPropertyDescriptor(prototype, name).set);
  for (const name of [
    "nodeType", "nodeName", "nodeValue", "textContent", "parentNode", "parentElement",
    "firstChild", "nextSibling", "ownerDocument", "isConnected"
  ]) dom[name] = getter(Node.prototype, name);
  for (const name of ["localName", "tagName", "namespaceURI", "classList", "innerHTML"])
    dom[name] = getter(Element.prototype, name);
  const childNodes = getter(Node.prototype, "childNodes");
  const attributes = getter(Element.prototype, "attributes");
  const getAttribute = call(Element.prototype.getAttribute);
  const hasAttribute = call(Element.prototype.hasAttribute);
  dom.element = (node) => node != null && dom.nodeType(node) === 1;
  dom.attr = (node, name) => dom.element(node) ? getAttribute(node, name) : null;
  dom.has = (node, name) => dom.element(node) && hasAttribute(node, name);
  dom.childNodes = (node) => node ? Array.from(childNodes(node)) : [];
  dom.children = (node) => dom.childNodes(node).filter(dom.element);
  dom.attributes = (node) => Array.from(attributes(node));
  dom.setAttribute = call(Element.prototype.setAttribute);
  dom.setAttributeNS = call(Element.prototype.setAttributeNS);
  dom.removeAttribute = call(Element.prototype.removeAttribute);
  dom.removeAttributeNS = call(Element.prototype.removeAttributeNS);
  dom.formControls = getter(HTMLFormElement.prototype, "elements");
  dom.replaceWith = call(Element.prototype.replaceWith);
  const htmlFocus = HTMLElement.prototype.focus;
  const svgFocus = SVGElement.prototype.focus;
  const mathFocus = typeof MathMLElement === "undefined" ? null : MathMLElement.prototype.focus;
  dom.focus = (node, options) => {
    const method = node instanceof HTMLElement ? htmlFocus : node instanceof SVGElement ? svgFocus : mathFocus;
    if (typeof method !== "function") throw new Error("hemx target does not support native focus");
    method.call(node, options);
  };
  dom.scrollIntoView = call(Element.prototype.scrollIntoView);
  dom.setHTML = setter(Element.prototype, "innerHTML");
  dom.setText = setter(Node.prototype, "textContent");
  dom.setNodeValue = setter(Node.prototype, "nodeValue");
  dom.insertBefore = call(Node.prototype.insertBefore);
  dom.removeChild = call(Node.prototype.removeChild);
  dom.contains = call(Node.prototype.contains);
  // Attribute queries run in the engine instead of visiting every node from
  // script; callers still decide ownership explicitly.
  const queryMethods = new Map([
    [1, Element.prototype.querySelectorAll],
    [9, Document.prototype.querySelectorAll],
    [11, DocumentFragment.prototype.querySelectorAll],
  ]);
  dom.query = (scope, selector) => {
    const method = queryMethods.get(dom.nodeType(scope));
    return method ? Array.from(method.call(scope, selector)) : [];
  };
  dom.matches = call(Element.prototype.matches);
  const escapeCSS = CSS.escape;
  dom.attrSelector = (name, value) => `[${name}="${escapeCSS(String(value))}"]`;
  dom.clone = call(Node.prototype.cloneNode);
  dom.getRootNode = call(Node.prototype.getRootNode);
  dom.on = call(EventTarget.prototype.addEventListener);
  dom.off = call(EventTarget.prototype.removeEventListener);
  dom.dispatch = call(EventTarget.prototype.dispatchEvent);
  dom.remove = (node) => {
    const parent = dom.parentNode(node);
    if (parent) dom.removeChild(parent, node);
  };
  const elementMove = Element.prototype.moveBefore;
  const fragmentMove = DocumentFragment.prototype.moveBefore;
  const documentMove = Document.prototype.moveBefore;
  const moveMethod = (parent) => {
    const kind = dom.nodeType(parent);
    return kind === 1 ? elementMove : kind === 11 ? fragmentMove : kind === 9 ? documentMove : null;
  };
  dom.canMove = (parent) => typeof moveMethod(parent) === "function";
  dom.atomicMove = (parent, node, reference) => moveMethod(parent).call(parent, node, reference);
  // Without moveBefore (Safari today) a connected element can only be removed
  // and inserted again. It stays the same node with its control values, but
  // loses focus, selection, frame documents, playing media, open dialogs and
  // popovers, and fullscreen. That loss must never happen silently, so only a
  // row holding none of them may be reinserted.
  const stateful = "iframe, frame, embed, object, audio, video, dialog[open]";
  dom.holdsState = (node) => {
    if (!dom.element(node)) return false;
    const doc = dom.ownerDocument(node);
    const inside = (other) => !!other && (other === node || dom.contains(node, other));
    if (inside(doc.activeElement) || inside(doc.fullscreenElement)) return true;
    // A selection that merely covers the row (Range.selectNode) has both ends in
    // the parent, so the ends say nothing: ask each range whether it meets the row.
    const selection = doc.getSelection ? doc.getSelection() : null;
    if (selection && !selection.isCollapsed) {
      for (let index = 0; index < selection.rangeCount; index += 1) {
        const range = selection.getRangeAt(index);
        if (!range.collapsed && range.intersectsNode(node)) return true;
      }
    }
    if (dom.matches(node, stateful) || dom.query(node, stateful).length) return true;
    for (const popover of [node, ...dom.query(node, "[popover]")]) {
      try {
        if (dom.matches(popover, ":popover-open")) return true;
      } catch {
        // A browser without popovers has none open.
      }
    }
    return false;
  };
  dom.canRelocate = (parent, node) => dom.canMove(parent) || !dom.holdsState(node);
  dom.relocate = (parent, node, reference) => dom.canMove(parent)
    ? dom.atomicMove(parent, node, reference)
    : dom.insertBefore(parent, node, reference);
  return dom;
}
