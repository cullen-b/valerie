// Casino games drawn as pixel art on <canvas id="casino">: blackjack, video poker, baccarat, roulette and slots.
// Used by the "You're offline" page (pages.offline_game) and valerie://page/casino (the play_* commands).
(() => {
  const W = 210, H = 132, cv = document.getElementById("casino");
  if (!cv) return;
  const ctx = cv.getContext("2d");
  // A 3×5 pixel font: five rows of three, top to bottom.
  const FONT = {
    A:"010101111101101",B:"110101110101110",C:"011100100100011",D:"110101101101110",E:"111100110100111",F:"111100110100100",
    G:"011100101101011",H:"101101111101101",I:"111010010010111",J:"001001001101010",K:"101101110101101",L:"100100100100111",
    M:"101111111101101",N:"110101101101101",O:"010101101101010",P:"110101110100100",Q:"010101101110011",R:"110101110101101",
    S:"011100010001110",T:"111010010010010",U:"101101101101111",V:"101101101101010",W:"101101111111101",X:"101101010101101",
    Y:"101101010010010",Z:"111001010100111",0:"111101101101111",1:"010110010010111",2:"110001010100111",3:"110001010001110",
    4:"101101111001001",5:"111100110001110",6:"011100111101111",7:"111001010010010",8:"111101111101111",9:"111101111001110",
    "!":"010010010000010","?":"110001010000010",".":"000000000000010",",":"000000000010100","-":"000000111000000",
    "+":"000010111010000",":":"000010000010000","=":"000111000111000"," ":"000000000000000",
  };
  const SUIT = {
    "♠":["0001000","0011100","0111110","1111111","1101011","0001000","0011100"],
    "♥":["0110110","1111111","1111111","0111110","0011100","0001000","0000000"],
    "♦":["0001000","0011100","0111110","1111111","0111110","0011100","0001000"],
    "♣":["0011100","0011100","1101011","1111111","1101011","0001000","0011100"],
  };
  const SPRITE = {
    SEVEN:["111111111","111111111","000001110","000011100","000111000","000111000","001110000","001110000","001110000"],
    BELL:["000010000","000111000","001111100","001111100","011111110","011111110","111111111","000000000","000111000"],
    CHERRY:["000000011","000001100","000010100","000100010","011000110","111101111","111101111","011000110","000000000"],
  };
  const RANKS = ["A","2","3","4","5","6","7","8","9","10","J","Q","K"];
  const still = matchMedia("(prefers-reduced-motion: reduce)");
  // Only Valerie's own page keeps your chips: the offline page belongs to the site that didn't load.
  const OWN = location.protocol === "valerie:";
  let oy = 0, S = 2, ink = "#535353", dim = "#cfcfcf", chips = 100, best = 100, ended = -Infinity, backOnline = false, said = "";
  try { const s = OWN && JSON.parse(localStorage.getItem("casino") || "{}"); if (s && s.chips > 0) { chips = s.chips; best = s.best || chips; } } catch {}
  const save = () => { best = Math.max(best, chips); if (OWN) try { localStorage.setItem("casino", JSON.stringify({ chips, best, game: game.name })); } catch {} };

  // Drawing, in the 210×124 grid of big pixels.
  const px = (x, y, w = 1, h = 1, c = ink) => { ctx.fillStyle = c; ctx.fillRect(Math.round(x) * S, (Math.round(y) + oy) * S, w * S, h * S); };
  const wide = s => String(s).length * 4 - 1;
  const text = (s, x, y, c = ink) => {
    x = Math.round(x); y = Math.round(y);
    for (const ch of String(s)) {
      const g = FONT[ch] || FONT[" "];
      for (let i = 0; i < 15; i++) if (g[i] === "1") c ? px(x + i % 3, y + (i / 3 | 0), 1, 1, c) : ctx.clearRect((x + i % 3) * S, (y + oy + (i / 3 | 0)) * S, S, S);
      x += 4;
    }
  };
  const right = (s, x, y, c) => text(s, x - wide(s), y, c);
  const middle = (s, y, c) => text(s, Math.round((W - wide(s)) / 2), y, c);
  const box = (x, y, w, h, c = ink) => { px(x + 1, y, w - 2, 1, c); px(x + 1, y + h - 1, w - 2, 1, c); px(x, y + 1, 1, h - 2, c); px(x + w - 1, y + 1, 1, h - 2, c); };
  const sprite = (rows, x, y, k = 1, c = ink) => rows.forEach((row, j) => [...row].forEach((b, i) => b === "1" && px(x + i * k, y + j * k, k, k, c)));
  const card = (x, y, c, down, top) => {
    ctx.clearRect(x * S, (y + oy) * S, 22 * S, 30 * S);
    box(x, y, 22, 30);
    if (down) {
      for (let j = 2; j < 28; j++) for (let i = 2; i < 20; i++) if ((i + j) % 4 === 0) px(x + i, y + j);
      return;
    }
    text(c.r, x + 2, y + 2);
    // In the corner under the rank, so it still shows when the next card covers the rest.
    sprite(SUIT[c.s], x + 2, y + 9);
    if (top) sprite(SUIT[c.s], x + 12, y + 19);
  };
  const hand = (cards, y, hideSecond) => { const x = Math.round((W - 22 - (cards.length - 1) * 14) / 2); cards.forEach((c, i) => card(x + i * 14, y, c, hideSecond && i === 1, i === cards.length - 1)); };
  const dashes = y => { for (let x = 0; x < W; x += 4) px(x, y, 2); };

  // Timing: steps one after another, or all at once for people who asked for less motion.
  const later = (f, ms) => (still.matches ? f() : setTimeout(f, ms));
  const seq = (steps, gap, done) => {
    let i = 0;
    const go = () => { if (i < steps.length) { steps[i++](); draw(); later(go, gap); } else done(); };
    go();
  };
  const shuffle = a => { for (let i = a.length - 1; i > 0; i--) { const j = Math.floor(Math.random() * (i + 1)); [a[i], a[j]] = [a[j], a[i]]; } return a; };
  const deck = (n = 1) => { const d = []; for (let k = 0; k < n; k++) for (const s in SUIT) for (const r of RANKS) d.push({ r, s }); return shuffle(d); };
  const shoe = n => { let cards = []; return () => { if (cards.length < 13 * n) cards = deck(n); return cards.pop(); }; };
  // Takes a bet; out of chips, the house hands over another 100 first.
  const take = n => { let note = ""; if (chips < n) { chips = 100; note = "THE HOUSE GIVES YOU 100. "; } chips -= n; return note; };
  const won = (back, bet) => back > bet ? " +" + (back - bet) : back === bet ? " BET BACK" : " -" + bet;
  const over = () => {
    ended = performance.now();
    save(); draw();
    if (backOnline && !games.some(g => g.live)) setTimeout(() => location.reload(), 900);
  };

  const blackjack = (() => {
    const next = shoe(6);
    let dealer = [], player = [], stake = 0, hide = true;
    const total = h => { let t = 0, a = 0; for (const c of h) { t += c.r === "A" ? 11 : isNaN(c.r) ? 10 : +c.r; if (c.r === "A") a++; } while (t > 21 && a) { t -= 10; a--; } return t; };
    const natural = h => h.length === 2 && total(h) === 21;
    const g = { name: "BLACKJACK", live: false, busy: false, msg: "PRESS SPACE TO DEAL" };
    const finish = () => {
      hide = false; g.live = g.busy = false;
      const p = total(player), d = total(dealer), bj = natural(player) && !natural(dealer);
      const back = bj ? stake * 2.5 : p > 21 || (natural(dealer) && !natural(player)) ? 0 : d > 21 || p > d ? stake * 2 : p < d ? 0 : stake;
      chips += back;
      g.msg = (back === stake ? "PUSH" : bj ? "BLACKJACK!" : p > 21 ? "BUST" : d > 21 ? "DEALER BUSTS" : back ? "YOU WIN" : "DEALER WINS") + won(back, stake);
      over();
    };
    const stand = () => {
      g.busy = true; hide = false; g.msg = "DEALER PLAYS";
      const step = () => { if (total(dealer) >= 17) return finish(); dealer.push(next()); draw(); later(step, 380); };
      later(step, 380);
    };
    const deal = () => {
      const note = take(10); stake = 10; hide = true; g.live = true;
      player = [next(), next()]; dealer = [next(), next()];
      g.msg = note + "HIT OR STAND?";
      if (natural(player) || natural(dealer)) finish();
    };
    const hit = () => { player.push(next()); const t = total(player); if (t > 21) finish(); else if (t === 21) stand(); };
    const double = () => { chips -= 10; stake = 20; player.push(next()); total(player) > 21 ? finish() : stand(); };
    g.buttons = () => {
      const play = g.live && !g.busy;
      return [
        { label: "DEAL", key: "enter", act: deal, on: !g.live, lit: true },
        { label: "HIT", key: "h", act: hit, on: play, lit: true },
        { label: "STAND", key: "s", act: stand, on: play },
        { label: "DOUBLE", key: "d", act: double, on: play && player.length === 2 && chips >= 10 },
      ];
    };
    g.render = () => {
      middle("DEALER" + (dealer.length ? " " + (g.live && hide ? total([dealer[0]]) : total(dealer)) : ""), 10);
      hand(dealer, 18, g.live && hide);
      dashes(52);
      middle("YOU" + (player.length ? " " + total(player) : ""), 56);
      hand(player, 64);
    };
    return g;
  })();

  const poker = (() => {
    const PAY = [["ROYAL FLUSH", 250], ["STRAIGHT FLUSH", 50], ["4 OF A KIND", 25], ["FULL HOUSE", 9], ["FLUSH", 6], ["STRAIGHT", 4], ["3 OF A KIND", 3], ["TWO PAIR", 2], ["JACKS OR BETTER", 1]];
    let cards = [], held = [], pack = [], win = -1;
    const value = r => ({ A: 14, K: 13, Q: 12, J: 11 })[r] || +r;
    const rank = h => {
      const v = h.map(c => value(c.r)).sort((a, b) => b - a), n = {};
      v.forEach(x => (n[x] = (n[x] || 0) + 1));
      const counts = Object.values(n).sort((a, b) => b - a).join("");
      const flush = h.every(c => c.s === h[0].s);
      const straight = counts === "11111" && (v[0] - v[4] === 4 || v.join() === "14,5,4,3,2");
      if (straight && flush) return v[4] === 10 ? 0 : 1;
      if (counts === "41") return 2;
      if (counts === "32") return 3;
      if (flush) return 4;
      if (straight) return 5;
      if (counts === "311") return 6;
      if (counts === "221") return 7;
      if (counts === "2111" && +Object.keys(n).find(k => n[k] === 2) >= 11) return 8;
      return -1;
    };
    const g = { name: "POKER", live: false, busy: false, msg: "JACKS OR BETTER. PRESS SPACE TO DEAL" };
    const deal = () => {
      const note = take(10); pack = deck(); cards = pack.splice(0, 5); held = [false, false, false, false, false]; win = -1; g.live = true;
      g.msg = note + "PICK CARDS TO KEEP, THEN DRAW";
    };
    const swap = () => {
      g.busy = true;
      const swapped = [0, 1, 2, 3, 4].filter(i => !held[i]);
      swapped.forEach(i => (cards[i] = null));
      seq(swapped.map(i => () => (cards[i] = pack.pop())), 140, () => {
        win = rank(cards); g.live = g.busy = false;
        const back = win < 0 ? 0 : PAY[win][1] * 10;
        chips += back;
        g.msg = (win < 0 ? "NO WIN" : PAY[win][0]) + won(back, 10);
        over();
      });
    };
    const hold = i => { if (g.live && !g.busy) held[i] = !held[i]; };
    g.buttons = () => [{ label: g.live ? "DRAW" : "DEAL", key: "enter", act: g.live ? swap : deal, on: !g.busy, lit: true }];
    g.key = k => { const i = "12345".indexOf(k); if (i < 0 || !k) return false; hold(i); return true; };
    g.click = (x, y) => { if (y < 22 || y > 52 || x % 25 > 22 || x >= 125) return false; hold(Math.floor(x / 25)); return g.live; };
    g.render = () => {
      middle("BET 10", 10);
      for (let i = 0; i < 5; i++) {
        const c = cards[i];
        c ? card(i * 25, 22, c, false, true) : card(i * 25, 22, null, true);
        if (g.live) held[i] ? text("KEEP", i * 25 + 4, 56) : text(i + 1, i * 25 + 10, 56, dim);
      }
      PAY.forEach(([name, n], i) => {
        const y = 22 + i * 7, lit = i === win && !g.live;
        if (lit) px(130, y - 1, 80, 7);
        text(name, 131, y, lit ? null : ink);
        right(n * 10, 209, y, lit ? null : dim);
      });
    };
    return g;
  })();

  const baccarat = (() => {
    const next = shoe(8);
    let player = [], banker = [], bet = "PLAYER";
    const val = c => (c.r === "A" ? 1 : c.r === "10" || isNaN(c.r) ? 0 : +c.r);
    const tot = h => h.reduce((t, c) => t + val(c), 0) % 10;
    const g = { name: "BACCARAT", live: false, busy: false, msg: "BET 20 ON PLAYER, BANKER OR TIE" };
    const settle = () => {
      const p = tot(player), b = tot(banker), side = p > b ? "PLAYER" : b > p ? "BANKER" : "TIE";
      const back = side === bet ? (bet === "TIE" ? 180 : bet === "BANKER" ? 39 : 40) : side === "TIE" ? 20 : 0;
      chips += back; g.live = g.busy = false;
      g.msg = (side === "TIE" ? "TIE" : side + " WINS") + won(back, 20);
      over();
    };
    // The third-card rules: the player draws on 0-5; the banker's draw depends on the player's third card.
    const third = () => {
      const p = tot(player), b = tot(banker), more = [];
      if (p >= 8 || b >= 8) return settle();
      let p3 = null;
      if (p <= 5) { const c = next(); p3 = val(c); more.push(() => player.push(c)); }
      const draws = p3 === null ? b <= 5 : b <= 2 || (b === 3 && p3 !== 8) || (b === 4 && p3 >= 2 && p3 <= 7) || (b === 5 && p3 >= 4 && p3 <= 7) || (b === 6 && p3 >= 6 && p3 <= 7);
      if (draws) more.push(() => banker.push(next()));
      seq(more, 380, settle);
    };
    const play = side => () => {
      const note = take(20); bet = side; player = []; banker = []; g.live = g.busy = true;
      g.msg = note + "BET 20 ON " + side;
      seq([() => player.push(next()), () => banker.push(next()), () => player.push(next()), () => banker.push(next())], 300, () => later(third, 300));
    };
    g.buttons = () => ["PLAYER", "BANKER", "TIE"].map(s => ({ label: s, key: s[0].toLowerCase(), act: play(s), on: !g.live, lit: s === bet }));
    g.render = () => {
      middle("PLAYER" + (player.length ? " " + tot(player) : ""), 10);
      hand(player, 18);
      dashes(52);
      middle("BANKER" + (banker.length ? " " + tot(banker) : ""), 56);
      hand(banker, 64);
      const pays = " PLAYER 1:1  BANKER 0.95:1  TIE 8:1 ", px0 = Math.round((W - wide(pays)) / 2);
      ctx.clearRect(px0 * S, (49 + oy) * S, wide(pays) * S, 7 * S);
      middle(pays, 50, dim);
    };
    return g;
  })();

  const roulette = (() => {
    const ORDER = [0, 32, 15, 19, 4, 21, 2, 25, 17, 34, 6, 27, 13, 36, 11, 30, 8, 23, 10, 5, 24, 16, 33, 1, 20, 14, 31, 9, 22, 18, 29, 7, 28, 12, 35, 3, 26];
    const RED = new Set([1, 3, 5, 7, 9, 12, 14, 16, 18, 19, 21, 23, 25, 27, 30, 32, 34, 36]);
    let at = 0, bet = "RED", last = [];
    const wins = (b, n) => (b === "ZERO" ? n === 0 : n !== 0 && ({ RED: RED.has(n), BLACK: !RED.has(n), ODD: n % 2 === 1, EVEN: n % 2 === 0, LOW: n <= 18, HIGH: n >= 19 })[b]);
    const cell = (n, x, y, h) => {
      const label = String(n), tx = x + 6 - wide(label) / 2, ty = y + (h - 5) / 2;
      if (n !== 0 && RED.has(n)) { px(x, y, 12, h); text(label, tx, ty, null); }
      else { box(x, y, 12, h); text(label, tx, ty); if (n === 0) { px(x + 2, y + 2); px(x + 9, y + 2); px(x + 2, y + h - 3); px(x + 9, y + h - 3); } }
    };
    const g = { name: "ROULETTE", live: false, busy: false, msg: "PICK A BET TO SPIN" };
    const land = n => {
      last = [n, ...last].slice(0, 12);
      const back = wins(bet, n) ? (bet === "ZERO" ? 360 : 20) : 0;
      chips += back; g.live = false;
      g.msg = (n === 0 ? "ZERO" : n + " " + (RED.has(n) ? "RED" : "BLACK")) + won(back, 10);
      over();
    };
    const spin = b => () => {
      const note = take(10); bet = b; g.live = true; g.msg = note + "BET 10 ON " + b;
      const r = Math.floor(Math.random() * 37), from = Math.round(at) % 37, to = from + 111 + ((r - from) % 37 + 37) % 37;
      if (still.matches) { at = to % 37; return land(ORDER[r]); }
      const t0 = performance.now();
      const tick = now => {
        const t = Math.min(1, (now - t0) / 2800);
        at = from + (to - from) * (1 - (1 - t) ** 3);
        draw();
        t < 1 ? requestAnimationFrame(tick) : ((at = to % 37), land(ORDER[r]));
      };
      requestAnimationFrame(tick);
    };
    g.buttons = () => ["RED", "BLACK", "ODD", "EVEN", "LOW", "HIGH", "ZERO"].map(s => ({ label: s, key: s[0].toLowerCase(), act: spin(s), on: !g.live, lit: s === bet }));
    g.render = () => {
      middle("BET 10", 10);
      px(103, 18, 5); px(104, 19, 3); px(105, 20);
      for (let i = Math.floor(at) - 9; i <= Math.floor(at) + 10; i++) cell(ORDER[((i % 37) + 37) % 37], 105 + (i - at) * 13 - 6, 22, 14);
      middle("FILLED = RED. OUTLINED = BLACK", 42, dim);
      if (last.length) { const x = Math.round((W - 20 - last.length * 14) / 2); text("LAST", x, 54); last.forEach((n, i) => cell(n, x + 20 + i * 14, 52, 9)); }
      middle("ZERO PAYS 35:1. THE REST PAY 1:1", 68, dim);
      middle("LOW = 1-18. HIGH = 19-36", 76, dim);
    };
    return g;
  })();

  const slots = (() => {
    const SYMBOLS = ["CHERRY", "DIAMOND", "BELL", "BAR", "SEVEN"], WEIGHT = [6, 5, 4, 3, 2];
    const PAYS = [["3 SEVENS", 100], ["3 BARS", 40], ["3 BELLS", 20], ["3 DIAMONDS", 12], ["3 CHERRIES", 5], ["2 CHERRIES", 1]];
    let reels = ["SEVEN", "SEVEN", "SEVEN"], spinning = [false, false, false], win = -1;
    const pick = () => { let r = Math.random() * 20; for (let i = 0; i < 5; i++) if ((r -= WEIGHT[i]) < 0) return SYMBOLS[i]; return "CHERRY"; };
    const symbol = (name, x, y) => {
      if (name === "DIAMOND") sprite(SUIT["♦"], x + 8, y + 8, 2);
      else if (name === "BAR") { px(x + 4, y + 10, 22, 10); text("BAR", x + 10, y + 13, null); }
      else sprite(SPRITE[name], x + 6, y + 6, 2);
    };
    const g = { name: "SLOTS", live: false, busy: false, msg: "PRESS SPACE TO SPIN" };
    const settle = () => {
      const three = reels.every(s => s === reels[0]), cherries = reels.filter(s => s === "CHERRY").length;
      win = three ? ["SEVEN", "BAR", "BELL", "DIAMOND", "CHERRY"].indexOf(reels[0]) : cherries === 2 ? 5 : -1;
      const back = win < 0 ? 0 : PAYS[win][1] * 10;
      chips += back; g.live = false;
      g.msg = (win < 0 ? "NO WIN" : PAYS[win][0]) + won(back, 10);
      over();
    };
    const spin = () => {
      const note = take(10), end = [pick(), pick(), pick()];
      g.live = true; win = -1; g.msg = note + "SPINNING";
      if (still.matches) { reels = end; return settle(); }
      spinning = [true, true, true];
      const roll = setInterval(() => { reels = reels.map((s, i) => (spinning[i] ? pick() : s)); draw(); }, 70);
      [700, 1100, 1500].forEach((ms, i) => setTimeout(() => {
        spinning[i] = false; reels[i] = end[i]; draw();
        if (i === 2) { clearInterval(roll); settle(); }
      }, ms));
    };
    g.buttons = () => [{ label: "SPIN", key: "enter", act: spin, on: !g.live, lit: true }];
    g.render = () => {
      middle("BET 10", 10);
      reels.forEach((s, i) => {
        const x = 45 + i * 45, y = 20;
        box(x - 1, y - 1, 32, 32);
        ctx.save();
        ctx.beginPath(); ctx.rect(x * S, (y + oy) * S, 30 * S, 30 * S); ctx.clip();
        symbol(s, x, y + (spinning[i] ? ((performance.now() / 9) % 12) - 6 : 0));
        ctx.restore();
      });
      PAYS.forEach(([name, n], i) => {
        const x = i < 3 ? 22 : 112, y = 60 + (i % 3) * 8, lit = i === win && !g.live;
        if (lit) px(x - 1, y - 1, 78, 7);
        text(name, x, y, lit ? null : ink);
        right(n * 10, x + 76, y, lit ? null : dim);
      });
    };
    return g;
  })();

  const games = [blackjack, poker, baccarat, roulette, slots];
  let game = blackjack, tabs = [], buttons = [];
  const choose = name => {
    const g = games.find(g => g.name === String(name || "").toUpperCase());
    if (!g) return;
    game = g; draw();
    if (OWN) { save(); try { history.replaceState(null, "", "#" + g.name.toLowerCase()); } catch {} }
  };

  const draw = () => {
    const r = cv.getBoundingClientRect();
    S = Math.max(1, Math.round((r.width || 420) / W * devicePixelRatio));
    if (cv.width !== W * S) { cv.width = W * S; cv.height = H * S; }
    // Follow the page's own colours (light text = a dark page), not just the system's setting.
    const [r0, g0, b0] = (getComputedStyle(document.body).color.match(/\d+/g) || [0, 0, 0]).map(Number), night = r0 + g0 + b0 > 384;
    ink = night ? "#acacac" : "#535353";
    dim = night ? "#5e5e5e" : "#c4c4c4";
    ctx.clearRect(0, 0, cv.width, cv.height);
    tabs = [];
    let x = Math.round((W - games.reduce((t, g) => t + wide(g.name) + 8, -2)) / 2);
    for (const g of games) {
      const w = wide(g.name) + 6;
      if (g === game) { px(x, 0, w, 9); text(g.name, x + 3, 2, null); }
      else text(g.name, x + 3, 2, dim);
      tabs.push({ g, x, w });
      x += w + 2;
    }
    const hi = "HI " + String(Math.round(best)).padStart(5, "0") + "  ", now = String(Math.round(chips)).padStart(5, "0");
    x = Math.round((W - wide(hi + now)) / 2);
    text(hi, x, 12, dim); text(now, x + wide(hi) + 1, 12);
    oy = 10; game.render(); oy = 0;
    middle(game.msg, H - 25);
    buttons = [];
    const list = game.buttons(), y = H - 12;
    x = Math.round((W - list.reduce((t, b) => t + wide(b.label) + 10, -4)) / 2);
    for (const b of list) {
      const w = wide(b.label) + 6, c = b.on ? ink : dim, solid = b.lit && b.on;
      if (solid) { px(x, y, w, 11, c); text(b.label, x + 3, y + 2, null); }
      else { box(x, y, w, 11, c); text(b.label, x + 3, y + 2, c); }
      // The key that presses it, underlined (its first letter).
      if (b.key && b.key.length === 1 && b.label[0].toLowerCase() === b.key) solid ? ctx.clearRect((x + 3) * S, (y + 8) * S, 3 * S, S) : px(x + 3, y + 8, 3, 1, c);
      buttons.push({ ...b, x, w });
      x += w + 4;
    }
    if (game.msg !== said) { said = game.msg; const m = document.getElementById("casino-msg"); if (m) m.textContent = said.toLowerCase(); }
  };

  const act = b => { b.act(); draw(); };
  cv.addEventListener("click", e => {
    const r = cv.getBoundingClientRect(), x = (e.clientX - r.left) / r.width * W, y = (e.clientY - r.top) / r.height * H;
    if (y < 10) { const t = tabs.find(t => x >= t.x && x < t.x + t.w); if (t) choose(t.g.name); return; }
    if (y >= H - 13) { const b = buttons.find(b => b.on && x >= b.x - 1 && x <= b.x + b.w + 1); if (b) act(b); return; }
    if (game.click && game.click(x, y - 10)) return draw();
    const lit = buttons.find(b => b.on && b.lit);
    if (!game.live && lit) act(lit);
  });
  addEventListener("keydown", e => {
    if (e.metaKey || e.ctrlKey || e.altKey || (e.target.closest && e.target.closest("input, textarea, select, [contenteditable]"))) return;
    const k = e.key.toLowerCase();
    if ((k === " " || k === "enter") && e.target.closest && e.target.closest("button, a, summary")) return;
    if (k === "arrowleft" || k === "arrowright") {
      e.preventDefault();
      choose(games[(games.indexOf(game) + (k === "arrowright" ? 1 : games.length - 1)) % games.length].name);
      return;
    }
    if (e.repeat) return;
    const all = game.buttons(), b = k === " " ? all.find(b => b.on && b.lit) : all.find(b => b.on && b.key === k);
    if (b) {
      e.preventDefault();
      // A short pause after a round ends, so pressing Space again right away doesn't skip past the result.
      if (k !== " " || performance.now() - ended > 500) act(b);
    } else if (game.key && game.key(k)) { e.preventDefault(); draw(); }
  }, true);
  matchMedia("(prefers-color-scheme: dark)").addEventListener("change", draw);
  addEventListener("resize", draw);
  if (OWN) addEventListener("hashchange", () => choose(location.hash.slice(1)));
  // The offline page: back online mid-round, the round finishes first, then the page loads.
  window.casinoOnline = () => {
    if (!games.some(g => g.live)) return location.reload();
    backOnline = true;
    const h = document.querySelector("h1"), p = document.querySelector("main > p");
    if (h) h.textContent = "You're back online";
    if (p) p.textContent = "The page loads when this round ends.";
  };
  let start = OWN && location.hash.slice(1);
  try { if (!start && OWN) start = JSON.parse(localStorage.getItem("casino") || "{}").game; } catch {}
  game = games.find(g => g.name === String(start || "").toUpperCase()) || blackjack;
  draw();
})();
