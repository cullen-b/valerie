// Games drawn as pixel art on <canvas id="casino">: blackjack, video poker, baccarat, roulette, slots, hearts, spades and bridge.
// Used by the "You're offline" page (pages.offline_game) and valerie://page/casino (the play_* commands), where friends
// can join through an invite link (pages.game_invites, src/game_host.rs): this tab runs every game and draws each friend's
// seat for them, so a friend's browser only ever sees what that seat may see.
(() => {
  const W = 210, H = 148, OY = 20, cv = document.getElementById("casino");
  if (!cv) return;
  let ctx = cv.getContext("2d");
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
  // The players: you (k 0) and friends who joined by link (k 1, 2, …). `me` is the seat being drawn or acting right now.
  const host = { id: "", k: 0, night: false, help: false, get chips() { return chips; }, set chips(v) { chips = v; } };
  const people = [host];
  let me = 0;
  // A game's seat for player k (game.room seats; game.order picks which seat each player gets), or -1 when it's full.
  const seatOf = (g, k) => (k < (g.room || 1) ? (g.order ? g.order[k] : k) : -1);
  const at = (g, seat) => people.find(p => seatOf(g, p.k) === seat);
  const human = (g, seat) => seat >= 0 && !!at(g, seat);
  const seated = g => people.filter(p => seatOf(g, p.k) >= 0);
  const nameOf = p => (p === at(game, me) ? "YOU" : "P" + (p.k + 1));
  const save = () => { best = Math.max(best, chips); if (OWN) try { localStorage.setItem("casino", JSON.stringify({ chips, best, bots, game: game.name })); } catch {} };

  // Drawing, in the 210×148 grid of big pixels.
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
  const mini = (x, y, c, down, col = ink) => {
    ctx.clearRect(x * S, (y + oy) * S, 13 * S, 15 * S);
    box(x, y, 13, 15, col);
    if (down) { for (let j = 2; j < 13; j++) for (let i = 2; i < 11; i++) if ((i + j) % 4 === 0) px(x + i, y + j, 1, 1, col); return; }
    text(c.r, x + 2, y + 2, col);
    sprite(MINI[c.s], x + 2, y + 8, 1, col);
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
  const take = (p, n) => { let note = ""; if (p.chips < n) { p.chips = 100; note = "THE HOUSE GIVES YOU 100. "; } p.chips -= n; return note; };
  // Shared tables (baccarat, roulette): a round starts once everyone seated has bet, or 5 seconds after the first bet.
  const whenBetsIn = (g, bets, go) => {
    let timer = 0, n = 0;
    const start = () => { clearTimeout(timer); timer = 0; go(); };
    const tick = () => { if (n <= 0) return start(); g.msg = "BETS CLOSE IN " + n--; draw(); timer = setTimeout(tick, 1000); };
    return () => {
      if (Object.keys(bets()).length >= seated(g).length) return start();
      if (!timer) { n = 5; tick(); }
    };
  };
  const won = (back, bet) => back > bet ? " +" + (back - bet) : back === bet ? " BET BACK" : " -" + bet;
  const over = () => {
    ended = performance.now();
    save(); draw();
    if (backOnline && !games.some(g => g.live)) setTimeout(() => location.reload(), 900);
  };

  const blackjack = (() => {
    const next = shoe(6);
    // A hand for each player at the table: {who, seat, cards, stake, done}.
    let dealer = [], hands = [], hide = true;
    const total = h => { let t = 0, a = 0; for (const c of h) { t += c.r === "A" ? 11 : isNaN(c.r) ? 10 : +c.r; if (c.r === "A") a++; } while (t > 21 && a) { t -= 10; a--; } return t; };
    const natural = h => h.length === 2 && total(h) === 21;
    const g = { name: "BLACKJACK", room: 6, live: false, busy: false, msg: "PRESS SPACE TO DEAL", notes: {} };
    const mine = () => hands.find(h => h.who === at(g, me));
    const settle = () => {
      hide = false; g.live = g.busy = false;
      const d = total(dealer);
      for (const h of hands) {
        const p = total(h.cards), bj = natural(h.cards) && !natural(dealer);
        const back = bj ? h.stake * 2.5 : p > 21 || (natural(dealer) && !natural(h.cards)) ? 0 : d > 21 || p > d ? h.stake * 2 : p < d ? 0 : h.stake;
        h.who.chips += back;
        g.notes[h.seat] = (back === h.stake ? "PUSH" : bj ? "BLACKJACK!" : p > 21 ? "BUST" : d > 21 ? "DEALER BUSTS" : back ? "YOU WIN" : "DEALER WINS") + won(back, h.stake);
      }
      g.msg = d > 21 ? "DEALER BUSTS" : "DEALER HAS " + d;
      over();
    };
    const dealerPlays = () => {
      g.busy = true; hide = false; g.msg = "DEALER PLAYS"; g.notes = {};
      const step = () => { if (total(dealer) >= 17) return settle(); dealer.push(next()); draw(); later(step, 380); };
      later(step, 380);
    };
    // Once every hand is done, the dealer plays (unless every hand already busted or has blackjack).
    const check = () => {
      if (!g.live || g.busy || !hands.every(h => h.done)) return;
      hands.every(h => total(h.cards) > 21 || natural(h.cards)) ? settle() : dealerPlays();
    };
    const deal = () => {
      if (g.live) return;
      dealer = [next(), next()]; hide = true; g.live = true; g.busy = false; g.notes = {}; g.msg = "HIT OR STAND?";
      hands = seated(g).map(who => {
        const seat = seatOf(g, who.k);
        g.notes[seat] = take(who, 10) + "HIT OR STAND?";
        return { who, seat, cards: [next(), next()], stake: 10, done: false };
      });
      if (natural(dealer)) return settle();
      for (const h of hands) if (natural(h.cards)) { h.done = true; g.notes[h.seat] = "BLACKJACK! WAITING FOR THE OTHERS"; }
      check();
    };
    const done = (h, why) => { h.done = true; g.notes[h.seat] = why + "WAITING FOR THE OTHERS"; check(); };
    const hit = () => {
      const h = mine();
      if (!h || h.done || g.busy) return;
      h.cards.push(next());
      const t = total(h.cards);
      if (t > 21) done(h, "BUST. "); else if (t === 21) done(h, "21. ");
    };
    const stand = () => { const h = mine(); if (h && !h.done && !g.busy) done(h, ""); };
    const double = () => {
      const h = mine();
      if (!h || h.done || g.busy || h.cards.length !== 2 || h.who.chips < 10) return;
      h.who.chips -= 10; h.stake = 20; h.cards.push(next());
      done(h, total(h.cards) > 21 ? "BUST. " : "");
    };
    g.changed = p => { const h = hands.find(h => h.who === p); if (h && !h.done) { h.done = true; check(); } };
    g.buttons = () => {
      const h = mine(), play = g.live && !g.busy && h && !h.done;
      return [
        { label: "DEAL", key: "enter", act: deal, on: !g.live && !!at(g, me), lit: true },
        { label: "HIT", key: "h", act: hit, on: play, lit: true },
        { label: "STAND", key: "s", act: stand, on: play },
        { label: "DOUBLE", key: "d", act: double, on: play && h.cards.length === 2 && h.who.chips >= 10 },
      ];
    };
    g.render = () => {
      const h = mine(), cards = h ? h.cards : [];
      middle("DEALER" + (dealer.length ? " " + (g.live && hide ? total([dealer[0]]) : total(dealer)) : ""), 10);
      hand(dealer, 18, g.live && hide);
      dashes(52);
      middle("YOU" + (cards.length ? " " + total(cards) : ""), 56);
      hand(cards, 64);
      // The other players at the table, in a line under your hand.
      const others = hands.filter(x => x !== h).map(x => nameOf(x.who) + " " + (total(x.cards) > 21 ? "BUST" : total(x.cards)) + (x.done && g.live ? "." : ""));
      if (others.length) middle(others.join("  "), 97, dim);
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
    let n = 0, dealer = 0, holes = [], board = [], pack = [], put = [], paid = [], folded = [], acted = [], says = [], allIn = [];
    let bet = 0, raises = 0, street = 0, turn = 0, showdown = false, fast = false, waiting = -1, why = "";
    // The players in this hand, by seat; the other seats are bots.
    let hum = new Map();
    const g = { name: "POKER", room: 6, live: false, busy: false, msg: "TEXAS HOLD 'EM. PRESS SPACE TO DEAL", notes: {} };
    const size = () => (street < 2 ? 10 : 20);
    const active = () => [...Array(n).keys()].filter(i => !folded[i]);
    const pot = () => paid.reduce((a, b) => a + b, 0);
    const inHand = () => g.live && hum.has(me) && hum.get(me) === at(g, me);
    const seatName = i => (hum.has(i) ? nameOf(hum.get(i)) : "BOT " + i);
    // As many seats as bots wanted plus you, or more when more people are playing.
    const want = () => Math.min(6, Math.max(bots + 1, ...seated(g).map(p => seatOf(g, p.k) + 1)));
    // People play with their own chips (short, they're all in); the bots never run out.
    const pay = (i, amount) => {
      const who = hum.get(i);
      if (who) { amount = Math.min(amount, who.chips); who.chips -= amount; if (!who.chips) allIn[i] = true; }
      put[i] += amount; paid[i] += amount;
    };
    const needs = i => !folded[i] && !allIn[i] && (put[i] < bet || !acted[i]);
    const nextFrom = i => { for (let k = 1; k <= n; k++) { const j = (i + k) % n; if (needs(j)) return j; } return -1; };
    const act = (i, kind) => {
      const owe = bet - put[i];
      if (kind === "fold" && owe > 0) { folded[i] = true; says[i] = "FOLD"; }
      else if (kind === "raise" && raises < 4 && !(hum.has(i) && hum.get(i).chips <= owe)) {
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
      let winners;
      why = "";
      if (left.length === 1) winners = left;
      else {
        showdown = true;
        const ranks = left.map(i => bestOf([...holes[i], ...board]));
        const top = ranks.reduce((t, r) => (cmp(r, t) > 0 ? r : t));
        winners = left.filter((_, k) => cmp(ranks[k], top) === 0);
        why = " WITH " + NAMES[top[0]];
      }
      const pays = {};
      for (const w of winners.filter(w => hum.has(w))) {
        // All in, a player can only win what they put in from each player.
        const reach = allIn[w] ? paid.reduce((t, x) => t + Math.min(x, paid[w]), 0) : pot();
        pays[w] = Math.floor(reach / winners.length);
        hum.get(w).chips += pays[w];
      }
      g.live = g.busy = false; waiting = -1; g.notes = {};
      const all = pot(), w0 = winners[0], split = winners.length > 1;
      g.msg = () => (split ? (me in pays ? "YOU SPLIT " + pays[me] : "SPLIT POT") : w0 === me && me in pays ? "YOU WIN " + pays[me] : seatName(w0) + " WINS " + all) + why;
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
      if (hum.has(p)) {
        waiting = p; g.busy = false;
        const owe = Math.min(bet - put[p], hum.get(p).chips);
        g.notes = { [p]: owe ? "YOUR TURN. " + owe + " TO CALL" : "YOUR TURN" };
        g.msg = () => seatName(p) + " TO PLAY";
        return draw();
      }
      g.busy = true; act(p, think(p)); draw(); wait();
    }
    const deal = () => {
      if (g.live || !at(g, me)) return;
      n = want();
      hum = new Map(seated(g).map(p => [seatOf(g, p.k), p]));
      g.notes = {};
      for (const [i, p] of hum) if (p.chips < 10) { p.chips = 100; g.notes[i] = "THE HOUSE GIVES YOU 100. BLINDS 5 AND 10"; }
      dealer = (dealer + 1) % n; pack = deck(); board = [];
      holes = [...Array(n)].map(() => pack.splice(0, 2));
      put = Array(n).fill(0); paid = Array(n).fill(0); folded = Array(n).fill(false); acted = Array(n).fill(false); says = Array(n).fill(""); allIn = Array(n).fill(false);
      bet = 10; raises = 1; street = 0; showdown = fast = false; waiting = -1; g.live = g.busy = true;
      const sb = n === 2 ? dealer : (dealer + 1) % n, bb = (sb + 1) % n;
      pay(sb, 5); pay(bb, 10); says[sb] = "SMALL 5"; says[bb] = "BIG 10";
      turn = bb; g.msg = "BLINDS 5 AND 10";
      draw(); wait();
    };
    // Once no person is left in the hand, the bots play it out quickly.
    const hurry = () => { if (![...hum.keys()].some(i => !folded[i])) fast = true; };
    const you = kind => () => {
      if (waiting !== me || !inHand()) return;
      waiting = -1; g.busy = true;
      act(me, kind);
      hurry();
      g.notes = { [me]: folded[me] ? "YOU FOLDED" : "" }; g.msg = "";
      draw(); wait();
    };
    // Someone who leaves mid-hand folds.
    g.changed = p => {
      const i = [...hum].find(([, x]) => x === p)?.[0];
      if (i === undefined || !g.live) return;
      hum.delete(i); folded[i] = true; says[i] = "LEFT";
      hurry();
      if (waiting === i) { waiting = -1; g.busy = true; wait(); }
    };
    const setBots = d => { bots = Math.min(5, Math.max(1, bots + d)); g.msg = bots + (bots === 1 ? " BOT" : " BOTS") + ". PRESS SPACE TO DEAL"; g.notes = {}; save(); };
    g.buttons = () => {
      if (!g.live) return [
        { label: "DEAL", key: "enter", act: deal, on: !!at(g, me), lit: true },
        { label: "- BOT", key: "-", act: () => setBots(-1), on: bots > 1 },
        { label: "+ BOT", key: "+", act: () => setBots(1), on: bots < 5 },
      ];
      const mine = inHand() && waiting === me, owe = mine ? bet - put[me] : 0, purse = mine ? hum.get(me).chips : 0;
      return [
        { label: "FOLD", key: "f", act: you("fold"), on: mine && owe > 0 },
        { label: owe ? "CALL " + Math.min(owe, purse) : "CHECK", key: "c", act: you("call"), on: mine, lit: true },
        { label: owe ? "RAISE" : "BET", key: owe ? "r" : "b", act: you("raise"), on: mine && raises < 4 && purse > owe },
      ];
    };
    g.key = k => { if (k !== "=" || g.live) return false; setBots(1); return true; };
    // Everyone else along the top, starting with the seat after yours; your seat at the bottom right.
    g.render = () => {
      const playing = n === want() && n > 0, total = playing ? n : want(), seats = total - 1, w = 40, mine = playing && hum.has(me) && hum.get(me) === at(g, me);
      const x0 = Math.round((W - (seats * w + (seats - 1) * 2)) / 2);
      for (let j = 1; j <= seats; j++) {
        const k = (Math.max(me, 0) + j) % total, x = x0 + (j - 1) * (w + 2), out = playing && folded[k];
        const name = (playing ? seatName(k) : human(g, k) ? nameOf(at(g, k)) : "BOT " + k) + (playing && dealer === k ? " D" : "");
        if (g.live && turn === k && (g.busy || waiting === k)) { px(x, 10, w, 8); text(name, x + 2, 11, null); }
        else text(name, x + 2, 11, out ? dim : ink);
        if (!playing) continue;
        text(says[k] || "", x + 2, 20, dim);
        if (!out && holes[k]) holes[k].forEach((c, j) => mini(x + 2 + j * 15, 27, c, !showdown));
      }
      const bx = 14, by = 54;
      text("POT " + (playing ? pot() : 0), bx, 46);
      for (let i = 0; i < 5; i++) board[i] && playing ? card(bx + i * 25, by, board[i], false, true) : box(bx + i * 25, by, 22, 30, dim);
      const label = "YOU" + (playing && dealer === me ? " D" : "");
      if (mine && waiting === me) { px(149, 45, wide(label) + 2, 7); text(label, 150, 46, null); }
      else text(label, 150, 46, mine && folded[me] ? dim : ink);
      if (mine) text(says[me] || "", 150 + wide(label) + 5, 46, dim);
      if (mine && holes[me]) holes[me].forEach((c, j) => card(150 + j * 24, by, c, false, true));
      else { box(150, by, 22, 30, dim); box(174, by, 22, 30, dim); }
    };
    return g;
  })();

  const baccarat = (() => {
    const next = shoe(8);
    // bets: seat → {who, side}; last: each seat's last side, lit on its buttons.
    let player = [], banker = [], bets = {}, last = {};
    const val = c => (c.r === "A" ? 1 : c.r === "10" || isNaN(c.r) ? 0 : +c.r);
    const tot = h => h.reduce((t, c) => t + val(c), 0) % 10;
    const g = { name: "BACCARAT", room: 6, live: false, busy: false, msg: "BET 20 ON PLAYER, BANKER OR TIE", notes: {} };
    const settle = () => {
      const p = tot(player), b = tot(banker), side = p > b ? "PLAYER" : b > p ? "BANKER" : "TIE";
      for (const [seat, x] of Object.entries(bets)) {
        const back = side === x.side ? (side === "TIE" ? 180 : side === "BANKER" ? 39 : 40) : side === "TIE" ? 20 : 0;
        x.who.chips += back;
        g.notes[seat] = (side === "TIE" ? "TIE" : side + " WINS") + won(back, 20);
      }
      g.msg = side === "TIE" ? "TIE" : side + " WINS";
      g.live = g.busy = false; bets = {};
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
    const go = () => {
      player = []; banker = []; g.live = g.busy = true; g.msg = "NO MORE BETS";
      seq([() => player.push(next()), () => banker.push(next()), () => player.push(next()), () => banker.push(next())], 300, () => later(third, 300));
    };
    const ready = whenBetsIn(g, () => bets, go);
    const play = side => () => {
      const who = at(g, me);
      if (g.live || !who || bets[me]) return;
      if (!Object.keys(bets).length) g.notes = {};
      g.notes[me] = take(who, 20) + "BET 20 ON " + side;
      bets[me] = { who, side }; last[me] = side;
      ready();
    };
    g.buttons = () => ["PLAYER", "BANKER", "TIE"].map(s => ({ label: s, key: s[0].toLowerCase(), act: play(s), on: !g.live && !bets[me] && !!at(g, me), lit: s === (last[me] || "PLAYER") }));
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
    // spot: where the wheel has turned to; bets: seat → {who, side}; mine: each seat's last bet, lit on its buttons.
    let spot = 0, bets = {}, mine = {}, last = [];
    const wins = (b, n) => (b === "ZERO" ? n === 0 : n !== 0 && ({ RED: RED.has(n), BLACK: !RED.has(n), ODD: n % 2 === 1, EVEN: n % 2 === 0, LOW: n <= 18, HIGH: n >= 19 })[b]);
    const cell = (n, x, y, h) => {
      const label = String(n), tx = x + 6 - wide(label) / 2, ty = y + (h - 5) / 2;
      if (n !== 0 && RED.has(n)) { px(x, y, 12, h); text(label, tx, ty, null); }
      else { box(x, y, 12, h); text(label, tx, ty); if (n === 0) { px(x + 2, y + 2); px(x + 9, y + 2); px(x + 2, y + h - 3); px(x + 9, y + h - 3); } }
    };
    const g = { name: "ROULETTE", room: 6, live: false, busy: false, msg: "PICK A BET TO SPIN", notes: {} };
    const land = n => {
      last = [n, ...last].slice(0, 12);
      const where = n === 0 ? "ZERO" : n + " " + (RED.has(n) ? "RED" : "BLACK");
      for (const [seat, x] of Object.entries(bets)) {
        const back = wins(x.side, n) ? (x.side === "ZERO" ? 360 : 20) : 0;
        x.who.chips += back;
        g.notes[seat] = where + won(back, 10);
      }
      g.msg = where; g.live = false; bets = {};
      over();
    };
    const go = () => {
      g.live = true; g.msg = "NO MORE BETS";
      const r = Math.floor(Math.random() * 37), from = Math.round(spot) % 37, to = from + 111 + ((r - from) % 37 + 37) % 37;
      if (still.matches) { spot = to % 37; return land(ORDER[r]); }
      const t0 = performance.now();
      const tick = now => {
        const t = Math.min(1, (now - t0) / 2800);
        spot = from + (to - from) * (1 - (1 - t) ** 3);
        draw();
        t < 1 ? requestAnimationFrame(tick) : ((spot = to % 37), land(ORDER[r]));
      };
      requestAnimationFrame(tick);
    };
    const ready = whenBetsIn(g, () => bets, go);
    const spin = b => () => {
      const who = at(g, me);
      if (g.live || !who || bets[me]) return;
      if (!Object.keys(bets).length) g.notes = {};
      g.notes[me] = take(who, 10) + "BET 10 ON " + b;
      bets[me] = { who, side: b }; mine[me] = b;
      ready();
    };
    g.buttons = () => ["RED", "BLACK", "ODD", "EVEN", "LOW", "HIGH", "ZERO"].map(s => ({ label: s, key: s[0].toLowerCase(), act: spin(s), on: !g.live && !bets[me] && !!at(g, me), lit: s === (mine[me] || "RED") }));
    g.render = () => {
      middle("BET 10", 10);
      px(103, 18, 5); px(104, 19, 3); px(105, 20);
      for (let i = Math.floor(spot) - 9; i <= Math.floor(spot) + 10; i++) cell(ORDER[((i % 37) + 37) % 37], 105 + (i - spot) * 13 - 6, 22, 14);
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
    // One player (you); friends watch.
    const g = { name: "SLOTS", watch: true, live: false, busy: false, msg: "PRESS SPACE TO SPIN" };
    const settle = () => {
      const three = reels.every(s => s === reels[0]), cherries = reels.filter(s => s === "CHERRY").length;
      win = three ? ["SEVEN", "BAR", "BELL", "DIAMOND", "CHERRY"].indexOf(reels[0]) : cherries === 2 ? 5 : -1;
      const back = win < 0 ? 0 : PAYS[win][1] * 10;
      chips += back; g.live = false;
      g.msg = (win < 0 ? "NO WIN" : PAYS[win][0]) + won(back, 10);
      over();
    };
    const spin = () => {
      if (g.live || me !== 0) return;
      const note = take(host, 10), end = [pick(), pick(), pick()];
      g.live = true; win = -1; g.msg = note + "SPINNING";
      if (still.matches) { reels = end; return settle(); }
      spinning = [true, true, true];
      const roll = setInterval(() => { reels = reels.map((s, i) => (spinning[i] ? pick() : s)); draw(); }, 70);
      [700, 1100, 1500].forEach((ms, i) => setTimeout(() => {
        spinning[i] = false; reels[i] = end[i]; draw();
        if (i === 2) { clearInterval(roll); settle(); }
      }, ms));
    };
    g.buttons = () => [{ label: "SPIN", key: "enter", act: spin, on: !g.live && me === 0, lit: true }];
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

  // Trick-taking games (hearts, spades, bridge): four seats, whoever's looking at the bottom, play goes clockwise.
  // Seat 0 is yours; friends sit across from you first (your partner in spades and bridge), then left and right.
  const COMPASS = ["SOUTH", "WEST", "NORTH", "EAST"], SUITS = ["♠", "♥", "♣", "♦"], TRICK_ORDER = [0, 2, 1, 3];
  const nm = p => (p === me ? "YOU" : COMPASS[p]);
  const takes = w => (w === me ? "YOU TAKE IT" : nm(w) + " TAKES IT");
  const AT = [[98, 43], [80, 30], [98, 17], [116, 30]], HY = 73;
  // A small card with the big suit symbols, so spades and clubs (and hearts and diamonds) are easy to tell apart.
  const pip = (x, y, c, col = ink) => {
    ctx.clearRect(x * S, (y + oy) * S, 13 * S, 17 * S);
    box(x, y, 13, 17, col);
    text(c.r, x + 2, y + 2, col);
    sprite(SUIT[c.s], x + 3, y + 8, 1, col);
  };
  const BIG = {
    "♠":["000010000","000111000","001111100","011111110","111111111","111111111","011010110","000010000","000111000"],
    "♥":["011000110","111101111","111111111","111111111","011111110","001111100","000111000","000010000","000000000"],
    "♦":["000010000","000111000","001111100","011111110","111111111","011111110","001111100","000111000","000010000"],
    "♣":["000111000","001111100","001111100","110111011","111111111","110111011","000010000","000111000","001111100"],
  };
  // A card in your hand: double-size rank and a 9-pixel suit in the 14 columns the next card leaves showing.
  const held = (x, y, c, col = ink) => {
    ctx.clearRect(x * S, (y + oy) * S, 22 * S, 26 * S);
    box(x, y, 22, 26, col);
    [...c.r].forEach((ch, j) => sprite(FONT[ch].match(/.../g), x + (c.r.length > 1 ? 1 : 5) + j * 7, y + 2, 2, col));
    sprite(BIG[c.s], x + 3, y + 14, 1, col);
  };
  const rank = r => ({ A: 14, K: 13, Q: 12, J: 11 })[r] || +r;
  const order = h => h.sort((a, b) => SUITS.indexOf(a.s) - SUITS.indexOf(b.s) || a.v - b.v);
  const deal4 = () => { const d = deck().map(c => ({ ...c, v: rank(c.r) })); return [0, 1, 2, 3].map(i => order(d.slice(i * 13, i * 13 + 13))); };
  const suited = (h, s) => h.filter(c => c.s === s);
  // The winning play: the highest trump, else the highest card of the suit led.
  const top = (trick, trump) => trick.reduce((w, x) => ((x.c.s === w.c.s ? x.c.v > w.c.v : x.c.s === trump) ? x : w));
  const handX = n => Math.round((W - (n - 1) * 15 - 22) / 2);

  // The table a trick game plays on. `o` says who you play for, what's legal, how bots play and what a trick and a deal mean.
  const table = (g, o) => {
    // actor: the seat whose person plays this turn (bridge's declarer plays dummy too); sel: each seat's cursor.
    const s = { hands: [[], [], [], []], trick: [], turn: 0, taken: [0, 0, 0, 0], gone: new Set(), sel: [0, 0, 0, 0], wait: false, actor: -1 };
    s.leading = () => s.trick.length % 4 === 0;
    s.follow = h => { if (s.leading()) return h; const f = suited(h, s.trick[0].c.s); return f.length ? f : h; };
    s.legal = p => o.legal(p, s.hands[p]);
    // The hand a seat looks at: the one it's playing from right now, else its own.
    s.view = v => (s.wait && s.actor === v ? s.turn : v);
    s.ok = i => s.wait && s.actor === me && s.legal(s.turn).includes(s.hands[s.turn][i]);
    const step = () => {
      const a = o.who(s.turn);
      if (a >= 0) {
        s.wait = true; s.actor = a; g.busy = false;
        const ok = s.legal(s.turn), h = s.hands[s.turn];
        if (!ok.includes(h[s.sel[a]])) s.sel[a] = h.indexOf(ok[0]);
        g.msg = () => (me === s.actor ? o.prompt() : nm(s.actor) + " TO PLAY");
        return draw();
      }
      s.wait = false; s.actor = -1; g.busy = true; draw();
      later(() => put(s.turn, o.bot(s.turn, s.legal(s.turn))), 450);
    };
    const put = (p, c) => {
      if (s.trick.length === 4) { s.trick = []; g.msg = ""; }
      const h = s.hands[p];
      h.splice(h.indexOf(c), 1);
      s.trick.push({ p, c }); s.gone.add(c.r + c.s);
      if (o.played) o.played(p, c);
      if (s.trick.length < 4) { s.turn = (p + 1) % 4; return step(); }
      const w = top(s.trick, o.trump()).p;
      s.taken[w]++; s.turn = w;
      g.msg = o.took(w);
      // The trick stays on the table until the next card is led.
      if (!h.length) return o.done();
      draw(); later(step, 900);
    };
    s.start = p => { s.trick = []; s.taken = [0, 0, 0, 0]; s.gone = new Set(); s.turn = p; step(); };
    // Someone joined or left: a turn a person was waiting on goes to whoever plays that seat now.
    s.kick = () => { if (s.wait && o.who(s.turn) !== s.actor) step(); };
    s.play = i => {
      if (!s.ok(i)) return false;
      const c = s.hands[s.turn][i];
      s.wait = false; s.actor = -1;
      put(s.turn, c);
      return true;
    };
    // The cursor skips cards you can't play.
    s.move = d => {
      const h = s.hands[s.view(me)], ok = s.wait && s.actor === me ? s.legal(s.turn) : h;
      if (!ok.length) return;
      let i = s.sel[me];
      for (let k = 0; k < h.length; k++) { i = (i + d + h.length) % h.length; if (ok.includes(h[i])) break; }
      s.sel[me] = i;
    };
    s.at = (x, y) => {
      const h = s.hands[s.view(me)], x0 = handX(h.length), i = Math.min(Math.floor((x - x0) / 15), h.length - 1);
      return y >= HY - 5 && y <= HY + 26 && i >= 0 && x - x0 - i * 15 < 22 ? i : -1;
    };
    // lab(p) is a seat's [name, detail]; `hot` the seat to play; `raised` cards lifted out of your hand.
    s.render = (lab, hot, raised, cursor) => {
      const tag = (str, x, y, on, how) => {
        const w = wide(str);
        x = how === "mid" ? Math.round((W - w) / 2) : how === "right" ? x - w : x;
        if (on) { px(x - 1, y - 1, w + 2, 7); text(str, x, y, null); } else text(str, x, y);
      };
      // The seat k places clockwise from yours: 1 on the left, 2 across, 3 on the right.
      const seat = k => (me + k) % 4, v = s.view(me);
      const one = p => lab(p).filter(Boolean).join(" ");
      tag(one(seat(2)), 0, 10, hot === seat(2), "mid");
      const [w1, w2] = lab(seat(1)), [e1, e2] = lab(seat(3));
      tag(w1, 4, 48, hot === seat(1)); if (w2) text(w2, 4, 55, dim);
      tag(e1, 206, 48, hot === seat(3), "right"); if (e2) right(e2, 206, 55, dim);
      tag(one(v), 0, 62, hot === v, "mid");
      s.trick.forEach(({ p, c }) => pip(AT[(p - me + 4) % 4][0], AT[(p - me + 4) % 4][1], c));
      const h = s.hands[v], x0 = handX(h.length), ok = s.wait && s.actor === me ? s.legal(s.turn) : null;
      h.forEach((c, i) => held(x0 + i * 15, HY - (raised && raised.has(c) ? 4 : 0), c, ok && !ok.includes(c) ? dim : ink));
      if (cursor && h[s.sel[me]]) px(x0 + s.sel[me] * 15 + 2, HY + 27, 11, 1);
    };
    return s;
  };

  // A partnership player (spades, bridge): leaves tricks its partner is winning, wins cheaply, leads its sure winners.
  const partnerBot = (t, p, ok, trump, helpPal = true) => {
    const h = t.hands[p], cost = c => c.v + (c.s === trump ? 20 : 0);
    const low = cs => cs.reduce((a, b) => (cost(b) < cost(a) ? b : a)), high = cs => cs.reduce((a, b) => (cost(b) > cost(a) ? b : a));
    // No higher card of its suit is still out.
    const boss = c => { for (let v = c.v + 1; v <= 14; v++) if (!t.gone.has(RANKS[v === 14 ? 0 : v - 1] + c.s) && !h.some(x => x.v === v && x.s === c.s)) return false; return true; };
    if (t.leading()) {
      const sure = ok.filter(c => boss(c) && c.s !== trump);
      if (sure.length) return high(sure);
      const side = ok.filter(c => c.s !== trump);
      if (!side.length) return boss(high(ok)) ? high(ok) : low(ok);
      return SUITS.map(x => suited(side, x)).filter(x => x.length).sort((a, b) => b.length - a.length)[0][0];
    }
    const tr = t.trick, lead = tr[0].c.s, w = top(tr, trump), last = tr.length === 3;
    const wins = ok.filter(c => (c.s === w.c.s ? c.v > w.c.v : c.s === trump));
    if (helpPal && w.p === (p + 2) % 4 && (last || boss(w.c) || (w.c.s === trump && lead !== trump))) return low(ok);
    if (!wins.length) return low(ok);
    if (last || wins[0].s !== lead) return low(wins);
    const sure = wins.filter(boss);
    if (sure.length) return low(sure);
    // Second hand low, third hand high.
    return tr.length === 1 ? low(ok) : high(wins);
  };

  const hearts = (() => {
    const g = { name: "HEARTS", cards: true, room: 4, order: TRICK_ORDER, live: false, busy: false, msg: "AVOID HEARTS AND THE QUEEN OF SPADES. SPACE DEALS" };
    const PASS = [1, 3, 2, 0], WAY = ["LEFT", "RIGHT", "ACROSS", ""];
    // marked: each seat's cards picked to pass (then the ones it got); ready: who has passed.
    let score = [0, 0, 0, 0], got = [0, 0, 0, 0], round = 0, phase = "idle", broken = false, first = true, finished = false;
    let marked = [0, 1, 2, 3].map(() => new Set()), ready = [false, false, false, false];
    const queen = c => c.s === "♠" && c.r === "Q";
    const pts = c => (c.s === "♥" ? 1 : queen(c) ? 13 : 0);
    const bot = (p, ok) => {
      const t0 = t.trick;
      if (t.leading()) {
        const risky = c => c.s === "♠" && !t.gone.has("Q♠") && (c.v >= 12 || t.hands[p].some(queen));
        const pool = ok.filter(c => !risky(c));
        return (pool.length ? pool : ok).reduce((a, b) => (b.v < a.v || (b.v === a.v && a.s === "♥") ? b : a));
      }
      const w = top(t0, null).c, safe = ok.filter(c => !queen(c));
      if (ok[0].s === t0[0].c.s) {
        const under = ok.filter(c => c.v < w.v);
        if (under.length) return under[under.length - 1];
        const pool = safe.length ? safe : ok;
        return t0.length === 3 && !t0.some(x => pts(x.c)) ? pool[pool.length - 1] : pool[0];
      }
      const q = ok.find(queen);
      if (q) return q;
      const big = ok.filter(c => c.s === "♠" && c.v > 12 && !t.gone.has("Q♠")), red = suited(ok, "♥");
      if (big.length) return big[big.length - 1];
      if (red.length) return red[red.length - 1];
      return ok.reduce((a, b) => (b.v > a.v ? b : a));
    };
    const t = table(g, {
      who: p => (human(g, p) ? p : -1),
      trump: () => null,
      legal: (p, h) => {
        if (t.leading()) {
          if (first) return h.filter(c => c.s === "♣" && c.r === "2");
          const ok = broken ? h : h.filter(c => c.s !== "♥");
          return ok.length ? ok : h;
        }
        const f = t.follow(h);
        // Nothing worth points on the first trick, unless that's all you have.
        if (first && f[0].s !== t.trick[0].c.s) { const clean = f.filter(c => !pts(c)); return clean.length ? clean : f; }
        return f;
      },
      played: (p, c) => { if (c.s === "♥") broken = true; marked[p].clear(); },
      took: w => {
        const n = t.trick.reduce((a, x) => a + pts(x.c), 0);
        got[w] += n; first = false;
        return () => takes(w) + (n ? " +" + n : "");
      },
      bot,
      prompt: () => (t.leading() ? (first ? "LEAD THE 2 OF CLUBS" : broken ? "YOUR LEAD" : "YOUR LEAD. HEARTS AREN'T BROKEN YET") : "YOUR TURN"),
      done: () => {
        const moon = got.indexOf(26), took = [...got];
        score = score.map((x, i) => x + (moon < 0 ? got[i] : i === moon ? 0 : 26));
        g.live = g.busy = false; phase = "idle"; round++;
        const low = Math.min(...score), best = [0, 1, 2, 3].filter(i => score[i] === low);
        finished = Math.max(...score) >= 100;
        const end = finished;
        g.msg = () => end
          ? (best.includes(me) ? (best.length > 1 ? "YOU TIE FOR THE WIN" : "YOU WIN THE GAME") : nm(best[0]) + " WINS THE GAME") + " WITH " + low
          : moon === me ? "YOU SHOT THE MOON!" : moon >= 0 ? nm(moon) + " SHOT THE MOON" : took[me] ? "YOU TOOK " + took[me] + " THIS HAND" : "A CLEAN HAND!";
        over();
      },
    });
    const begin = () => { phase = "play"; t.start(t.hands.findIndex(h => h.some(c => c.s === "♣" && c.r === "2"))); };
    const deal = () => {
      if (g.live) return;
      if (finished) { score = [0, 0, 0, 0]; round = 0; finished = false; }
      t.hands = deal4(); t.trick = []; t.sel = [0, 0, 0, 0];
      got = [0, 0, 0, 0]; broken = false; first = true; g.live = true;
      marked = [0, 1, 2, 3].map(() => new Set()); ready = [false, false, false, false];
      const way = WAY[round % 4];
      if (PASS[round % 4]) {
        phase = "pass";
        g.msg = () => {
          const n = marked[me].size;
          return ready[me] ? "WAITING FOR THE OTHERS TO PASS" : n === 3 ? "PRESS SPACE TO PASS THEM " + way : n ? "PICK " + (3 - n) + " MORE TO PASS " + way : "PICK 3 CARDS TO PASS " + way;
        };
      } else { g.msg = "NO PASSING THIS HAND"; begin(); }
    };
    const toggle = i => {
      const c = t.hands[me][i], m = marked[me];
      if (!c || ready[me]) return;
      t.sel[me] = i;
      if (m.has(c)) m.delete(c); else if (m.size < 3) m.add(c);
    };
    // Bots pass what's most likely to cost them: high spades while short in spades, then high hearts, then high cards.
    const danger = (c, h) => (c.s === "♠" && c.v >= 12 ? (suited(h, "♠").length < 5 ? 60 + c.v : c.v - 10) : c.s === "♥" ? 20 + c.v : c.v);
    // The cards change hands once every person has passed; bots pass right away.
    const swap = () => {
      if (phase !== "pass" || [0, 1, 2, 3].some(p => human(g, p) && !ready[p])) return;
      const way = PASS[round % 4];
      const give = t.hands.map((h, p) => (human(g, p) ? [...marked[p]] : [...h].sort((a, b) => danger(b, h) - danger(a, h)).slice(0, 3)));
      t.hands = t.hands.map((h, p) => h.filter(c => !give[p].includes(c)));
      give.forEach((cards, p) => t.hands[(p + way) % 4].push(...cards));
      t.hands.forEach(order);
      marked = [0, 1, 2, 3].map(p => new Set(give[(p - way + 4) % 4]));
      g.msg = "THE RAISED CARDS ARE THE ONES YOU GOT";
      begin();
    };
    const pass = () => { if (marked[me].size !== 3 || ready[me]) return; ready[me] = true; swap(); };
    g.changed = () => { swap(); t.kick(); };
    g.buttons = () => {
      if (phase === "pass") {
        const full = marked[me].size === 3, done = ready[me];
        return [
          { label: marked[me].has(t.hands[me][t.sel[me]]) ? "UNPICK" : "PICK", key: "enter", act: () => toggle(t.sel[me]), on: !done, lit: !full },
          { label: "PASS " + WAY[round % 4], key: "p", act: pass, on: full && !done, lit: full },
        ];
      }
      if (phase === "play") return [{ label: "PLAY", key: "enter", act: () => t.play(t.sel[me]), on: t.ok(t.sel[me]), lit: true }];
      return [{ label: finished || !round ? "NEW GAME" : "DEAL", key: "enter", act: deal, on: true, lit: true }];
    };
    g.key = k => { if (!g.live || !k.startsWith("arrow")) return false; t.move(k === "arrowleft" ? -1 : 1); return true; };
    g.click = (x, y) => {
      const i = t.at(x, y);
      if (i < 0) return false;
      if (phase === "pass") toggle(i); else if (phase === "play" && !t.play(i)) t.sel[me] = i;
      return true;
    };
    g.score = () => (round || g.live ? "HAND " + (round + (g.live ? 1 : 0)) + ". FIRST TO 100 LOSES" : "FIRST TO 100 POINTS LOSES");
    g.render = () => t.render(p => [nm(p) + " " + score[p], g.live || round ? "+" + got[p] : ""], phase === "play" ? t.turn : -1, marked[me], (phase === "pass" && !ready[me]) || t.actor === me);
    return g;
  })();

  const spades = (() => {
    const g = { name: "SPADES", cards: true, room: 4, order: TRICK_ORDER, live: false, busy: false, msg: () => "YOU AND " + nm((me + 2) % 4) + " AGAINST " + nm((me + 1) % 4) + " AND " + nm((me + 3) % 4) + ". TO 500" };
    // bidder: the person bidding now; myBid: what each seat's bid buttons show.
    let score = [0, 0], bags = [0, 0], bids = [null, null, null, null], dealer = 3, phase = "idle", myBid = [3, 3, 3, 3], bidder = -1, broken = false, finished = false, played = 0;
    // About how many tricks a hand takes: aces, guarded kings and queens, long spades, and short suits to trump in.
    const est = h => {
      let n = 0;
      const sp = suited(h, "♠").length;
      for (const su of SUITS) {
        const x = suited(h, su), has = r => x.some(c => c.r === r);
        if (su === "♠") n += has("A") + (has("K") && sp >= 2) + (has("Q") && sp >= 3) * 0.6 + Math.max(0, sp - 4);
        else {
          n += has("A") * (x.length <= 5 ? 1 : 0.5) + (has("K") && x.length >= 2 && x.length <= 4) * 0.8 + (has("Q") && x.length === 3) * 0.3;
          if (sp >= 3) n += x.length === 0 ? 1 : x.length === 1 ? 0.5 : 0;
        }
      }
      return Math.min(13, Math.max(1, Math.round(n)));
    };
    const t = table(g, {
      who: p => (human(g, p) ? p : -1),
      trump: () => "♠",
      legal: (p, h) => {
        if (!t.leading()) return t.follow(h);
        const ok = broken ? h : h.filter(c => c.s !== "♠");
        return ok.length ? ok : h;
      },
      played: (p, c) => { if (c.s === "♠") broken = true; },
      took: w => () => takes(w),
      // A partner who bid nil wants its tricks taken over.
      bot: (p, ok) => partnerBot(t, p, ok, "♠", bids[(p + 2) % 4] !== 0),
      prompt: () => (t.leading() && !broken && t.hands[me].some(c => c.s !== "♠") ? "YOUR LEAD. SPADES AREN'T BROKEN YET" : t.leading() ? "YOUR LEAD" : "YOUR TURN"),
      done: () => {
        const delta = [0, 1].map(k => {
          const nil = [k, k + 2].filter(p => bids[p] === 0), bid = bids[k] + bids[k + 2], took = t.taken[k] + t.taken[k + 2];
          let d = nil.reduce((a, p) => a + (t.taken[p] ? -100 : 100), 0);
          if (!bid) bags[k] += took;
          else if (took >= bid) { d += bid * 10 + took - bid; bags[k] += took - bid; }
          else d -= bid * 10;
          // Ten overtricks (bags) cost 100.
          if (bags[k] >= 10) { bags[k] -= 10; d -= 100; }
          score[k] += d;
          return d;
        });
        g.live = g.busy = false; phase = "idle"; played++;
        const sign = n => (n >= 0 ? "+" + n : String(n)), sc = [...score];
        finished = score.some(x => x >= 500 || x <= -200) && score[0] !== score[1];
        const end = finished;
        // Each side reads its own result: "US" is the team of whoever's looking.
        g.msg = () => {
          const us = me % 2;
          return end ? (sc[us] > sc[1 - us] ? "YOU WIN THE GAME " : "THEY WIN THE GAME ") + Math.max(...sc) + " TO " + Math.min(...sc) : "US " + sign(delta[us]) + "   THEM " + sign(delta[1 - us]);
        };
        over();
      },
    });
    const ask = p => {
      bidder = -1;
      if (bids.every(b => b !== null)) { phase = "play"; return t.start((dealer + 1) % 4); }
      if (human(g, p)) {
        phase = "bid"; bidder = p; g.busy = false;
        g.msg = () => (me === bidder ? "HOW MANY TRICKS WILL YOU TAKE?" : nm(bidder) + " IS BIDDING");
        return draw();
      }
      phase = "wait"; g.busy = true; draw();
      later(() => { bids[p] = est(t.hands[p]); ask((p + 1) % 4); }, 450);
    };
    const deal = () => {
      if (g.live) return;
      if (finished) { score = [0, 0]; bags = [0, 0]; played = 0; finished = false; }
      dealer = (dealer + 1) % 4;
      t.hands = deal4(); t.trick = []; t.sel = [0, 0, 0, 0]; t.taken = [0, 0, 0, 0];
      bids = [null, null, null, null]; broken = false; g.live = true; phase = "wait";
      myBid = t.hands.map(est);
      ask((dealer + 1) % 4);
    };
    const bid = () => { if (phase !== "bid" || bidder !== me) return; bids[me] = myBid[me]; phase = "wait"; ask((me + 1) % 4); };
    const nudge = d => { myBid[me] = Math.min(13, Math.max(0, myBid[me] + d)); };
    // The person bidding left: a bot bids for that seat.
    g.changed = () => {
      if (phase === "bid" && !human(g, bidder)) { bids[bidder] = est(t.hands[bidder]); ask((bidder + 1) % 4); }
      t.kick();
    };
    g.buttons = () => {
      const b = myBid[me];
      if (phase === "bid" && bidder === me) return [
        { label: "-", key: "-", act: () => nudge(-1), on: b > 0 },
        { label: b ? "BID " + b : "BID NIL", key: "enter", act: bid, on: true, lit: true },
        { label: "+", key: "+", act: () => nudge(1), on: b < 13 },
      ];
      if (phase === "bid") return [{ label: "BIDDING", key: "", act: () => {}, on: false }];
      if (phase === "play") return [{ label: "PLAY", key: "enter", act: () => t.play(t.sel[me]), on: t.ok(t.sel[me]), lit: true }];
      if (phase === "wait") return [{ label: "BIDDING", key: "", act: () => {}, on: false }];
      return [{ label: finished || !played ? "NEW GAME" : "DEAL", key: "enter", act: deal, on: true, lit: true }];
    };
    g.key = k => {
      const mine = phase === "bid" && bidder === me;
      if (k === "=" && mine) { nudge(1); return true; }
      if (!g.live || !k.startsWith("arrow")) return false;
      if (mine) nudge(k === "arrowleft" ? -1 : 1); else t.move(k === "arrowleft" ? -1 : 1);
      return true;
    };
    g.click = (x, y) => {
      const i = t.at(x, y);
      if (i < 0 || phase !== "play") return false;
      if (!t.play(i)) t.sel[me] = i;
      return true;
    };
    g.score = () => { const us = me % 2; return "US " + score[us] + " BAGS " + bags[us] + "   THEM " + score[1 - us] + " BAGS " + bags[1 - us]; };
    g.render = () => t.render(p => {
      const b = bids[p] === null ? "" : bids[p] ? "BID " + bids[p] : "NIL";
      return [nm(p), b && (phase === "play" || !g.live) ? b + " TOOK " + t.taken[p] : b];
    }, phase === "play" ? t.turn : phase === "bid" ? bidder : -1, null, t.actor === me);
    return g;
  })();

  // Contract bridge, scored like duplicate with nobody vulnerable and no doubles. When your side wins the auction you play
  // both hands; when theirs does, you defend.
  const bridge = (() => {
    const g = { name: "BRIDGE", cards: true, room: 4, order: TRICK_ORDER, live: false, busy: false, msg: () => "YOU AND " + nm((me + 2) % 4) + " AGAINST " + nm((me + 1) % 4) + " AND " + nm((me + 3) % 4) };
    const STRAIN = ["C", "D", "H", "S", "NT"], TRUMP = ["♣", "♦", "♥", "♠", null];
    const call = b => (b < 0 ? "PASS" : ((b / 5) | 0) + 1 + STRAIN[b % 5]);
    const hcp = h => h.reduce((a, c) => a + Math.max(0, c.v - 10), 0);
    const balanced = h => SUITS.every(su => suited(h, su).length >= 2) && SUITS.filter(su => suited(h, su).length === 2).length <= 1;
    // caller: the person calling now; myBid and hint: each seat's call buttons.
    let score = [0, 0], dealer = 3, calls = [], last = [], high = -1, by = -1, phase = "idle", contract = -1, declarer = -1, dummy = -1, shown = false, played = 0;
    let caller = -1, myBid = [0, 0, 0, 0], hint = [-1, -1, -1, -1];
    // Who plays a seat's cards: dummy's are the declarer's; a person whose partner is a bot plays both of their side's hands
    // when their side declares. -1: a bot.
    const who = p => {
      let c = declarer >= 0 && p === dummy ? declarer : p;
      if (!human(g, c) && declarer >= 0 && c % 2 === declarer % 2 && human(g, (c + 2) % 4)) c = (c + 2) % 4;
      return human(g, c) ? c : -1;
    };
    // A bot bids the cheapest contract it thinks its side can make, from its points and what its partner has bid.
    const botCall = p => {
      const h = t.hands[p], pal = (p + 2) % 4, palCalls = calls.filter(x => x.p === pal), palBids = palCalls.filter(x => x.b >= 0);
      const opened = palBids.length > 0 && calls.find(x => x.b >= 0) === palBids[0];
      const palPts = palBids.length ? (opened ? 13 : 9) : palCalls.length ? 6 : 8, palStrains = new Set(palBids.map(x => x.b % 5));
      const options = [];
      for (let st = 0; st < 5; st++) {
        let tricks;
        if (st < 4) {
          const len = suited(h, TRUMP[st]).length, fit = len + (palStrains.has(st) ? (opened && palBids[0].b % 5 === st && st >= 2 ? 5 : 4) : 3);
          if (fit < 8 && len < 6) continue;
          const short = SUITS.filter(su => su !== TRUMP[st]).reduce((a, su) => a + Math.max(0, 3 - suited(h, su).length), 0);
          tricks = 6 + (hcp(h) + short + palPts + Math.max(0, fit - 8) - 16) * 0.4;
        } else {
          if (!balanced(h) && !palStrains.has(4)) continue;
          tricks = 6 + (hcp(h) + palPts - 17) * 0.4;
        }
        const game = [5, 5, 4, 4, 3][st];
        let level = Math.min(7, Math.floor(tricks) - 6);
        if (level > game && level < 6) level = game;
        let b = high < 0 ? st : (((high / 5) | 0) + (high % 5 < st ? 0 : 1)) * 5 + st;
        if (level < 1 || b > 34 || ((b / 5) | 0) + 1 > level) continue;
        // Partner showed this suit and there's game (or a slam) in it: bid it straight away.
        if (palStrains.has(st) && level >= game) b = Math.max(b, (level - 1) * 5 + st);
        options.push({ b, st, rate: tricks + (st >= 2 ? 0.3 : 0) });
      }
      if (by === pal) {
        const raise = options.find(x => x.st === high % 5);
        if (raise) return raise.b;
        if (high >= 5) return -1;
      }
      return options.length ? options.reduce((a, x) => (x.rate > a.rate ? x : a)).b : -1;
    };
    const say = (p, b) => { calls.push({ p, b }); last[p] = b; if (b >= 0) { high = b; by = p; } };
    const ask = p => {
      caller = -1;
      if (high < 0 && calls.length === 4) {
        g.live = g.busy = false; phase = "idle";
        g.msg = "EVERYONE PASSED. SPACE DEALS AGAIN";
        return over();
      }
      if (high >= 0 && calls.length >= 4 && calls.slice(-3).every(x => x.b < 0)) return begin();
      if (human(g, p)) {
        phase = "call"; g.busy = false; caller = p;
        // Space makes the call a bot would make in your seat; the arrows change it.
        hint[p] = botCall(p);
        myBid[p] = Math.min(34, hint[p] >= 0 ? hint[p] : high + 1);
        g.msg = () => (me === caller ? "YOUR CALL. YOU HAVE " + hcp(t.hands[me]) + " POINTS" : nm(caller) + " TO CALL");
        return draw();
      }
      phase = "auction"; g.busy = true; draw();
      later(() => { say(p, botCall(p)); ask((p + 1) % 4); }, 550);
    };
    const begin = () => {
      const st = high % 5, side = by % 2;
      declarer = calls.find(x => x.b >= 0 && x.b % 5 === st && x.p % 2 === side).p;
      dummy = (declarer + 2) % 4; contract = high; shown = false; phase = "play";
      t.start((declarer + 1) % 4);
    };
    const t = table(g, {
      who,
      trump: () => TRUMP[contract % 5],
      legal: (p, h) => t.follow(h),
      played: () => { shown = true; },
      took: w => () => takes(w),
      bot: (p, ok) => partnerBot(t, p, ok, TRUMP[contract % 5]),
      prompt: () => (t.turn !== me ? "PLAY FROM " + nm(t.turn) : t.leading() && !t.gone.size ? "YOU LEAD FIRST AGAINST " + call(contract) : "YOUR TURN"),
      done: () => {
        const lvl = ((contract / 5) | 0) + 1, st = contract % 5, side = declarer % 2, need = lvl + 6, got = t.taken[side] + t.taken[side + 2];
        let pts, gainer = side;
        const made = got >= need, deal = contract;
        if (made) {
          const per = st < 2 ? 20 : 30, trick = st === 4 ? 40 + 30 * (lvl - 1) : per * lvl;
          pts = trick + (got - need) * (st === 4 ? 30 : per) + (trick >= 100 ? 300 : 50) + (lvl === 6 ? 500 : lvl === 7 ? 1000 : 0);
        } else {
          pts = 50 * (need - got); gainer = 1 - side;
        }
        score[gainer] += pts;
        g.live = g.busy = false; phase = "idle"; played++;
        // Read from the looking seat's side: "YOU MADE" for the declarers, "THEY MADE" for the defenders.
        g.msg = () => {
          const them = side !== me % 2;
          const m = made ? (them ? "THEY MADE " : "YOU MADE ") + call(deal) + (got > need ? " WITH " + (got - need) + " OVER" : "") : (them ? "THEY WENT DOWN " : "DOWN ") + (need - got);
          return m + ". " + (gainer !== me % 2 ? "THEM" : "US") + " +" + pts;
        };
        over();
      },
    });
    const deal = () => {
      if (g.live) return;
      dealer = (dealer + 1) % 4;
      t.hands = deal4(); t.trick = []; t.sel = [0, 0, 0, 0]; t.taken = [0, 0, 0, 0];
      calls = []; last = []; high = by = contract = declarer = dummy = -1; shown = false; g.live = true;
      ask(dealer);
    };
    const bid = b => () => { if (phase !== "call" || caller !== me) return; say(me, b); ask((me + 1) % 4); };
    const nudge = d => { myBid[me] = Math.min(34, Math.max(high + 1, myBid[me] + d)); hint[me] = myBid[me]; };
    // The person calling left: a bot calls for that seat.
    g.changed = () => {
      if (phase === "call" && !human(g, caller)) { const p = caller; say(p, botCall(p)); ask((p + 1) % 4); }
      t.kick();
    };
    g.buttons = () => {
      const b = myBid[me], h = hint[me];
      if (phase === "call" && caller === me) return [
        { label: "PASS", key: "p", act: bid(-1), on: true, lit: h < 0 },
        { label: "-", key: "-", act: () => nudge(-1), on: b > high + 1 },
        { label: "BID " + call(b), key: "enter", act: bid(b), on: high < 34, lit: h >= 0 },
        { label: "+", key: "+", act: () => nudge(1), on: b < 34 },
      ];
      if (phase === "play") return [{ label: "PLAY", key: "enter", act: () => t.play(t.sel[me]), on: t.ok(t.sel[me]), lit: true }];
      if (phase === "auction" || phase === "call") return [{ label: "BIDDING", key: "", act: () => {}, on: false }];
      return [{ label: "DEAL", key: "enter", act: deal, on: true, lit: true }];
    };
    g.key = k => {
      const mine = phase === "call" && caller === me;
      if (k === "=" && mine) { nudge(1); return true; }
      if (!g.live || !k.startsWith("arrow")) return false;
      if (mine) nudge(k === "arrowleft" ? -1 : 1); else t.move(k === "arrowleft" ? -1 : 1);
      return true;
    };
    g.click = (x, y) => {
      const i = t.at(x, y);
      if (i < 0 || phase !== "play") return false;
      if (!t.play(i)) t.sel[me] = i;
      return true;
    };
    g.score = () => {
      const us = me % 2;
      return phase === "play"
        ? call(contract) + " BY " + nm(declarer) + "   TRICKS US " + (t.taken[us] + t.taken[us + 2]) + " THEM " + (t.taken[1 - us] + t.taken[3 - us])
        : "SCORE US " + score[us] + "   THEM " + score[1 - us];
    };
    // A face-up hand you aren't playing from right now (dummy, or yours while you play your partner's), as four lines of ranks.
    const listing = (p, x) => {
      text(p === dummy ? nm(p) + " DUMMY" : nm(p), x, 10, dim);
      ["♠", "♥", "♦", "♣"].forEach((su, j) => {
        const y = 17 + j * 7;
        text("SHDC"[j], x, y, dim);
        text(suited(t.hands[p], su).map(c => c.r).reverse().join("") || "-", x + 6, y);
      });
    };
    g.render = () => {
      const playing = phase === "play" || (!g.live && contract >= 0);
      t.render(p => {
        if (!playing) return [nm(p) + (g.live && p === dealer ? (p === me ? " DEAL" : " DEALS") : ""), last[p] === undefined ? "" : call(last[p])];
        return [nm(p), p === declarer ? "DECLARER" : p === dummy ? "DUMMY" : ""];
      }, phase === "play" ? t.turn : phase === "call" ? caller : -1, null, t.actor === me);
      if (phase !== "play") return;
      // Playing both of your side's hands, you see the other one; otherwise dummy, once the first card is led.
      const v = t.view(me), both = declarer % 2 === me % 2 && who(dummy) === me && who(declarer) === me;
      const other = both ? (v === declarer ? dummy : declarer) : dummy !== me ? dummy : -1;
      if (other >= 0 && other !== v && (other === me || shown)) listing(other, (other - me + 4) % 4 === 3 ? 150 : 2);
    };
    return g;
  })();

  // The rules, shown in place of the table by the ? in the corner (pages.game_help; window.casinoHelp is false when it's off).
  const HELP = {
    BLACKJACK: ["GET CLOSER TO 21 THAN THE DEALER WITHOUT GOING OVER. NUMBER CARDS COUNT AS THEY SHOW, J Q K COUNT 10, AN ACE 1 OR 11.",
      "HIT TAKES A CARD. STAND KEEPS YOUR HAND. DOUBLE DOUBLES YOUR BET FOR ONE LAST CARD. THE DEALER DRAWS TO 17.",
      "EACH HAND BETS 10. A WIN PAYS 1:1, BLACKJACK 3:2, AND A TIE GIVES YOUR BET BACK."],
    POKER: ["TEXAS HOLD 'EM AGAINST 1 TO 5 BOTS. MAKE THE BEST 5-CARD HAND FROM YOUR 2 CARDS AND THE 5 SHARED ONES.",
      "BLINDS ARE 5 AND 10. BETS ARE 10 BEFORE THE TURN CARD AND 20 AFTER IT. FOLD GIVES UP, CALL MATCHES THE BET, RAISE ADDS TO IT.",
      "BEST TO WORST: STRAIGHT FLUSH, FOUR OF A KIND, FULL HOUSE, FLUSH, STRAIGHT, THREE OF A KIND, TWO PAIR, PAIR."],
    BACCARAT: ["BET 20 ON WHICH HAND COMES CLOSER TO 9: PLAYER OR BANKER, OR ON A TIE. THE DEALER DRAWS ALL THE CARDS.",
      "ACES COUNT 1, 10 J Q K COUNT 0, THE REST AS THEY SHOW. ONLY THE LAST DIGIT OF A TOTAL COUNTS, SO 7 + 8 IS 5.",
      "PLAYER PAYS 1:1, BANKER 0.95:1, TIE 8:1."],
    ROULETTE: ["BET 10 ON WHERE THE BALL LANDS, 0 TO 36. RED, BLACK, ODD, EVEN, LOW 1-18 AND HIGH 19-36 PAY 1:1. ZERO PAYS 35:1.",
      "ZERO IS NONE OF THE OTHERS, SO EVERY OTHER BET LOSES ON IT."],
    SLOTS: ["EACH SPIN COSTS 10. MATCH THE 3 REELS TO WIN.",
      "3 SEVENS PAY 1000, 3 BARS 400, 3 BELLS 200, 3 DIAMONDS 120, 3 CHERRIES 50 AND ANY 2 CHERRIES 10."],
    HEARTS: ["EVERY HEART YOU TAKE IS 1 POINT AND THE QUEEN OF SPADES IS 13. LOW SCORE WINS WHEN SOMEONE REACHES 100.",
      "PASS 3 CARDS BEFORE EACH HAND: LEFT, RIGHT, ACROSS, THEN NONE. THE 2 OF CLUBS LEADS. FOLLOW SUIT IF YOU CAN.",
      "NO LEADING HEARTS UNTIL ONE HAS BEEN PLAYED. TAKE ALL 26 POINTS TO SHOOT THE MOON: EVERYONE ELSE GETS 26."],
    SPADES: ["YOU AND NORTH PLAY AGAINST WEST AND EAST. BID HOW MANY OF THE 13 TRICKS YOU'LL TAKE. SPADES ARE TRUMP.",
      "FOLLOW SUIT IF YOU CAN. NO LEADING SPADES UNTIL ONE HAS BEEN PLAYED. THE HIGHEST SPADE, OR ELSE THE HIGHEST CARD OF THE SUIT LED, TAKES THE TRICK.",
      "MAKE YOUR TEAM'S BID FOR 10 A TRICK. EACH EXTRA TRICK IS A BAG: 10 BAGS COST 100. NIL IS +-100. FIRST TO 500 WINS."],
    BRIDGE: ["YOU AND NORTH PLAY AGAINST WEST AND EAST. BID HOW MANY TRICKS OVER 6 YOUR SIDE WILL TAKE, AND WHAT'S TRUMP: C D H S, OR NT FOR NONE.",
      "THE HIGHEST BID PLAYS. THE BIDDER'S PARTNER IS DUMMY: CARDS FACE UP. WIN THE BID AND YOU PLAY BOTH HANDS.",
      "FOLLOW SUIT IF YOU CAN. A J Q K COUNT 4 3 2 1 POINTS. ABOUT 13 OF THEM ARE WORTH AN OPENING BID."],
  };
  const helpOn = window.casinoHelp !== false;
  let help = false;
  // Words wrapped to fit the width, a few pixels between paragraphs.
  const helpLines = () => {
    const out = [];
    (HELP[game.name] || []).forEach((para, i) => {
      let line = "";
      for (const w of para.split(" ")) { if (line && wide(line + " " + w) > W - 8) { out.push([line, i]); line = w; } else line = line ? line + " " + w : w; }
      out.push([line, i]);
    });
    return out;
  };

  const games = [blackjack, poker, baccarat, roulette, slots, hearts, spades, bridge];
  let game = blackjack, tabs = [], buttons = [];
  const choose = name => {
    const g = games.find(g => g.name === String(name || "").toUpperCase());
    if (!g) return;
    game = g; draw();
    if (OWN) { save(); try { history.replaceState(null, "", "#" + g.name.toLowerCase()); } catch {} }
  };

  const colours = night => { ink = night ? "#acacac" : "#535353"; dim = night ? "#5e5e5e" : "#c4c4c4"; };
  // A message can be text, a function of who's looking (`me`), or replaced for one seat by game.notes.
  const msgOf = g => { const m = g.notes && me in g.notes ? g.notes[me] : g.msg; return typeof m === "function" ? m() : m; };
  const layout = list => {
    let x = Math.round((W - list.reduce((t, b) => t + wide(b.label) + 10, -4)) / 2);
    return list.map(b => { const w = wide(b.label) + 6, o = { ...b, x, w }; x += w + 4; return o; });
  };
  // Draws the table as seat `me` sees it, for person `who` (whose chips show at the top). `full`: no seat for them here.
  const paint = (who, helping, note, full) => {
    ctx.clearRect(0, 0, W * S, H * S);
    // Two rows of tabs: the casino, then the card table.
    const tabs = [];
    let x;
    [games.filter(g => !g.cards), games.filter(g => g.cards)].forEach((row, j) => {
      x = Math.round((W - row.reduce((t, g) => t + wide(g.name) + 8, -2)) / 2);
      for (const g of row) {
        const w = wide(g.name) + 6, y = j * 10;
        if (g === game) { px(x, y, w, 9); text(g.name, x + 3, y + 2, null); }
        else text(g.name, x + 3, y + 2, dim);
        tabs.push({ g, x, y, w });
        x += w + 2;
      }
    });
    if (helpOn) { const q = W - 9; if (helping) { px(q, 10, 9, 9); text("?", q + 3, 12, null); } else { box(q, 10, 9, 9, dim); text("?", q + 3, 12, dim); } }
    if (who === host) for (const b of invites()) { box(b.x, 10, b.w, 9, dim); text(b.label, b.x + 2, 12, b.off ? dim : ink); }
    if (full) {
      middle("THIS TABLE IS FULL", 60);
      middle("YOU CAN PLAY WHEN THE HOST PICKS A GAME WITH ROOM", 70, dim);
      return { tabs, buttons: [], msg: "THIS TABLE IS FULL" };
    }
    if (game.score) middle(game.score(), 22);
    else if (who === host) {
      const hi = "HI " + String(Math.round(best)).padStart(5, "0") + "  ", now = String(Math.round(chips)).padStart(5, "0");
      x = Math.round((W - wide(hi + now)) / 2);
      text(hi, x, 22, dim); text(now, x + wide(hi) + 1, 22);
    } else middle("CHIPS " + String(Math.round(who.chips)).padStart(5, "0"), 22);
    if (helping) helpLines().forEach(([l, i], j) => text(l, 4, 31 + j * 7 + i * 3));
    else { oy = OY; game.render(); oy = 0; }
    const msg = note || msgOf(game);
    middle(msg, H - 25);
    const buttons = layout(game.buttons()), y = H - 12;
    for (const b of buttons) {
      const { x, w } = b, c = b.on ? ink : dim, solid = b.lit && b.on;
      if (solid) { px(x, y, w, 11, c); text(b.label, x + 3, y + 2, null); }
      else { box(x, y, w, 11, c); text(b.label, x + 3, y + 2, c); }
      // The key that presses it, underlined (its first letter).
      if (/^[a-z]$/.test(b.key || "") && b.label[0].toLowerCase() === b.key) solid ? ctx.clearRect((x + 3) * S, (y + 8) * S, 3 * S, S) : px(x + 3, y + 8, 3, 1, c);
    }
    return { tabs, buttons, msg };
  };

  const draw = () => {
    const was = me;
    me = 0;
    const r = cv.getBoundingClientRect();
    S = Math.max(1, Math.round((r.width || 420) / W * devicePixelRatio));
    if (cv.width !== W * S) { cv.width = W * S; cv.height = H * S; }
    // Follow the page's own colours (light text = a dark page), not just the system's setting.
    const [r0, g0, b0] = (getComputedStyle(document.body).color.match(/\d+/g) || [0, 0, 0]).map(Number);
    colours(r0 + g0 + b0 > 384);
    const out = paint(host, help, hostNote);
    tabs = out.tabs; buttons = out.buttons;
    const say = help ? (HELP[game.name] || []).join(" ") : out.msg;
    if (say !== said) { said = say; const m = document.getElementById("casino-msg"); if (m) m.textContent = said.toLowerCase(); }
    me = was;
    shareSoon();
  };

  // Friends (pages.game_invites, valerie://page/casino only): INVITE opens a link through Valerie and copies it, LINK copies
  // it again, STOP ends it. Each friend's seat is drawn here and sent as a picture; their clicks and keys come back.
  let invitesOn = false, hosting = "off", hostNote = "", noteTimer = 0, sharing = 0, sent = {};
  const note = t => { hostNote = t; clearTimeout(noteTimer); noteTimer = setTimeout(() => { hostNote = ""; draw(); }, 6000); draw(); };
  const invites = () => {
    if (!invitesOn) return [];
    const friends = people.length - 1;
    const list = hosting === "off" ? [{ label: "INVITE", act: invite }]
      : hosting === "starting" ? [{ label: "OPENING", act: () => {}, off: true }]
      : [{ label: "LINK" + (friends ? " +" + friends : ""), act: invite }, { label: "STOP", act: stopHosting }];
    let x = 1;
    return list.map(b => { const w = wide(b.label) + 4, o = { ...b, x, w }; x += w + 2; return o; });
  };
  const invite = () => {
    valerie.call("host_game").then(r => {
      if (r.error) { hosting = "off"; return note("COULDN'T OPEN A LINK. THE LOG SAYS WHY"); }
      hosting = r.state === "ready" ? "ready" : "starting";
      note(hosting === "ready" ? "LINK COPIED. SEND IT TO FRIENDS" : "OPENING A LINK FOR FRIENDS");
    }).catch(() => note("COULDN'T OPEN A LINK. THE LOG SAYS WHY"));
  };
  const everyoneLeft = () => {
    for (const p of people.splice(1)) games.forEach(g => g.changed && g.changed(p));
    sent = {};
  };
  const stopHosting = () => {
    valerie.call("stop_hosting").catch(() => {});
    hosting = "off"; everyoneLeft();
    note("THE LINK NO LONGER WORKS");
  };
  const off = document.createElement("canvas");
  const frameFor = p => {
    const seat = seatOf(game, p.k), keep = [ctx, S, ink, dim, me];
    off.width = W; off.height = H;
    ctx = off.getContext("2d"); S = 1; colours(p.night);
    me = seat;
    let out;
    try { out = paint(p, p.help, "", seat < 0 && !game.watch); } finally { [ctx, S, ink, dim, me] = keep; }
    return { png: off.toDataURL("image/png"), msg: out.msg };
  };
  const share = () => {
    sharing = 0;
    const frames = {};
    for (const p of people.slice(1)) {
      const f = frameFor(p);
      if (sent[p.id] !== f.png) { sent[p.id] = f.png; frames[p.id] = f; }
    }
    if (Object.keys(frames).length) valerie.call("game_frames", { frames }).catch(() => {});
  };
  function shareSoon() { if (hosting === "ready" && people.length > 1 && !sharing) sharing = setTimeout(share, 100); }
  // A friend's click or key, played as their seat.
  const guestInput = m => {
    const p = people.find(x => x.id === m.input);
    if (!p) return;
    const seat = seatOf(game, p.k), can = seat >= 0;
    me = seat;
    try {
      if (m.type === "click") {
        const x = +m.x, y = +m.y;
        if (helpOn && y >= 9 && y < OY && x >= W - 10) p.help = !p.help;
        else if (p.help && y >= OY && y < H - 13) p.help = false;
        else if (!can || y < OY) return;
        else if (y >= H - 13) { const b = layout(game.buttons()).find(b => b.on && x >= b.x - 1 && x <= b.x + b.w + 1); if (b) { p.help = false; b.act(); } }
        else if (!(game.click && game.click(x, y - OY))) { const lit = game.buttons().find(b => b.on && b.lit); if (!game.live && lit) lit.act(); }
      } else if (m.type === "key") {
        const k = String(m.k || "");
        if (helpOn && (k === "?" || (p.help && k === "escape"))) p.help = k === "?" && !p.help;
        else if (can && !p.help) {
          const all = game.buttons(), b = k === " " ? all.find(b => b.on && b.lit) : all.find(b => b.on && b.key === k);
          if (b) { if (k !== " " || performance.now() - ended > 500) b.act(); }
          else if (game.key) game.key(k);
        }
      }
    } finally { me = 0; }
    draw();
  };
  const freeK = () => { let k = 1; while (people.some(p => p.k === k)) k++; return k; };
  window.casinoHost = m => {
    if (m.ready) { hosting = "ready"; note("LINK COPIED. SEND IT TO FRIENDS"); }
    else if (m.error) { hosting = "off"; note("COULDN'T OPEN A LINK. THE LOG SAYS WHY"); }
    else if (m.stopped) { hosting = "off"; everyoneLeft(); draw(); }
    else if (m.join) {
      const p = { id: String(m.join), k: freeK(), night: !!m.night, help: false, chips: 100 };
      people.push(p);
      games.forEach(g => g.changed && g.changed(null));
      note("P" + (p.k + 1) + " JOINED");
    } else if (m.leave) {
      const i = people.findIndex(p => p.id === m.leave);
      if (i < 1) return;
      const [p] = people.splice(i, 1);
      delete sent[p.id];
      games.forEach(g => g.changed && g.changed(p));
      note("P" + (p.k + 1) + " LEFT");
    } else if (m.input) guestInput(m);
  };
  if (OWN && window.valerie) {
    // A reload ends any hosting the page did before (its friends' seats went with it).
    valerie.call("hosting_status").then(st => {
      invitesOn = !!st.on;
      if (st.state && st.state !== "off") valerie.call("stop_hosting").catch(() => {});
      draw();
    }).catch(() => {});
    addEventListener("pagehide", () => { if (hosting !== "off") valerie.call("stop_hosting").catch(() => {}); });
    // Tells Valerie the tab is still here; hosting ends 90 seconds after it stops hearing that.
    setInterval(() => {
      if (hosting === "off") return;
      valerie.call("game_frames", {}).then(r => { if (r && r.state === "off" && hosting === "ready") { hosting = "off"; everyoneLeft(); draw(); } }).catch(() => {});
    }, 15000);
  }

  const act = b => { help = false; b.act(); draw(); };
  cv.addEventListener("click", e => {
    const r = cv.getBoundingClientRect(), x = (e.clientX - r.left) / r.width * W, y = (e.clientY - r.top) / r.height * H;
    if (helpOn && y >= 9 && y < OY && x >= W - 10) { help = !help; return draw(); }
    if (help && y >= OY && y < H - 13) { help = false; return draw(); }
    if (y >= 9 && y < OY) { const b = invites().find(b => !b.off && x >= b.x && x < b.x + b.w); if (b) return b.act(); }
    if (y < OY) { const t = tabs.find(t => y >= t.y && y < t.y + 10 && x >= t.x && x < t.x + t.w); if (t) choose(t.g.name); return; }
    if (y >= H - 13) { const b = buttons.find(b => b.on && x >= b.x - 1 && x <= b.x + b.w + 1); if (b) act(b); return; }
    if (game.click && game.click(x, y - OY)) return draw();
    const lit = buttons.find(b => b.on && b.lit);
    if (!game.live && lit) act(lit);
  });
  addEventListener("keydown", e => {
    if (e.metaKey || e.ctrlKey || e.altKey || (e.target.closest && e.target.closest("input, textarea, select, [contenteditable]"))) return;
    const k = e.key.toLowerCase();
    if (helpOn && (k === "?" || (help && k === "escape"))) { e.preventDefault(); help = k === "?" && !help; return draw(); }
    if ((k === " " || k === "enter") && e.target.closest && e.target.closest("button, a, summary")) return;
    if (k === "arrowleft" || k === "arrowright") {
      e.preventDefault();
      // In the middle of a card game the arrows pick a card (or a bid); otherwise they switch games.
      if (!help && game.key && game.key(k)) return draw();
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
  // The offline page opens on the game in pages.offline_game_pick (window.casinoStart), or a random one.
  let start = OWN ? location.hash.slice(1) : window.casinoStart || "random";
  try { if (!start && OWN) start = JSON.parse(localStorage.getItem("casino") || "{}").game; } catch {}
  if (start === "random") start = games[Math.floor(Math.random() * games.length)].name;
  game = games.find(g => g.name === String(start || "").toUpperCase()) || blackjack;
  draw();
})();
