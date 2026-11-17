// Minimal CDP client for the DVD Forge webview.
//   node cdp.mjs eval "<js>"            -> prints result
//   node cdp.mjs shot <out.png> [js]    -> optional js, waits, screenshot
import { writeFileSync } from "node:fs";

const [cmd, arg1, arg2] = process.argv.slice(2);
const list = await (await fetch("http://127.0.0.1:9333/json")).json();
const page = list.find((entry) => entry.type === "page");
const ws = new WebSocket(page.webSocketDebuggerUrl);
let id = 0;
const pending = new Map();
ws.onmessage = (event) => {
  const msg = JSON.parse(event.data);
  if (msg.id && pending.has(msg.id)) {
    pending.get(msg.id)(msg);
    pending.delete(msg.id);
  }
};
await new Promise((resolve) => (ws.onopen = resolve));
const send = (method, params = {}) =>
  new Promise((resolve) => {
    const n = ++id;
    pending.set(n, resolve);
    ws.send(JSON.stringify({ id: n, method, params }));
  });
const evaluate = async (expression) => {
  const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (r.result?.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails).slice(0, 500));
  return r.result?.result?.value;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

if (cmd === "eval") {
  console.log(JSON.stringify(await evaluate(arg1), null, 1));
} else if (cmd === "shot") {
  await send("Emulation.setDeviceMetricsOverride", { width: 1600, height: 900, deviceScaleFactor: 1, mobile: false });
  await sleep(800);
  if (arg2) console.log(JSON.stringify(await evaluate(arg2)));
  await sleep(1500);
  const r = await send("Page.captureScreenshot", { format: "png", captureBeyondViewport: false });
  writeFileSync(arg1, Buffer.from(r.result.data, "base64"));
  console.log("saved", arg1);
}
ws.close();
process.exit(0);
