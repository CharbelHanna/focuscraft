# FocusCraft — handover

Context carried over from an Argentum session on 2026-11-11, where focus stacking
was investigated as an Argentum feature before being split out into this
standalone application. Nothing was built here yet; the repo is empty.

## What this is meant to be

A standalone focus-stacking application: take a group of photos shot at
different focal planes and produce one merged, rendered photo.

Decisions already taken in the investigation:

| Question | Answer |
|---|---|
| Kind of stacking | Focus stacking (extended depth of field) |
| Output | A new editable photo, not just a flat export |
| Grouping | Persistent stacks the user creates in the library |
| Auto-detection | No — the user picks the frames |

The "persistent stacks" and "editable output" decisions were made when this was
going to live inside Argentum, which already had a library and an editor. As a
standalone app both need rethinking: there is no library to persist a stack in
and no editor to reopen the result in, so decide what those mean here before
building to them.

## Licence — decided

**FocusCraft is AGPL-3.0.** The full licence text is in `LICENSE` (verbatim
FSF text, same as Argentum's). The network-use source obligation of section 13
applies: if this is ever offered to users over a network, they must be able to
get the source.

That settles the legal question the original handover raised — because
FocusCraft is AGPL-3.0, porting Argentum's `focus_stacking.rs` (inherited
RapidRAW code under the same licence) would have been permitted.

**It is still being written from published descriptions rather than ported.**
That is now a deliberate design choice, not a licence requirement. Reasons it is
worth keeping to:

- The output-encoding and scene-linear decisions below (see "the one hard-won
  lesson") are the main thing this app should do differently. Porting imports
  the display-referred assumptions those decisions are coupled to.
- The "persistent stacks" / "editable output" shape still has to be redesigned
  for a standalone app with no library and no editor.

Working rule that follows from this: write the algorithm from papers and
published descriptions, not from reading `focus_stacking.rs`. Keeping to that
leaves the implementation genuinely independent, which preserves the option of
relicensing later. It is not a legal obligation, so if consulting the Argentum
source for a specific parameter or sanity check is ever worth more than that
option, that is a free choice to make — just make it knowingly.

## What exists in Argentum, as prior art only

`C:\Coding\Repos\Argentum\src-tauri\src\focus_stacking.rs` is ~2200 lines and
already complete. The summary below is what a finished implementation needs to
contain — a checklist of parts, not a design to copy. Per the working rule
above, build these from published descriptions; the file itself stays unread.

- Pyramidal inverse-compositional alignment over a 22-parameter lens warp
  (affine + radial k1/k2/k3 + polynomial), solved at a downscaled size and
  rescaled to full resolution.
- Per-frame pose regularisation with a weighted quadratic fit across the stack.
- Multi-scale per-pixel focus measurement, a depth label per pixel, then a
  weighted Laplacian-pyramid merge and an auto-crop to the valid region.
- A `FrameSource` trait (`len`/`dims`/`get`) with two implementations: all frames
  in memory, or f32 planes spilled to a temp directory, chosen against a 2 GiB
  budget. Estimate used: `N × W × H × 3 × 4` bytes.
- Alignment is to the **middle** frame (`n / 2`), not the first. The merged
  result is therefore in the middle frame's geometry — this matters for any
  metadata or crop carried onto the output.

## The one hard-won lesson worth carrying over

Argentum's stack had a real, shipping bug that is easy to reproduce in any app
that decodes RAW and writes a TIFF:

Frames were decoded, then put through a **display encoding** (a gamma 2.38 curve
plus a contrast line, clamped at 1.0). The merge preserved that encoding, and the
result was written straight out as a 16-bit TIFF. But the editor reopened that
TIFF as an ordinary image and decoded it with the **sRGB** curve. Two different
curves, so the stack opened darker and flatter than the frames that made it —
before any adjustment was applied. Had a saved "look" then been applied on top,
it would have been applied to values it was never measured against.

**The rule:** whatever curve the consumer will decode with, encode with exactly
its inverse. Decide the output's colour encoding deliberately and write it down,
rather than letting it fall out of whichever decode path ran first.

A second, related trap in the same code: the display encoding clamped at 1.0,
which it reached at scene-linear 0.759. Highlights above that were destroyed
*before* the merge ran, so no amount of fixing the output encoding could recover
them. If this app merges in scene-linear light instead, it avoids both problems —
but note that Argentum's alignment and focus-measurement were tuned against
display-referred values, so the two choices are coupled.

## Machine prerequisites — installed and verified 2026-11-13

All build prerequisites are in place and tested end to end:

| Component | Version |
|---|---|
| Rust (stable-x86_64-pc-windows-msvc) | rustc 1.100.0, cargo 1.100.0, rustup 1.29.3 |
| MSVC C++ toolset | 14.44.35207 (VS Community 2022) |
| Windows SDK | 10.0.26100.0 |
| WebView2 runtime (Tauri) | 157.0.4341.46 |
| Node / npm | v23.11.0 / 10.9.2 |

Verified by building and running, not just by checking versions: a C program
through `vcvars64` + `cl.exe`; `cargo new` + `cargo build` + run; and a crate
with a `cc` build script compiling and linking C into a Rust binary. That last
one matters — RAW and codec crates build C this way, and it also confirms
crates.io is reachable from this network.

Notes that still apply:

- Build on **Windows, not WSL**. A desktop GUI app built in WSL produces a Linux
  binary you cannot run, and building across `/mnt/c` is very slow. WSL2 Ubuntu is
  available on this machine but is the wrong tool for this.
- In git bash, `which link` still resolves to `/usr/bin/link`. In practice this
  does **not** break cargo builds — rustc locates MSVC's `link.exe` itself via
  vswhere rather than through PATH, and a full build from git bash succeeded with
  the shadow in place. It can still bite hand-rolled `cl`/`link` invocations, so
  use a Developer Command Prompt for those.
- `cmd` on this machine will not run a bare `prog.exe` from the current directory
  (`NoDefaultCurrentDirectoryInExePath` is set). Use `.\prog.exe`. A build step
  can look like it failed at the run stage because of this.
- MSVC here is **French-localised** — compiler diagnostics come back in French.
- Node v23.11.0 is a non-LTS release that some tooling rejects on engine checks.
  `npm` also fails from git bash (`cannot execute: required file not found`);
  `npm.cmd` works, so call it via `cmd //c npm ...`.

Install gotcha worth recording: `rustup-init.exe` must be run **unelevated**, as
your own user. Run elevated, it installs into the elevating account's profile —
on a domain machine that can be a hidden `C:\Users\<DOMAIN>_<user>_$` profile the
normal account cannot even read — and writes that account's PATH, leaving `cargo`
invisible from an ordinary shell. Any such stray profile directories are best
left alone.

## State of the Argentum repo

Clean. An encoding fix had been started there and was fully reverted when this
split was decided — `git status` in `C:\Coding\Repos\Argentum` shows no changes
from that work. Nothing is pending on the Argentum side.
