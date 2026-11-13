// The Zecky lock and its medallion, drawn the same way as Lock it Mac
// (PadlockView.swift): a gold padlock with the engraved Z and a name plate,
// guilloché line work behind it, and a thin ring that fills towards the next
// milestone. Only drawing; nothing here knows about the wallet.

"use strict";

const Lockart = (() => {
  const SVG = "http://www.w3.org/2000/svg";
  let uid = 0;

  // ---- The padlock, in a 300 × 340 space ----

  function padlock(host, { size = 212, plate = null } = {}) {
    const id = `pl${uid++}`;
    host.innerHTML = `
<svg class="padlock" viewBox="0 0 300 340" width="${size}" height="${(size * 340) / 300}" aria-hidden="true">
  <defs>
    <linearGradient id="${id}-shackle" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0" stop-color="rgb(138,110,59)"/>
      <stop offset="0.33" stop-color="rgb(247,230,184)"/>
      <stop offset="0.6" stop-color="rgb(201,158,79)"/>
      <stop offset="1" stop-color="rgb(112,84,33)"/>
    </linearGradient>
    <linearGradient id="${id}-body" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="rgb(245,222,163)"/>
      <stop offset="0.42" stop-color="#D4A657"/>
      <stop offset="1" stop-color="rgb(128,92,36)"/>
    </linearGradient>
    <linearGradient id="${id}-shine" x1="0" y1="0" x2="1" y2="0.25">
      <stop offset="0" stop-color="#fff" stop-opacity="0"/>
      <stop offset="0.28" stop-color="#fff" stop-opacity="0.32"/>
      <stop offset="0.46" stop-color="#fff" stop-opacity="0"/>
    </linearGradient>
    <filter id="${id}-blur" x="-20%" y="-200%" width="140%" height="500%"><feGaussianBlur stdDeviation="8"/></filter>
  </defs>
  <ellipse cx="150" cy="332" rx="112" ry="10" fill="#000" opacity="0.6" filter="url(#${id}-blur)"/>
  <g class="shackle">
    <path d="M84 160 V96 A66 66 0 0 1 216 96 V160" fill="none" stroke="url(#${id}-shackle)" stroke-width="26"/>
  </g>
  <g class="lock-body">
    <rect x="36" y="148" width="228" height="176" rx="34" fill="url(#${id}-body)"/>
    <rect x="36" y="148" width="228" height="176" rx="34" fill="url(#${id}-shine)"/>
    <rect x="41" y="153" width="218" height="166" rx="30" fill="none" stroke="rgb(255,242,209)" stroke-opacity="0.5" stroke-width="2"/>
    <rect x="36.75" y="148.75" width="226.5" height="174.5" rx="33.5" fill="none" stroke="rgb(89,66,23)" stroke-opacity="0.65" stroke-width="1.5"/>
    <g class="z" transform="translate(0 ${plate ? 0 : 20})">
      <text x="150" y="${plate ? 252 : 252}" text-anchor="middle" class="z-light">Z</text>
      <text x="150" y="${plate ? 249 : 249}" text-anchor="middle" class="z-ink">Z</text>
    </g>
    ${plate ? `
    <g class="plate">
      <rect x="92" y="274" width="116" height="30" rx="6" fill="#14161C" stroke="rgb(242,214,150)" stroke-opacity="0.7" stroke-width="1.2"/>
      <text x="152.5" y="294" text-anchor="middle" class="plate-text">${plate}</text>
    </g>` : ""}
  </g>
</svg>`;
    return host.querySelector("svg");
  }

  function setPlate(host, text) {
    const label = host.querySelector(".plate-text");
    if (label) label.textContent = text;
  }

  function setOpen(host, open) {
    host.classList.toggle("is-open", open);
  }

  // ---- The medallion: guilloché rosettes, densest near the lock ----

  function drawMedallion(canvas, gold = [212, 166, 87]) {
    const ctx = canvas.getContext("2d");
    const size = canvas.width;
    const c = size / 2;
    const maxR = size / 2;
    ctx.clearRect(0, 0, size, size);
    const bands = [
      [0.40, 0.030, 36, 6], [0.52, 0.035, 42, 6], [0.66, 0.030, 48, 5],
      [0.80, 0.025, 54, 4], [0.93, 0.020, 60, 3],
    ];
    ctx.lineWidth = 0.7;
    bands.forEach(([r, a, n, copies], bi) => {
      const baseR = r * maxR;
      const amp = a * maxR;
      const fade = 1 - bi / bands.length;
      ctx.strokeStyle = `rgba(${gold.join(",")},${0.05 + 0.1 * fade})`;
      for (let copy = 0; copy < copies; copy += 1) {
        const phase = (copy / copies) * ((2 * Math.PI) / n);
        ctx.beginPath();
        const steps = 1440;
        for (let i = 0; i <= steps; i += 1) {
          const t = (i / steps) * 2 * Math.PI;
          const rr = baseR + amp * Math.sin(n * (t + phase)) + amp * 0.35 * Math.sin(3 * t);
          const x = c + rr * Math.cos(t);
          const y = c + rr * Math.sin(t);
          if (i === 0) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        }
        ctx.closePath();
        ctx.stroke();
      }
    });
    [[0.985, 0.1], [0.995, 0.05]].forEach(([f, o]) => {
      ctx.strokeStyle = `rgba(${gold.join(",")},${o})`;
      ctx.beginPath();
      ctx.arc(c, c, f * maxR, 0, 2 * Math.PI);
      ctx.stroke();
    });
  }

  // ---- Engraved rings: one fine line per big milestone, like tree rings ----

  const ENGRAVED = [
    [0.535, 2.0, 48], [0.565, 2.4, 56], [0.505, 1.6, 40], [0.595, 2.0, 64], [0.475, 1.4, 36], [0.625, 1.8, 72],
  ];

  function drawEngraved(canvas, count, newestBright) {
    const ctx = canvas.getContext("2d");
    const size = canvas.width;
    const c = size / 2;
    ctx.clearRect(0, 0, size, size);
    ctx.lineWidth = 0.6;
    ENGRAVED.slice(0, count).forEach(([r, amp, waves], k) => {
      const opacity = k === count - 1 && newestBright ? 0.6 : 0.26;
      ctx.strokeStyle = `rgba(212,166,87,${opacity})`;
      for (let copy = 0; copy < 3; copy += 1) {
        const phase = (copy / 3) * ((2 * Math.PI) / waves);
        ctx.beginPath();
        for (let i = 0; i <= 480; i += 1) {
          const t = (i / 480) * 2 * Math.PI;
          const rr = (r * size) / 2 + amp * Math.sin(waves * (t + phase));
          const x = c + rr * Math.cos(t);
          const y = c + rr * Math.sin(t);
          if (i === 0) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        }
        ctx.closePath();
        ctx.stroke();
      }
    });
  }

  // ---- The milestone ring: fills towards the next milestone ----

  function milestone(svg, progress, glow) {
    const d = 320;
    const r = d / 2;
    const c = 170;
    const p = Math.max(0.004, Math.min(1, progress));
    let ticks = "";
    for (let i = 0; i < 60; i += 1) {
      const long = i % 5 === 0;
      const angle = (i * 6 - 90) * (Math.PI / 180);
      const r1 = r - 10 - (long ? 4 : 2);
      const r2 = r - 10 + (long ? 4 : 2);
      ticks += `<line x1="${c + r1 * Math.cos(angle)}" y1="${c + r1 * Math.sin(angle)}" x2="${c + r2 * Math.cos(angle)}" y2="${c + r2 * Math.sin(angle)}" stroke="var(--gold)" stroke-opacity="${long ? 0.35 : 0.14}" stroke-width="1"/>`;
    }
    const tip = (p * 360 - 90) * (Math.PI / 180);
    svg.innerHTML = `
  <circle cx="${c}" cy="${c}" r="${r}" fill="none" stroke="var(--gold)" stroke-opacity="0.14" stroke-width="2"/>
  ${glow ? `<circle cx="${c}" cy="${c}" r="${r}" fill="none" stroke="var(--gold)" stroke-opacity="0.14" stroke-width="16" stroke-linecap="round" pathLength="100" stroke-dasharray="${p * 100} 100" transform="rotate(-90 ${c} ${c})" class="ms-glow"/>` : ""}
  <circle class="ms-arc" cx="${c}" cy="${c}" r="${r}" fill="none" stroke="var(--gold)" stroke-width="2.5" stroke-linecap="round" pathLength="100" stroke-dasharray="${p * 100} 100" transform="rotate(-90 ${c} ${c})"/>
  <circle class="ms-tip" cx="${c + r * Math.cos(tip)}" cy="${c + r * Math.sin(tip)}" r="4" fill="var(--gold)"/>
  ${ticks}`;
  }

  return { padlock, setPlate, setOpen, drawMedallion, drawEngraved, milestone };
})();

