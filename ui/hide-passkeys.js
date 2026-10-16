// Runs in every frame before the page. Valerie can't do passkeys until Apple grants it the browser passkey permission,
// and WebKit still offers them, so sites show a passkey prompt that never finishes. Hiding them makes sites offer
// passwords or "try another way" instead.
(() => {
  let ipc = window.ipc && window.ipc.postMessage && window.ipc.postMessage.bind(window.ipc);
  const wanted = () => {
    try {
      ipc = ipc || window.ipc.postMessage.bind(window.ipc);
      if (window.top === window) ipc(JSON.stringify({ t: "passkey_wanted" }));
    } catch (e) {}
  };
  try { delete window.PublicKeyCredential; } catch (e) {}
  // On the prototype: on a tab's first page navigator.credentials may not exist yet when this runs.
  const proto = window.CredentialsContainer && CredentialsContainer.prototype;
  if (!proto) return;
  for (const k of ["create", "get"]) {
    const real = proto[k];
    if (typeof real !== "function") continue;
    Object.defineProperty(proto, k, { configurable: true, writable: true, value(o) {
      if (o && o.publicKey && !(k === "get" && o.mediation === "conditional")) wanted();
      if (o && o.publicKey) return Promise.reject(new DOMException("Passkeys aren't available in this browser.", "NotSupportedError"));
      return real.call(this, o);
    } });
  }
})();
