---
name: app-walkthrough
description: Use the real desktop app the way a normal person would — open it, try to finish real tasks (make a DVD of holiday videos, burn a TV season, make an ISO/VIDEO_TS folder with every chapter), watch where they get stuck, and list missing features against DVDStyler and similar tools. Use when asked to "see how a user uses the app", "what is missing", "usability test", "dogfood", or before/after any UI rework. Drives the built Tauri app through the WebView2 debugging port (no extra dependencies).
---

# App walkthrough (real user, real app)

Goal: evidence, not opinions. Every finding comes from something you *did* in
the running app (clicked, typed, waited, looked at a screenshot). A pass with no
interaction is **Incomplete**. Pair with the `ux-audit` skill for its
checklists; this skill is the DVD-authoring-specific, desktop-specific part.

## 1. Build and launch the walkthrough build (Windows)

The debug build exposes WebView2's DevTools port **only** through this
config; release builds never do (see the `desktop-hardening` skill).

```bash
cd apps/desktop
npx tauri build --debug --no-bundle --config ../../.claude/skills/app-walkthrough/scripts/walkthrough-config.json
```

Launch with a project (or none, for a first-run test):

```powershell
$env:KINODISC_PROJECT = "<path to a project.json>"   # omit for first run
Start-Process "<repo>\target\debug\kinodisc-desktop.exe"
```

Reset first-run state when testing onboarding:
`node scripts/cdp.mjs eval "localStorage.clear(); location.reload()"`.

## 2. Drive it

`scripts/cdp.mjs` (Node ≥ 22, no packages):

```bash
node scripts/cdp.mjs eval "<js>"                 # run JS in the page, print JSON result
node scripts/cdp.mjs shot out.png "<js>"         # optional JS, wait 1.5 s, 1600x900 screenshot
```

Useful helpers to prepend to `<js>`:

```js
window.__click=(t)=>{const n=x=>x.innerText.replace(/\s+/g,' ').trim();
 const bs=[...document.querySelectorAll('button,[role=button]')];
 const b=bs.find(x=>n(x)===t)||bs.find(x=>n(x).includes(t)); if(b){b.click();return true}return false};
window.__set=(sel,val)=>{sel.value=val;sel.dispatchEvent(new Event('change',{bubbles:true}))};
```

Keyboard: `window.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape'}))`.
Native file dialogs cannot be scripted: open projects with `KINODISC_PROJECT`,
or call the store from the console for the step after the dialog, and note
that the dialog itself was not tested.

Always **Read the screenshot** after a step; judge what a person would see.

## 3. Personas and tasks

Run at least two personas per pass, in the interface language they would use.
The full task list and the missing-feature checklist are in
[references/personas-and-tasks.md](references/personas-and-tasks.md).

For each task record: steps taken, where you hesitated or got lost (that is
the finding), time/clicks, errors, and whether it was completed.

## 4. Report

Write `docs/walkthroughs/<date>-<persona>.md`:

- Verdict: Completed / Completed with friction / Blocked / Incomplete.
- Findings ranked by impact on a real person (Blocker, Major, Minor, Polish),
  each with steps to reproduce, screenshot path, and the suspected file.
- Missing features (checklist results), each marked Have / Partial / Missing.
- What worked well (keep it).

Fix-and-verify: after fixing a finding, rerun that task and attach the new
screenshot. Close the app when done (`taskkill /IM kinodisc-desktop.exe /F`).
