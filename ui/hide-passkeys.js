// Runs in every frame before the page. Valerie can't do passkeys until Apple grants it the browser passkey permission,
// and WebKit still offers them, so sites show a passkey prompt that never finishes. Hiding them makes sites offer
// passwords or "try another way" instead.
(() => {
  try { delete window.PublicKeyCredential; } catch (e) {}
  const c = navigator.credentials;
  if (!c) return;
  for (const k of ["create", "get"]) {
    const real = c[k] && c[k].bind(c);
    if (!real) continue;
    Object.defineProperty(c, k, { configurable: true, writable: true, value(o) {
      if (o && o.publicKey) return Promise.reject(new DOMException("Passkeys aren't available in this browser.", "NotSupportedError"));
      return real(o);
    } });
  }
})();
