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
    "+":"000010111010000",":":"000010000010000","=":"000111000111000","'":"010010000000000"," ":"000000000000000",
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
  let oy = 0, S = 2, ink = "#535353", dim = "#cfcfcf", chips = 100, best = 100, ended = -Infinity, backOnline = false, said = "", bots = 1;
  try { const s = OWN && JSON.parse(localStorage.getItem("casino") || "{}"); if (s && s.chips > 0) { chips = s.chips; best = s.best || chips; } if (s && s.bots >= 1 && s.bots <= 5) bots = s.bots; } catch {}
  const save = () => { best = Math.max(best, chips); if (OWN) try { localStorage.setItem("casino", JSON.stringify({ chips, best, bots, game: game.name })); } catch {} };

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
  const MINI = { "♠": ["00100", "01110", "11111", "00100", "01110"], "♥": ["01010", "11111", "11111", "01110", "00100"], "♦": ["00100", "01110", "11111", "01110", "00100"], "♣": ["01110", "01110", "11111", "00100", "01110"] };
  const mini = (x, y, c, down) => {
    ctx.clearRect(x * S, (y + oy) * S, 13 * S, 15 * S);
    box(x, y, 13, 15);
    if (down) { for (let j = 2; j < 13; j++) for (let i = 2; i < 11; i++) if ((i + j) % 4 === 0) px(x + i, y + j); return; }
    text(c.r, x + 2, y + 2);
    sprite(MINI[c.s], x + 2, y + 8);
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

  // Texas hold'em against bots, fixed limit: blinds 5/10, bets of 10 before the turn and 20 after, four bets a round at most.
  const poker = (() => {
    const NAMES = ["HIGH CARD", "PAIR", "TWO PAIR", "THREE OF A KIND", "STRAIGHT", "FLUSH", "FULL HOUSE", "FOUR OF A KIND", "STRAIGHT FLUSH"];
    const value = r => ({ A: 14, K: 13, Q: 12, J: 11 })[r] || +r;
    const eval5 = h => {
      const v = h.map(c => value(c.r)).sort((a, b) => b - a), n = {};
      v.forEach(x => (n[x] = (n[x] || 0) + 1));
      const groups = Object.keys(n).map(Number).sort((a, b) => n[b] - n[a] || b - a), shape = groups.map(x => n[x]).join("");
      const flush = h.every(c => c.s === h[0].s), five = groups.length === 5;
      const top = five && v[0] - v[4] === 4 ? v[0] : five && v.join() === "14,5,4,3,2" ? 5 : 0;
      if (top && flush) return [8, top];
      if (shape === "41") return [7, ...groups];
      if (shape === "32") return [6, ...groups];
      if (flush) return [5, ...v];
      if (top) return [4, top];
      if (shape === "311") return [3, ...groups];
      if (shape === "221") return [2, ...groups];
      if (shape === "2111") return [1, ...groups];
      return [0, ...v];
    };
    const cmp = (a, b) => { for (let i = 0; i < Math.max(a.length, b.length); i++) { const d = (a[i] || 0) - (b[i] || 0); if (d) return d; } return 0; };
    // The best five of up to seven cards.
    const bestOf = cards => {
      let top = null;
      const k = cards.length;
      for (let a = 0; a < k; a++) for (let b = a + 1; b < k; b++) for (let c = b + 1; c < k; c++) for (let d = c + 1; d < k; d++) for (let e = d + 1; e < k; e++) {
        const r = eval5([cards[a], cards[b], cards[c], cards[d], cards[e]]);
        if (!top || cmp(r, top) > 0) top = r;
      }
      return top;
    };
    let n = 0, dealer = 0, holes = [], board = [], pack = [], put = [], paid = [], folded = [], acted = [], says = [];
    let bet = 0, raises = 0, street = 0, turn = 0, allIn = false, showdown = false, fast = false, yourTurn = false;
    const g = { name: "POKER", live: false, busy: false, msg: "TEXAS HOLD 'EM. PRESS SPACE TO DEAL" };
    const size = () => (street < 2 ? 10 : 20);
    const active = () => [...Array(n).keys()].filter(i => !folded[i]);
    const pot = () => paid.reduce((a, b) => a + b, 0);
    // You play with your own chips (short, you're all in); the bots never run out.
    const pay = (i, amount) => {
      if (i === 0) { amount = Math.min(amount, chips); chips -= amount; if (!chips) allIn = true; }
      put[i] += amount; paid[i] += amount;
    };
    const needs = i => !folded[i] && !(i === 0 && allIn) && (put[i] < bet || !acted[i]);
    const nextFrom = i => { for (let k = 1; k <= n; k++) { const j = (i + k) % n; if (needs(j)) return j; } return -1; };
    const act = (i, kind) => {
      const owe = bet - put[i];
      if (kind === "fold" && owe > 0) { folded[i] = true; says[i] = "FOLD"; }
      else if (kind === "raise" && raises < 4 && !(i === 0 && chips <= owe)) {
        bet += size(); raises++; pay(i, bet - put[i]);
        acted = acted.map(() => false);
        says[i] = owe ? "RAISE" : "BET " + size();
      } else { pay(i, owe); says[i] = owe ? "CALL " + owe : "CHECK"; }
      acted[i] = true;
    };
    const think = p => {
      const h = holes[p], owe = bet - put[p];
      let s;
      if (!board.length) {
        const [a, b] = h.map(c => value(c.r)).sort((x, y) => y - x);
        s = a === b ? 0.5 + a / 30 : (a + b) / 40 + (h[0].s === h[1].s ? 0.06 : 0) + (a - b === 1 ? 0.04 : 0);
      } else {
        const mine = bestOf([...h, ...board]), table = board.length === 5 ? bestOf(board) : null;
        s = [0.2, 0.45, 0.62, 0.74, 0.8, 0.85, 0.92, 0.97, 1][mine[0]] + (mine[0] === 0 ? mine[1] / 100 : 0);
        if (table && cmp(mine, table) === 0) s = 0.2;
        else if (mine[0] === 1 && !h.some(c => value(c.r) === mine[1])) s = 0.25;
      }
      s += (Math.random() - 0.5) * 0.2;
      if (Math.random() < 0.05) return "raise";
      if (s > 0.72) return Math.random() < 0.7 ? "raise" : "call";
      if (s > 0.42) return Math.random() < 0.15 ? "raise" : "call";
      if (!owe) return "call";
      return s > 0.3 && Math.random() < 0.5 ? "call" : "fold";
    };
    const finish = () => {
      const left = active();
      let winners, why = "";
      if (left.length === 1) winners = left;
      else {
        showdown = true;
        const ranks = left.map(i => bestOf([...holes[i], ...board]));
        const top = ranks.reduce((t, r) => (cmp(r, t) > 0 ? r : t));
        winners = left.filter((_, k) => cmp(ranks[k], top) === 0);
        why = " WITH " + NAMES[top[0]];
      }
      let mine = 0;
      if (winners.includes(0)) {
        // All in, you can only win what you put in from each player.
        const reach = allIn ? paid.reduce((t, x) => t + Math.min(x, paid[0]), 0) : pot();
        mine = Math.floor(reach / winners.length);
        chips += mine;
      }
      g.live = g.busy = yourTurn = false;
      g.msg = (winners.length > 1 ? (winners.includes(0) ? "YOU SPLIT " + mine : "SPLIT POT") : winners[0] === 0 ? "YOU WIN " + mine : "BOT " + winners[0] + " WINS " + pot()) + why;
      over();
    };
    const wait = () => later(step, fast ? 120 : 520);
    const nextStreet = () => {
      if (street === 3) return finish();
      street++;
      board.push(...pack.splice(0, street === 1 ? 3 : 1));
      bet = 0; raises = 0; turn = dealer;
      put = put.map(() => 0); acted = acted.map(() => false); says = says.map((s, i) => (folded[i] ? "FOLD" : ""));
      draw(); wait();
    };
    function step() {
      if (active().length === 1) return finish();
      const p = nextFrom(turn);
      if (p < 0) return nextStreet();
      turn = p;
      if (p === 0) {
        yourTurn = true; g.busy = false;
        const owe = Math.min(bet - put[0], chips);
        g.msg = owe ? "YOUR TURN. " + owe + " TO CALL" : "YOUR TURN";
        return draw();
      }
      g.busy = true; act(p, think(p)); draw(); wait();
    }
    const deal = () => {
      n = bots + 1;
      const note = chips < 10 ? ((chips = 100), "THE HOUSE GIVES YOU 100. ") : "";
      dealer = (dealer + 1) % n; pack = deck(); board = [];
      holes = [...Array(n)].map(() => pack.splice(0, 2));
      put = Array(n).fill(0); paid = Array(n).fill(0); folded = Array(n).fill(false); acted = Array(n).fill(false); says = Array(n).fill("");
      bet = 10; raises = 1; street = 0; allIn = showdown = fast = yourTurn = false; g.live = g.busy = true;
      const sb = n === 2 ? dealer : (dealer + 1) % n, bb = (sb + 1) % n;
      pay(sb, 5); pay(bb, 10); says[sb] = "SMALL 5"; says[bb] = "BIG 10";
      turn = bb; g.msg = note + "BLINDS 5 AND 10";
      draw(); wait();
    };
    const you = kind => () => {
      if (!yourTurn) return;
      yourTurn = false; g.busy = true;
      act(0, kind);
      if (folded[0]) fast = true;
      g.msg = folded[0] ? "YOU FOLDED" : "";
      draw(); wait();
    };
    const setBots = d => { bots = Math.min(5, Math.max(1, bots + d)); g.msg = bots + (bots === 1 ? " BOT" : " BOTS") + ". PRESS SPACE TO DEAL"; save(); };
    g.buttons = () => {
      if (!g.live) return [
        { label: "DEAL", key: "enter", act: deal, on: true, lit: true },
        { label: "- BOT", key: "-", act: () => setBots(-1), on: bots > 1 },
        { label: "+ BOT", key: "+", act: () => setBots(1), on: bots < 5 },
      ];
      const owe = bet - put[0];
      return [
        { label: "FOLD", key: "f", act: you("fold"), on: yourTurn && owe > 0 },
        { label: owe ? "CALL " + Math.min(owe, chips) : "CHECK", key: "c", act: you("call"), on: yourTurn, lit: true },
        { label: owe ? "RAISE" : "BET", key: owe ? "r" : "b", act: you("raise"), on: yourTurn && raises < 4 && chips > owe },
      ];
    };
    g.key = k => { if (k !== "=" || g.live) return false; setBots(1); return true; };
    g.render = () => {
      const seats = n - 1 === bots ? n - 1 : bots, playing = n - 1 === bots, w = 40;
      const x0 = Math.round((W - (seats * w + (seats - 1) * 2)) / 2);
      for (let k = 1; k <= seats; k++) {
        const x = x0 + (k - 1) * (w + 2), out = playing && folded[k], name = "BOT " + k + (playing && dealer === k ? " D" : "");
        if (g.live && g.busy && turn === k) { px(x, 10, w, 8); text(name, x + 2, 11, null); }
        else text(name, x + 2, 11, out ? dim : ink);
        if (!playing) continue;
        text(says[k] || "", x + 2, 20, dim);
        if (!out && holes[k]) holes[k].forEach((c, j) => mini(x + 2 + j * 15, 27, c, !showdown));
      }
      const bx = 14, by = 54;
      text("POT " + (playing ? pot() : 0), bx, 46);
      for (let i = 0; i < 5; i++) board[i] && playing ? card(bx + i * 25, by, board[i], false, true) : box(bx + i * 25, by, 22, 30, dim);
      const label = "YOU" + (playing && dealer === 0 ? " D" : "");
      if (yourTurn) { px(149, 45, wide(label) + 2, 7); text(label, 150, 46, null); }
      else text(label, 150, 46, playing && folded[0] ? dim : ink);
      if (playing) text(says[0] || "", 150 + wide(label) + 5, 46, dim);
      if (playing && holes[0]) holes[0].forEach((c, j) => card(150 + j * 24, by, c, false, true));
      else { box(150, by, 22, 30, dim); box(174, by, 22, 30, dim); }
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
      if (/^[a-z]$/.test(b.key || "") && b.label[0].toLowerCase() === b.key) solid ? ctx.clearRect((x + 3) * S, (y + 8) * S, 3 * S, S) : px(x + 3, y + 8, 3, 1, c);
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
