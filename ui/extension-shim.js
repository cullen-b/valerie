// Put first in the background and own pages of Chrome Web Store extensions (src/chrome_store.rs, `prepare`): Chrome
// behaviour WebKit's extension engine lacks. Ideas from Search's ExtensionShims.swift (github.com/driceroland/Search, MIT).
(() => {
  const root = typeof globalThis !== "undefined" ? globalThis : self;
  if (root.__valerieShim) return;
  Object.defineProperty(root, "__valerieShim", { value: true });

  // Bitwarden's bundled TypeScript uses these as class keys before anything else runs.
  for (const name of ["dispose", "asyncDispose"]) {
    if (Symbol[name] === undefined) Object.defineProperty(Symbol, name, { value: Symbol.for("Symbol." + name) });
  }

  // WebKit gives the background worker the user agent of the last web page, Safari's. A Chrome build that sees it
  // takes its Safari path: Bitwarden's then asks a Safari app for an answer thousands of times a second and never starts.
  const nav = root.navigator;
  if (nav && !/ Chrome\//.test(nav.userAgent)) {
    const ua = nav.userAgent.replace(/ Version\/[\d.]+.*$/, "").replace(/ Safari\/[\d.]+$/, "") + " Chrome/__VALERIE_CHROME__ Safari/537.36";
    const proto = Object.getPrototypeOf(nav);
    const major = "__VALERIE_CHROME__".split(".")[0];
    const brands = [{ brand: "Chromium", version: major }, { brand: "Google Chrome", version: major }, { brand: "Not.A/Brand", version: "99" }];
    const low = { brands, mobile: false, platform: "macOS" };
    const high = { architecture: "arm", bitness: "64", model: "", platformVersion: "15.0.0", wow64: false, uaFullVersion: "__VALERIE_CHROME__",
      fullVersionList: brands.map((b) => ({ brand: b.brand, version: b.version === major ? "__VALERIE_CHROME__" : b.version + ".0.0.0" })) };
    const data = Object.assign({}, low, {
      getHighEntropyValues: (hints) => Promise.resolve(Object.assign({}, low, ...(Array.isArray(hints) ? hints : []).filter((h) => h in high).map((h) => ({ [h]: high[h] })))),
      toJSON: () => low,
    });
    try {
      Object.defineProperty(proto, "userAgent", { get: () => ua, configurable: true });
      Object.defineProperty(proto, "appVersion", { get: () => ua.replace(/^Mozilla\//, ""), configurable: true });
      Object.defineProperty(proto, "vendor", { get: () => "Google Inc.", configurable: true });
      if (!("userAgentData" in nav)) Object.defineProperty(proto, "userAgentData", { get: () => data, configurable: true });
    } catch (_) {}
  }
})();
