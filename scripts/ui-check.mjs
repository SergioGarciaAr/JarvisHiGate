#!/usr/bin/env node
// Real-browser layout and scroll checks for the Jarvis UI.
//
// Drives the installed Google Chrome in headless mode over the DevTools
// protocol (no extra dependencies) against the Vite dev server serving
// dev/ui-harness.html, which mounts the real React app on an IPC mock.
// Input is sent as real wheel/keyboard events, not by setting scrollTop.
//
// Note: Tauri on macOS renders with WebKit (WKWebView), not Chromium. This
// verifies layout and scroll logic; a final manual check in the app is still needed.
//
//   npm run dev &               # Vite on http://localhost:1420
//   node scripts/ui-check.mjs [--only=baseline] [--shots=target/ui-check]

import { spawn } from "node:child_process";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const args = Object.fromEntries(process.argv.slice(2).map((a) => a.replace(/^--/, "").split("=")));
const BASE = args.base ?? "http://localhost:1420/dev/ui-harness.html";
const SHOTS = args.shots ?? "target/ui-check";
const PORT = 9333;
const CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const PROFILE = "target/chrome-profile";

mkdirSync(SHOTS, { recursive: true });
rmSync(PROFILE, { recursive: true, force: true });
const chrome = spawn(CHROME, [
  "--headless=new",
  `--remote-debugging-port=${PORT}`,
  `--user-data-dir=${PROFILE}`,
  "--no-first-run",
  "--no-default-browser-check",
  "--disable-extensions",
  "--hide-scrollbars=false",
  "about:blank",
], { stdio: "ignore" });

async function waitForChrome() {
  for (let i = 0; i < 50; i++) {
    try {
      return await (await fetch(`http://127.0.0.1:${PORT}/json/version`)).json();
    } catch {
      await sleep(100);
    }
  }
  throw new Error("Chrome no arrancó");
}

class Cdp {
  constructor(ws) {
    this.ws = ws;
    this.id = 0;
    this.pending = new Map();
    ws.onmessage = (m) => {
      const msg = JSON.parse(m.data);
      if (msg.id && this.pending.has(msg.id)) {
        const { resolve, reject } = this.pending.get(msg.id);
        this.pending.delete(msg.id);
        msg.error ? reject(new Error(msg.error.message)) : resolve(msg.result);
      }
    };
  }
  send(method, params = {}) {
    const id = ++this.id;
    this.ws.send(JSON.stringify({ id, method, params }));
    return new Promise((resolve, reject) => this.pending.set(id, { resolve, reject }));
  }
}

async function openPage() {
  await waitForChrome();
  const target = await (await fetch(`http://127.0.0.1:${PORT}/json/new?about:blank`, { method: "PUT" })).json();
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((r, j) => ((ws.onopen = r), (ws.onerror = j)));
  const cdp = new Cdp(ws);
  await cdp.send("Page.enable");
  await cdp.send("Runtime.enable");
  return cdp;
}

