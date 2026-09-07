# RCA: v0.9.0 still delivers cell-sized SGR after #3637

Analysis only. No patch. Scoped to Herdr **v0.9.0** (`b99002ac`, 18 commits ahead of `3a822e81`) and PR **#3637** (`3a822e8106b45745f9d463d9167033f977353d9f`).

Live host: Ghostty 1.3.1. Live Herdr: 0.9.0 linux x86_64 release. Probe is an ordinary child PTY (Python / Bubble Tea enable), not terminal-browser.

## Verdict

**#3637 did what it said on a different path.** It stops libghostty-vt from *encoding* an already-accepted pixel position as cells after a child `1006h` reassert. It does **not** make the ClientShell host start sending pixels to a non-graphics pane.

The nest still gets cells because:

1. v0.9.0 local UI is ClientShell (`#3487`).
2. ClientShell asks the **host** for mode 1016 only when the focused pane has an active Kitty graphics layer **and** the child has 1016 set.
3. The Bubble Tea / Python probe has 1016 SET (DECRQM `?1016;1$y`) but **no graphics layer**.
4. Host Ghostty therefore stays on cell SGR (`EnableMouseCapture` = `1000h 1002h 1003h 1006h`, no `1016h`).
5. The client never builds `ClientMousePosition::Pixels`. `#3637`’s encode override never runs.
6. Child DECRQM is answered by Herdr’s pane emulator, not by host Ghostty. SET + cell reports is still a supported combination after `#3637`.

Trailing `1006h` on the **child** is a red herring for Image 1. The same nest would stay cell-sized with the clean enable sequence, because host 1016 is never turned on.

Trailing `1006h` on **bare Ghostty** (Images 2 vs 3) is a real host-terminal format/mode split. It is the same split `#3637` papered over on the *encode* side. It is not what the nest sends to the host.

Hypothesis **A** (host Ghostty starved of pixels because of the child’s trailing `1006h`): **discarded for the nest.** Confirmed only for bare Ghostty.
Hypothesis **B** (Herdr still clears child pixel encode on `1006h` while DECRQM answers SET): **discarded for `Position::Pixels`.** `#3637` fixed that. **Still true** that DECRQM SET + cell encode happens when the incoming position is `Cell`.
Hypothesis **C** (both as the nest cause): **discarded.**

## What #3637 actually changed

PR: https://github.com/herdrdev/herdr/pull/3637
Issue: https://github.com/herdrdev/herdr/issues/3295
Merge: `3a822e81` — `fix: preserve pixel mouse after sgr reassertion`

Three files:

| File | Change |
|---|---|
| `src/ghostty/mod.rs` | Export `MOUSE_FORMAT_SGR_PIXELS`. |
| `src/pane/input.rs` | After `encoder.set_from_terminal()`, if mode 1016 is still SET and the event is `Position::Pixels`, force format `SGR_PIXELS`. Cell input still forces `SGR` (cell). |
| `src/server/headless/tests/mod.rs` | Inject `Pixels { x: 403, y: 240 }` after child bytes `1003h 1006h 1016h 1006h`. Expect `\x1b[<0;403;240M`. |

That is the exact synthetic measurement from the #3295 reopen (mid-line click encoded as `\x1b[<0;41;13M` = 403/10, 240/20). The test never talks to a host terminal.

Release note (“Pixel mouse coordinates remain correct when pane applications reassert SGR mouse reporting”) describes this encode fix, not host capture.

## Host mouse → child encode (v0.9.0 ClientShell)

