// Native submission owns validation, successful controls and submit cancellation.
// Hemx decides after every other submit listener has run and then takes the
// submission over by canceling that submit event. It never starts, cancels or
// intercepts a navigation, so browsers and automation see an ordinary submit.
function createForms(dom, ownerOf, accepts, admit) {
  const nativeForm = (form, name) =>
    Object.getOwnPropertyDescriptor(HTMLFormElement.prototype, name).get.call(form);
  const isSubmitter = (node) =>
    (node instanceof HTMLButtonElement && node.type === "submit") ||
    (node instanceof HTMLInputElement && (node.type === "submit" || node.type === "image"));

  function submission(form, source) {
    if (!form) return { action: location.href, method: "POST", enctype: "application/x-www-form-urlencoded", target: "" };
    const value = (name, property) => isSubmitter(source) && dom.has(source, `form${name}`)
      ? source[property] : nativeForm(form, name);
    // Without an own target the document's first <base target> applies.
    const base = document.querySelector("base[target]");
    return {
      action: value("action", "formAction"),
      method: value("method", "formMethod").toUpperCase(),
      enctype: value("enctype", "formEnctype"),
      target: value("target", "formTarget") || (base ? base.getAttribute("target") : ""),
    };
  }

  // `new FormData(form, submitter)` is the one serialization of a taken-over
  // submission; without submitter support it would drop the clicked button.
  function serializesSubmitter() {
    const form = document.createElement("form");
    const button = document.createElement("button");
    button.name = "hemx";
    button.value = "submitter";
    form.append(button);
    try {
      return new FormData(form, button).get("hemx") === "submitter";
    } catch (_) {
      return false;
    }
  }

  function bind() {
    if (!serializesSubmitter()) return;

    dom.on(document, "submit", (event) => {
      const form = event.target;
      if (!(form instanceof HTMLFormElement) || !event.isTrusted) return;
      const owner = ownerOf(form);
      if (!owner) return;
      // A window listener added while the event is still capturing runs after
      // every listener already on its path. (A listener that a submit handler
      // adds to the window during this dispatch runs after it.) Each submission
      // owns its listener: a nested submit (requestSubmit inside a submit
      // listener) reaches this one first but leaves it in place, because the
      // outer event is still dispatching. One left behind by stopped
      // propagation removes itself when a later submit finds its event done.
      const last = (current) => {
        if (current !== event && event.eventPhase !== Event.NONE) return;
        window.removeEventListener("submit", last);
        if (current === event) take(event, form, owner);
      };
      window.addEventListener("submit", last);
    }, true);
  }

  function take(event, form, owner) {
    // A listener that canceled the event vetoes; one that stopped propagation
    // (also on the window itself, before this listener) keeps it native.
    if (event.defaultPrevented || event.cancelBubble || !dom.isConnected(form) || ownerOf(form) !== owner) return;
    const source = event.submitter || form;
    if (source !== form && source.form !== form) return;
    if (!accepts(form, source)) return;
    const options = submission(form, source);
    if (!["GET", "POST"].includes(options.method) ||
        (options.target && options.target !== "_self") ||
        !["application/x-www-form-urlencoded", "multipart/form-data"].includes(options.enctype)) return;
    const action = new URL(options.action, location.href);
    if (action.origin !== location.origin || !["http:", "https:"].includes(action.protocol)) return;
    // Serialize exactly once, including formdata hooks and image coordinates.
    // If that throws, the native submission proceeds untouched.
    const data = source === form ? new FormData(form) : new FormData(form, source);
    event.preventDefault();
    admit(form, source, { form, data, submission: { ...options, action: action.href } });
  }

  return { bind, submission, isSubmitter };
}