const results = [];
function check(name, ok, detail = "") {
  results.push({ name, ok: !!ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? `  — ${detail}` : ""}`);
}

let cdp;
async function evaluate(expr) {
  const r = await cdp.send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
  return r.result.value;
}
async function waitFor(expr, timeout = 8000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    if (await evaluate(expr)) return true;
    await sleep(50);
  }
  return false;
}
async function viewport(width, height) {
  await cdp.send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile: false });
  await sleep(150);
}
async function goto(query = "") {
  await cdp.send("Page.navigate", { url: BASE + query });
  await waitFor("!!document.querySelector('#composer-input')", 15000);
  await sleep(200);
}
async function shot(name) {
  const { data } = await cdp.send("Page.captureScreenshot", { format: "png" });
  writeFileSync(`${SHOTS}/${name}.png`, Buffer.from(data, "base64"));
}
async function key(keyName, code = keyName, vk = 0, text) {
  await cdp.send("Input.dispatchKeyEvent", { type: "keyDown", key: keyName, code, windowsVirtualKeyCode: vk, text });
  await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", key: keyName, code, windowsVirtualKeyCode: vk });
}
async function send(text) {
  await evaluate("document.querySelector('#composer-input').focus()");
  await cdp.send("Input.insertText", { text });
  await key("Enter", "Enter", 13, "\r");
}
async function idle(timeout = 20000) {
  return waitFor("!document.querySelector('.msg--streaming') && !document.querySelector('.run--running') && !document.querySelector('.msg + .typing')", timeout);
}
async function setMock(obj) {
  await evaluate(`Object.assign(window.__mock, ${JSON.stringify(obj)})`);
}
async function wheel(deltaY, steps = 1) {
  const r = await evaluate("(() => { const b = document.querySelector('.chatlog').getBoundingClientRect(); return { x: b.x + b.width / 2, y: b.y + b.height / 2 }; })()");
  for (let i = 0; i < steps; i++) {
    await cdp.send("Input.dispatchMouseEvent", { type: "mouseWheel", x: r.x, y: r.y, deltaX: 0, deltaY });
    await sleep(40);
  }
  await sleep(200);
}
async function m() {
  return evaluate(`(() => {
    const log = document.querySelector('.chatlog');
    const comp = document.querySelector('.composer').getBoundingClientRect();
    const top = document.querySelector('.hud__top, .quick__head').getBoundingClientRect();
    const msgs = log.querySelectorAll('.msg, .run, .callout, .help');
    const last = msgs[msgs.length - 1]?.getBoundingClientRect();
    const lr = log.getBoundingClientRect();
    const jump = document.querySelector('.jump-latest');
    return {
      scrollTop: Math.round(log.scrollTop), scrollHeight: log.scrollHeight, clientHeight: log.clientHeight,
      distance: Math.round(log.scrollHeight - log.scrollTop - log.clientHeight),
      docScrollable: document.scrollingElement.scrollHeight > innerHeight + 1,
      composerTop: Math.round(comp.top), composerBottom: Math.round(comp.bottom), viewportH: innerHeight,
      headerTop: Math.round(top.top), headerBottom: Math.round(top.bottom),
      lastBottom: last ? Math.round(last.bottom) : null, logTop: Math.round(lr.top), logBottom: Math.round(lr.bottom),
      jumpVisible: !!jump && getComputedStyle(jump).display !== 'none' && jump.getBoundingClientRect().height > 0,
    };
  })()`);
}

const LONG = "Esta es una respuesta larga de prueba para comprobar el desplazamiento. ".repeat(12);

async function baseline() {
  await viewport(1180, 760);
  await goto();
  await setMock({ nextReply: LONG, intervalMs: 1, chunkSize: 200 });
  for (let i = 0; i < 12; i++) {
    await send(`mensaje ${i + 1}`);
    await idle();
  }
  const s = await m();
  await shot("baseline-long");
  console.log("baseline metrics", JSON.stringify(s));
  check("conversación larga desborda dentro de .chatlog", s.scrollHeight > s.clientHeight + 10, `scrollHeight=${s.scrollHeight} clientHeight=${s.clientHeight}`);
  check("el compositor está dentro de la ventana", s.composerBottom <= s.viewportH, `composerBottom=${s.composerBottom} viewport=${s.viewportH}`);
  check("se ve el último mensaje", s.lastBottom !== null && s.lastBottom <= s.logBottom + 2 && s.lastBottom > s.logTop, `lastBottom=${s.lastBottom} logBottom=${s.logBottom}`);
}

async function mainSuite() {
  await viewport(1180, 760);
  await goto();

  // Short conversation
  await setMock({ nextReply: "Hola, ¿en qué te ayudo?", intervalMs: 5, chunkSize: 6 });
  await send("hola");
  await idle();
  let s = await m();
  await shot("01-short");
  check("corta: sin desbordamiento ni salto", s.scrollHeight <= s.clientHeight + 1 && !s.jumpVisible, JSON.stringify(s));
  check("corta: compositor y cabecera visibles", s.composerBottom <= s.viewportH && s.headerTop >= 0);
  check("el documento no se desplaza (solo la conversación)", !s.docScrollable);

  // Long conversation
  await setMock({ nextReply: LONG, intervalMs: 1, chunkSize: 200 });
  for (let i = 0; i < 12; i++) {
    await send(`mensaje largo ${i + 1}`);
    await idle();
  }
  s = await m();
  await shot("02-long-bottom");
  check("larga: desborda dentro de .chatlog", s.scrollHeight > s.clientHeight * 2, `scrollHeight=${s.scrollHeight} clientHeight=${s.clientHeight}`);
  check("larga: sigue al final tras enviar", s.distance <= 4, `distance=${s.distance}`);
  check("larga: el último mensaje queda por encima del compositor", s.lastBottom <= s.composerTop, `lastBottom=${s.lastBottom} composerTop=${s.composerTop}`);
  check("larga: cabecera fija visible", s.headerTop === 0 && s.headerBottom > 0, `headerTop=${s.headerTop}`);
  check("larga: compositor anclado abajo", s.composerBottom === s.viewportH, `composerBottom=${s.composerBottom} viewport=${s.viewportH}`);

  // Wheel up reaches the beginning, wheel down reaches the end
  await wheel(-600, 40);
  s = await m();
  check("rueda hacia arriba llega al principio", s.scrollTop === 0, `scrollTop=${s.scrollTop}`);
  await shot("03-wheel-top");
  await wheel(600, 40);
  s = await m();
  check("rueda hacia abajo llega al último mensaje", s.distance <= 4, `distance=${s.distance}`);

  // Streaming while the user reads older messages: position must not move
  await setMock({ nextReply: LONG + LONG, intervalMs: 30, chunkSize: 20 });
  await send("respuesta en streaming mientras leo");
  await sleep(300);
  await wheel(-500, 3);
  const reading = await m();
  await sleep(1200);
  s = await m();
  await shot("04-reading-while-streaming");
  check("leyendo arriba: el streaming no mueve la vista", Math.abs(s.scrollTop - reading.scrollTop) <= 2, `antes=${reading.scrollTop} después=${s.scrollTop}`);
  check("leyendo arriba: aparece el indicador de mensajes nuevos", s.jumpVisible);

  // Jump to latest, then follow the stream
  await evaluate("document.querySelector('.jump-latest').click()");
  await sleep(600);
  s = await m();
  check("el indicador lleva al último mensaje", s.distance <= 4 && !s.jumpVisible, `distance=${s.distance}`);
  const samples = [];
  for (let i = 0; i < 5; i++) {
    await sleep(150);
    samples.push((await m()).distance);
  }
  check("cerca del final: el streaming se sigue automáticamente", samples.every((d) => d <= 4), `distancias=${samples.join(",")}`);
  await idle();

  // Cancel with Esc during streaming, scroll still works afterwards
  await setMock({ nextReply: LONG + LONG, intervalMs: 40, chunkSize: 10 });
  await send("cancela esto");
  await sleep(300);
  await evaluate("document.querySelector('#composer-input').focus()");
  await key("Escape", "Escape", 27);
  const cancelled = await waitFor("!!document.querySelector('.msg--cancelled')", 3000);
  check("Esc cancela el streaming", cancelled);
  s = await m();
  check("tras cancelar sigue al final", s.distance <= 4, `distance=${s.distance}`);
  await wheel(-400, 2);
  check("tras cancelar la rueda sigue funcionando", (await m()).scrollTop < s.scrollTop);
  await wheel(600, 20);

  // Keyboard scrolling on the focused log
  await evaluate("document.querySelector('.chatlog').focus()");
  const beforeKey = (await m()).scrollTop;
  await key("PageUp", "PageUp", 33);
  await sleep(400);
  check("PageUp en la conversación enfocada desplaza hacia arriba", (await m()).scrollTop < beforeKey);
  await key("End", "End", 35);
  await sleep(400);
  check("Fin vuelve al final", (await m()).distance <= 4);

  // Diagnostic report: grows when details open; whole report readable
  await send("jarvis diagnostico --full");
  await waitFor("!!document.querySelector('.report')", 5000);
  await idle();
  await sleep(300);
  s = await m();
  check("informe: visible y seguido al final", s.distance <= 4, `distance=${s.distance}`);
  await evaluate("document.querySelectorAll('.run:last-of-type details').forEach((d) => (d.open = true))");
  await sleep(400);
  s = await m();
  await shot("05-report-details");
  check("informe: al abrir detalles se mantiene al final", s.distance <= 4, `distance=${s.distance}`);
  const reportTop = await evaluate("(() => { const log = document.querySelector('.chatlog'); const r = document.querySelector('.run:last-of-type'); return r.offsetTop - log.offsetTop; })()");
  await wheel(-300, 60);
  await wheel(300, 80);
  s = await m();
  check("informe: se puede recorrer entero con la rueda", s.distance <= 4 && reportTop > 0, `distance=${s.distance}`);
  check("sin scroll anidado en el informe", await evaluate("[...document.querySelectorAll('.report *')].every((e) => !['auto','scroll'].includes(getComputedStyle(e).overflowY) || e.scrollHeight <= e.clientHeight)"));

  // Resize while at bottom, and while reading
  await viewport(900, 560);
  s = await m();
  await shot("06-resized-small");
  check("al reducir la ventana sigue al final", s.distance <= 4, `distance=${s.distance}`);
  check("al reducir la ventana el compositor sigue visible", s.composerBottom === s.viewportH, `composerBottom=${s.composerBottom} viewport=${s.viewportH}`);
  await wheel(-400, 3);
  const readingSmall = await m();
  await viewport(1180, 760);
  s = await m();
  check("al ampliar mientras lee no salta al final", s.distance > 4 && Math.abs(s.scrollTop - readingSmall.scrollTop) < 400, `scrollTop ${readingSmall.scrollTop}→${s.scrollTop}, distance=${s.distance}`);
  await wheel(600, 40);
}

async function quickSuite() {
  await viewport(680, 460);
  await goto("?window=quick");
  let s = await m();
  check("panel rápido: compositor y cabecera visibles al abrir", s.composerBottom <= s.viewportH && s.headerTop >= 0);
  await setMock({ nextReply: LONG, intervalMs: 1, chunkSize: 200 });
  for (let i = 0; i < 6; i++) {
    await send(`rápido ${i + 1}`);
    await idle();
  }
  s = await m();
  await shot("07-quick-long");
  check("panel rápido: desborda dentro de .chatlog", s.scrollHeight > s.clientHeight, `scrollHeight=${s.scrollHeight} clientHeight=${s.clientHeight}`);
  check("panel rápido: sigue al final", s.distance <= 4, `distance=${s.distance}`);
  check("panel rápido: compositor anclado", s.composerBottom <= s.viewportH && s.lastBottom <= s.composerTop, JSON.stringify(s));
  await wheel(-400, 20);
  check("panel rápido: la rueda llega al principio", (await m()).scrollTop === 0);
  // Panel hidden and shown again (the window is hidden, not destroyed)
  await evaluate("document.dispatchEvent(new Event('visibilitychange'))");
  await wheel(400, 30);
  check("panel rápido: la rueda vuelve al final", (await m()).distance <= 4);
}

async function voiceSuite() {
  await viewport(1180, 760);
  await goto();
  await waitFor("!!document.querySelector('.voice')", 3000);
  const mic = await evaluate("(() => { const b = document.querySelector('.voice__mic'); return b ? { disabled: b.disabled, title: b.title } : null; })()");
  check("voz: el micrófono aparece deshabilitado con motivo", mic && mic.disabled && mic.title.length > 10, JSON.stringify(mic));
  const chip = await evaluate("document.querySelector('.voice__state').textContent");
  check("voz: estado inicial inactivo", /inactivo/i.test(chip), chip);
  await evaluate("document.querySelector('.voice__autospeak input').click()");
  await setMock({ nextReply: "Respuesta para leer en voz alta.", intervalMs: 5, chunkSize: 8, speakMs: 1500 });
  await send("léelo");
  const speaking = await waitFor("/hablando/i.test(document.querySelector('.voice__state').textContent)", 4000);
  check("voz: al terminar la respuesta pasa a «hablando»", speaking);
  check("voz: el núcleo y la actividad principal muestran que Jarvis habla", /Hablando/.test(await evaluate("document.querySelector('.activity').textContent")));
  await shot("08-voice-speaking");
  await evaluate("document.querySelector('#composer-input').focus()");
  await key("Escape", "Escape", 27);
  const stopped = await waitFor("/cancelad/i.test(document.querySelector('.voice__state').textContent)", 2000);
  check("voz: Esc interrumpe la lectura", stopped);
  const calls = await evaluate("window.__mock.calls.filter((c) => c.cmd.startsWith('voice_')).map((c) => c.cmd)");
  check("voz: se llamó a voice_speak y voice_stop", calls.includes("voice_speak") && calls.includes("voice_stop"), calls.join(","));
  const s = await m();
  check("voz: el control no reduce el área de conversación", s.composerBottom === s.viewportH && s.headerBottom < 200, `headerBottom=${s.headerBottom}`);
}

try {
  cdp = await openPage();
  const only = args.only;
  if (!only || only === "baseline") await baseline();
  if (!only || only === "main") await mainSuite();
  if (!only || only === "quick") await quickSuite();
  if (!only || only === "voice") await voiceSuite();
} catch (e) {
  check("ejecución del script", false, e.stack);
} finally {
  chrome.kill();
  const failed = results.filter((r) => !r.ok).length;
  console.log(`\n${results.length - failed}/${results.length} comprobaciones superadas · capturas en ${SHOTS}/`);
  process.exitCode = failed ? 1 : 0;
}
