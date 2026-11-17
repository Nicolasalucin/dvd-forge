---
name: desktop-hardening
description: Harden this Tauri + Rust desktop app so malware cannot use it as a springboard (binary/DLL planting, abusing its external tools or IPC to run code or read/write files, poisoned project/bundle files, tampered downloads). Use when touching process spawning, tool discovery/provisioning, file import/export, Tauri commands/capabilities/CSP, the local media server, release packaging or signing, and before every release. Complements the generic `security-and-hardening` and `owasp-security` skills with desktop-specific threats.
---

# Desktop hardening (anti-springboard)

Threat model: the attacker already controls **some** data or a low-privilege
foothold on the machine (a downloaded project/bundle/video/subtitle, a
writable folder, a web page) and wants the app — a signed, trusted-looking
binary that launches other programs — to execute code, escalate, persist or
exfiltrate. The app must never be more powerful than the person using it.

Work through each area: find the code, verify, fix, add a test. Report each
item as OK / Fixed / Risk accepted (with reason).

## 1. Process launching (FFmpeg, DVDAuthor, burners)

- Spawn by **absolute, resolved path** from a fixed search order
  (`kinodisc-toolchain::resolve_binary`). Never resolve executables from the
  current working directory, the project folder or a media folder.
- Arguments as a vector, **never through a shell**. File names that start
  with `-` must not become options (prefix `./` or use `file:`/`--`).
- FFmpeg inputs: keep the protocol whitelist (`MEDIA_PROTOCOL_WHITELIST`);
  reject `concat:`/`http:`/`pipe:`/`data:` names coming from projects.
- Timeouts, kill-on-drop, no inherited handles; never run elevated.

## 2. Binary and DLL planting (Windows)

- The per-user toolchain folder (`%LOCALAPPDATA%\…\toolchain`) holds DLLs:
  it must be writable only by the user, and the app must refuse to run
  tools from folders writable by other users (`Program Files` installs
  stay read-only).
- Tools are verified by the **pinned SHA-256** of the downloaded archive;
  record per-file hashes in the provenance file and re-check them before
  launching downloaded tools.
- Do not add `.`/the exe folder to the DLL search path; prefer
  `SetDefaultDllDirectories` semantics. Installer: per-machine installs into
  `Program Files`, never into a user-writable folder that runs elevated.

## 3. Untrusted inputs (projects, bundles, media, subtitles)

- Project JSON paths: absolute paths are allowed only for user-chosen media;
  never let a project point the app at writing outside the output folder.
- Bundles: manifest-first, traversal-safe, size+hash per entry, full rollback
  (`kinodisc-project::bundle`). Keep the corrupted-bundle tests.
- Parsers (UDF/IFO/BDMV/PGS/SRT/ASS/VTT): bounded allocations, checked
  integer math (i686!), no panics — keep the seeded robustness tests green.
- Output: refuse to overwrite system folders; write via temp + rename.

## 4. WebView / IPC surface

- CSP stays strict (`script-src 'self'`, no remote origins, no `eval`).
- Capabilities: least privilege (`apps/desktop/src-tauri/capabilities`).
  Every `#[tauri::command]` treats its arguments as hostile: validate paths
  (exist, expected type/extension, inside allowed roots), cap sizes.
- `open_folder` and any "reveal/open" command must open folders only, never
  launch files or URLs.
- The local media server binds `127.0.0.1`, serves only registered files via
  unguessable tokens, and only `GET`/`HEAD` with `Range`.
- Release builds never enable WebView DevTools or a remote debugging port
  (the `app-walkthrough` config is debug-only).

## 5. Downloads, updates and releases

- Every download: HTTPS, pinned size + SHA-256 checked **before** extract or
  run; no scripts executed from archives.
- Release artifacts: publish `SHA256SUMS`; sign Windows/macOS binaries when a
  certificate is available; CI uses pinned actions and least-privilege
  `permissions`.
- Before pushing: scan the diff for secrets, tokens and personal paths.

## Verification

- `cargo test --workspace` (robustness tests), `cargo clippy -D warnings`.
- Add a regression test for every fixed issue.
- Record results in `docs/security/<date>-review.md`.
