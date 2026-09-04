# Help is an overlay layer on `App`, not a `Screen` variant

`Screen::Help` was a unit variant, so opening help replaced the current screen and
closing it returned to the dashboard. Making help work from inside a flow, and
return to that flow, looked like it wanted `Screen::Help { return_to: Box<Screen> }`.
It does not: `poll_sync_result` and `poll_gitop_result` gate on the current `Screen`
variant, and when the gate is false they do not skip, they set the worker's cancel
flag and drop the receiver. Wrapping a mid-sync or mid-fetch screen inside a `Help`
variant would therefore cancel the work it was showing. So help is `App.help:
Option<HelpState>`, an overlay layer rendered on top of the current screen, and the
screen is never moved.

## Consequences

- Every `matches!(&self.screen, ...)` guard, worker poll and auto-close timer keeps
  working unchanged while help is open, and background work continues behind it.
- The prior screen renders beneath the overlay, and the renderer suppresses the
  terminal cursor while help is open: `view()` computes `show_cursor =
  app.help.is_none()` and threads it to the three cursor-setting paths
  (`fuzzy_picker`, `render_text_input_dialog`, the config editor while editing).
  This has to be a property of the renderer rather than of the key gate. An
  earlier version of this decision argued the cursor could not appear because
  `?` is gated out of text-capturing stages, which is where all three paths
  live. That reasoning covered `?` alone: `F1` exists precisely to open help
  from inside a text input, and ratatui 0.30 offers no way to unset a cursor
  once a frame has set one, so the cursor was painted over the dialog.
- A git operation that succeeds while help is open still auto-closes to the
  dashboard beneath, so closing help then lands on the dashboard.
