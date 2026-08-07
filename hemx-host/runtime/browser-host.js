(() => {
  function variant(value) {
    if (!value || typeof value !== "object") return null;
    const keys = Object.keys(value);
    return keys.length === 1 ? { kind: keys[0], data: value[keys[0]] || {} } : null;
  }

  function failure(id, capability, kind, message) {
    return { Failed: { id: id || null, capability: capability || null, kind, message: String(message) } };
  }

  async function haptic(data) {
    if (!navigator.vibrate) return failure(data.id, "Haptics", "Unavailable", "haptics unsupported");
    const pattern = data.pattern === "Warning" || data.pattern === "Error" ? [30, 40, 30] : 20;
    navigator.vibrate(pattern);
    return { Acknowledged: { id: data.id } };
  }

  async function share(data) {
    if (!navigator.share) return failure(data.id, "Share", "Unavailable", "share unsupported");
    const payload = data.payload || {};
    const request = {};
    if (payload.title) request.title = payload.title;
    if (payload.text) request.text = payload.text;
    if (payload.url) request.url = payload.url;
    try {
      await navigator.share(request);
      return { ShareCompleted: { id: data.id, completed: true } };
    } catch (error) {
      if (error && error.name === "AbortError") return { ShareCompleted: { id: data.id, completed: false } };
      return failure(data.id, "Share", "Error", error && error.message ? error.message : error);
    }
  }

  function supports(capability, shape) {
    return (capability === "haptics" && shape === "Fire" && !!navigator.vibrate)
      || (capability === "share" && shape === "Request" && !!navigator.share);
  }

  async function perform(call) {
    const request = variant(call);
    if (!request) return failure(null, null, "Error", "invalid host call");
    if (request.kind === "Haptic") return haptic(request.data);
    if (request.kind === "Share") return share(request.data);
    return failure(request.data.id || null, null, "Unavailable", `unsupported host call ${request.kind}`);
  }

  window.hemxBrowserHost = Object.freeze({ name: "browser-pwa", supports, perform });
})();
