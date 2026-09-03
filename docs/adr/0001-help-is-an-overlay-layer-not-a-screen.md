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
- The prior screen renders beneath the overlay. This is only safe because `?` is
  gated to stages that do not capture text: the three cursor-setting render paths
  (`fuzzy_picker`, `render_text_input_dialog`, the config editor while editing) are
  exactly the text-capturing stages, and ratatui 0.30 offers no way to unset a
  cursor position once a frame has set one.
- A git operation that succeeds while help is open still auto-closes to the
  dashboard beneath, so closing help then lands on the dashboard.