```
host Ghostty
  → stdin SGR (`\x1b[<btn;x;yM`)
  → classify_unix_input  [PixelMouse only if host_sgr_pixels_active]
  → ClientShell.handle_pixel_mouse / pane_mouse_position
  → ClientPaneInputEvent::Mouse { position, geometry }
  → downgrade_ineligible_pixel_mouse  [needs pixel_mouse && host_sgr_pixels_active]
  → apply_client_pane_input_events
  → ghostty_mouse_encoder_for_terminal   ← #3637 lives only here
  → bytes into child PTY
```

Child CSI `1006h` / `1016h` is parsed by the pane’s libghostty-vt. It is **not** forwarded to host Ghostty.

Host enable (`src/client/terminal_setup.rs::set_mouse_capture`):

```
clear 1006l 1016l 1003l 1002l 1000l
EnableMouseCapture          → 1000h 1002h 1003h 1006h
if sgr_pixels: 1016h        → clean order, no trailing 1006
```

`effective_sgr_pixel_mouse = enabled && requested && exact_geometry`.

ClientShell request (`src/server/headless/render.rs::stream_host_mouse_capture_mode`):

```
sgr_pixels = client.pixel_mouse
          && pane_graphics.active_for_pane(pane_id)   // Kitty layer present
          && runtime.sgr_pixel_mouse_enabled()        // DEC 1016 SET
```

Locked in by `pixel_mouse_activation_requires_graphics_demand_not_direct_transport`:
child `1003h 1006h 1016h` + `pixel_mouse=true` → `MouseCapture { sgr_pixels: false }` until a graphics layer is stored.

TerminalAttach (legacy direct attach) does **not** use the graphics gate. Local 0.9.0 launches use ClientShell.

If host 1016 is off:

- `classify_unix_input` treats reports as raw cell mouse.
- `handle_pixel_mouse` never runs; `host_mouse_pixels` stays `None`.
- `pane_mouse_position` sees `hit.sgr_pixel_mouse == true` (surface bit from child mode) but falls back to `Cell`.
- `downgrade_ineligible_pixel_mouse` would also strip any leaked `Pixels` because `host_sgr_pixels_active != true`.
- Encoder `Position::Cell` + mode 1016 SET → `#3637` **forces cell SGR**.

DECRQM `?1016$p` is answered by the pane emulator (`mode_get(1016)`). Mode bit and encoder format are different flags.

## libghostty-vt / Ghostty format vs mode

Vendored handler (`vendor/libghostty-vt/src/termio/stream_handler.zig`):

```
1006h → flags.mouse_format = .sgr
1016h → flags.mouse_format = .sgr_pixels
```

These overwrite one **format** enum. They do **not** clear the other **mode** bit.

So `1006h 1016h 1006h` leaves:

- mode 1016 SET → DECRQM `?1016;1$y`
- `mouse_format = .sgr` → `set_from_terminal()` inherits cell SGR

That is the bug `#3637` patched on encode, and the same split Image 3 shows on bare Ghostty.

`#3637` does **not** change DECRQM. After the fix, **DECRQM SET + cell encode is still possible** whenever the encoder is given `Position::Cell`. The new test only covers `Position::Pixels`.

## Live evidence mapped

| Shot | Where | Enable | DECRQM 1016 | Reports | Meaning |
|---|---|---|---|---|---|
| Image 1 | Ghostty → Herdr 0.9.0 pane | BT (`…1016h 1006h`) | SET (Herdr pane VT) | cell (`x=16/36/48` on cols=52) | Host 1016 never requested. Encode got `Cell`. |
| Image 2 | Ghostty alone | clean (`…1006h 1016h`) | SET | pixel-ish (`x` to 698, `y=252` on 79×26) | Host Ghostty can emit pixels. |
| Image 3 | Ghostty alone | BT (`…1016h 1006h`) | SET | cell | Host Ghostty: trailing `1006h` resets **format** to cell, leaves mode SET. |

Image 2 vs 3 is Ghostty. Image 1 is Herdr ClientShell withholding host 1016. Do not collapse them.

Image 1 DECRQM is **not** a reading of host Ghostty mode 1016.

## #3295 / #3487 / #3637 roles

- `#3295` original: terminal-browser, DECRQM SET, cell clicks, `pixel_mouse: true`. Graphics app.
- Closed after `#3487` retest: terminal-browser on ClientShell with **already pixel-positioned** host events (`604,283` → pane `140,251`). Graphics demand satisfied; host 1016 on.
- Reopen (SLatz18): ordinary child PTY, synthetic encode after `1006h 1016h 1006h` produced cells. That is the `#3637` test.
- `#3637` shipped in v0.9.0. It does not touch `stream_host_mouse_capture_mode` or the graphics gate.
- Live nest (this RCA): same ordinary child PTY, **real host clicks**, still cells. Different failure than the synthetic encode.

## Maintainer-ready bullets

- `#3637` only forces `MOUSE_FORMAT_SGR_PIXELS` when encoding `Position::Pixels` while DEC 1016 remains SET. Regression test injects pixels; it does not enable host 1016.
- ClientShell host 1016 still requires `pane_graphics.active_for_pane`. Test: `pixel_mouse_activation_requires_graphics_demand_not_direct_transport`.
- v0.9.0 default attach is ClientShell. A 1016-only child (Bubble Tea, this Python probe) keeps DECRQM SET and receives cell SGR.
- Child `1006h`/`1016h` never reach host Ghostty. Host sequence is `EnableMouseCapture` then optional `1016h` (clean). Image 3 is not the nest’s host sequence.
- Bare Ghostty 1.3.1: clean enable → pixels (Image 2); trailing `1006h` → DECRQM SET + cells (Image 3). Same format/mode split as libghostty-vt.
- After `#3637`, DECRQM SET + cell encode remains possible and is the Image 1 path.
- Release note for `#3295` overstates the live nest. Encode-after-reassert is fixed; host pixels for non-graphics 1016 children are not.

## Smallest correct Herdr change (prose only)

If the contract is “DECRQM `?1016` SET and `pixel_mouse: true` ⇒ child reports are pixel-magnitude whenever the host can do 1016”:

Align ClientShell’s `sgr_pixels` predicate with TerminalAttach: `client.pixel_mouse && runtime.sgr_pixel_mouse_enabled()` (drop `pane_graphics.active_for_pane`). Keep `#3637`. Keep `effective_sgr_pixel_mouse`’s exact-geometry guard. Host then sends the Image 2 sequence; the child encode path already preserves pixels after a trailing `1006h`.

If the graphics gate is intentional, the release note / `#3295` closer should say pixel reports are for graphics panes only. Then Image 1 is expected, and DECRQM SET on a text-only 1016 child is the remaining inconsistency to document or to answer RESET.

Do not “fix” child trailing `1006h` by changing DECRQM alone. Mode 1016 staying SET is what `#3637` relies on.