// ---- Holding: how long the Zcash has been in the lock (as Lock it Mac) ----

const Holding = (() => {
  const YEAR = 365;

  function milestoneList(upTo) {
    const list = [7];
    for (let m = 1; m <= 11; m += 1) list.push(30 * m);
    let y = 1;
    while (list[list.length - 1] <= upTo) {
      list.push(YEAR * y);
      y += 1;
    }
    return list;
  }

  function reachedToday(days) {
    return days > 0 && milestoneList(days).includes(days);
  }

  function progress(days) {
    const held = Math.max(0, days);
    if (reachedToday(held)) return 1;
    const list = milestoneList(held);
    const next = list.find((d) => d > held) ?? list[list.length - 1];
    const prev = [0, ...list].filter((d) => d < next).pop() ?? 0;
    return (held - prev) / (next - prev);
  }

  const engraves = (d) => d === 7 || (d >= YEAR && d % YEAR === 0);
  const engraved = (days) => milestoneList(Math.max(0, days)).filter((d) => d <= days && engraves(d)).length;
  const engravedToday = (days) => reachedToday(days) && engraves(days);

  // Stretches of holding, from the wallet history: a stretch starts with the
  // first confirmed payment in after an empty lock; only an empty lock ends it.
  function walk(entries) {
    const list = [];
    let balance = 0;
    let open = false;
    const sorted = [...entries].sort((a, b) => a.height - b.height || a.time - b.time);
    for (const e of sorted) {
      const paymentIn = e.change > 0;
      if (!open && balance <= 0 && paymentIn) {
        list.push({ start: e.time, end: null });
        open = true;
      }
      balance += e.change;
      if (balance <= 0) {
        balance = 0;
        if (open) list[list.length - 1].end = e.time;
        open = false;
      }
    }
    return { stretches: list, balance };
  }

  function since(entries) {
    const { stretches } = walk(entries);
    const last = stretches[stretches.length - 1];
    return last && last.end === null ? last.start : null;
  }

  function historyComplete(entries, pendingIn, balance) {
    return walk(entries).balance + Math.max(0, pendingIn) >= balance;
  }

  // Calendar days, so a payment from yesterday evening counts as 1 day.
  function days(sinceSeconds, now = new Date()) {
    const a = new Date(sinceSeconds * 1000);
    const startA = new Date(a.getFullYear(), a.getMonth(), a.getDate());
    const startB = new Date(now.getFullYear(), now.getMonth(), now.getDate());
    return Math.max(0, Math.round((startB - startA) / 86400000));
  }

  function heldFor(d) {
    const unit = (n, one) => `${n} ${one}${n === 1 ? "" : "s"}`;
    if (d < 1) return "since today";
    if (d < 7) return "for " + unit(d, "day");
    if (d < 30) return "for " + unit(Math.floor(d / 7), "week");
    if (d < 365) return "for " + unit(Math.min(11, Math.floor(d / 30)), "month");
    return "for " + unit(Math.floor(d / 365), "year");
  }

  return { progress, reachedToday, engraved, engravedToday, since, historyComplete, days, heldFor };
})();
