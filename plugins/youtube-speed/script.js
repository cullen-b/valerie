// YouTube speed control. Runs on matching pages with `settings` from plugin.yaml.
if (window.__valerieSpeed) return;
const S = Object.assign({ faster_key: "+", slower_key: "-", reset_key: "*", step: 0.25, min_speed: 0.25, max_speed: 16,
  remember: true, display: "hover", shift_scroll: true }, settings || {});
const STORE = "valerie-video-speed";
let rate = (S.remember && parseFloat(localStorage.getItem(STORE))) || 1;
let ourChangeUntil = 0, loadedAt = 0, badge = null, flashTimer = 0, wheelAcc = 0;

const label = (r) => (Number.isInteger(r) ? r.toFixed(1) : String(r)) + "×";
const videos = () => [...document.querySelectorAll("video")];

function setRate(r, show = true) {
  rate = Math.round(Math.min(S.max_speed, Math.max(S.min_speed, r)) * 100) / 100;
  ourChangeUntil = performance.now() + 300;
  for (const v of videos()) {
    if (Math.abs(v.playbackRate - rate) > 0.001) try { v.playbackRate = rate; } catch (_) { /* above this browser's limit */ }
  }
  if (S.remember) try { localStorage.setItem(STORE, String(rate)); } catch (_) {}
  render(show);
  return label(rate);
}

// YouTube resets the speed for every new video; put ours back. A change made in YouTube's own menu is adopted.
document.addEventListener("loadedmetadata", (e) => {
  if (e.target.tagName !== "VIDEO") return;
  loadedAt = performance.now();
  setTimeout(() => setRate(rate, false), 50);
}, true);
document.addEventListener("ratechange", (e) => {
  const v = e.target;
  if (v.tagName !== "VIDEO" || performance.now() < ourChangeUntil || Math.abs(v.playbackRate - rate) < 0.001) return;
  if (performance.now() - loadedAt < 1500) return void setRate(rate, false);
  rate = Math.round(v.playbackRate * 100) / 100;
  if (S.remember) try { localStorage.setItem(STORE, String(rate)); } catch (_) {}
  render(true);
}, true);

// Keys: ignored while typing (search box, comments).
const typing = (t) => t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));
addEventListener("keydown", (e) => {
  if (e.metaKey || e.ctrlKey || e.altKey || !document.querySelector("video")) return;
  if (typing(e.composedPath ? e.composedPath()[0] : e.target)) return;
  let delta = 0;
  if (e.key === S.faster_key || (S.faster_key === "+" && e.key === "=")) delta = 1;
  else if (e.key === S.slower_key || (S.slower_key === "-" && e.key === "_")) delta = -1;
  else if (e.key !== S.reset_key) return;
  e.preventDefault(); e.stopImmediatePropagation(); // YouTube uses +/- for caption size
  setRate(delta ? rate + delta * S.step : 1);
}, true);

// Shift + scroll over the player.
addEventListener("wheel", (e) => {
  if (!S.shift_scroll || !e.shiftKey) return;
  const over = e.target.closest && (e.target.closest("#movie_player") || e.target.closest("video"));
  if (!over) return;
  e.preventDefault(); e.stopPropagation();
  wheelAcc += Math.abs(e.deltaY) > Math.abs(e.deltaX) ? e.deltaY : e.deltaX; // Shift turns the wheel sideways on macOS
  if (Math.abs(wheelAcc) >= 60) { setRate(rate + (wheelAcc < 0 ? 1 : -1) * S.step); wheelAcc = 0; }
}, { capture: true, passive: false });

// The badge on the video: − speed +. Click the number to go back to 1×.
function playerFor() { return document.querySelector("#movie_player") || (document.querySelector("video") || {}).parentElement; }
function ensureBadge() {
  const host = playerFor();
  if (!host || S.display === "never") return null;
  if (badge && badge.parentElement === host) return badge;
  badge = document.createElement("div");
  badge.className = "valerie-speed";
  // Built with DOM calls: YouTube's Trusted Types policy blocks innerHTML.
  const button = (d, text, title) => { const b = document.createElement("button"); b.dataset.d = d; b.textContent = text; b.title = title; return b; };
  const value = document.createElement("span"); value.title = "Back to 1×";
  badge.append(button("-1", "−", "Slower"), value, button("1", "+", "Faster"));
  badge.addEventListener("click", (e) => {
    e.preventDefault(); e.stopPropagation();
    const d = e.target.dataset && e.target.dataset.d;
    if (d) setRate(rate + Number(d) * S.step); else if (e.target.tagName === "SPAN") setRate(1);
  }, true);
  for (const ev of ["dblclick", "mousedown", "mouseup"]) badge.addEventListener(ev, (e) => e.stopPropagation(), true); // no pause / fullscreen
  if (getComputedStyle(host).position === "static") host.style.position = "relative";
  host.appendChild(badge);
  return badge;
}
function render(show) {
  const b = ensureBadge();
  if (!b) return;
  b.querySelector("span").textContent = label(rate);
  b.dataset.mode = S.display;
  if (show && S.display !== "never") {
    b.classList.add("flash");
    clearTimeout(flashTimer);
    flashTimer = setTimeout(() => b.classList.remove("flash"), 1200);
  }
}
// YouTube swaps pages without reloading.
document.addEventListener("yt-navigate-finish", () => setTimeout(() => setRate(rate, false), 300));

window.__valerieSpeed = Object.freeze({ change: (d) => setRate(rate + d * S.step), reset: () => setRate(1), get rate() { return rate; } });
setRate(rate, false);
