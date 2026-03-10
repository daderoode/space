use ratatui::style::{Color, Modifier, Style};

// ── Primary palette ──────────────────────────────────────────────────────────
pub const TEAL: Color         = Color::Rgb(0, 188, 180);
pub const MINT: Color         = Color::Rgb(100, 220, 180);
pub const LIGHT_BLUE: Color   = Color::Rgb(130, 190, 255);
pub const MUTED: Color        = Color::Rgb(100, 110, 120);
pub const TEXT: Color         = Color::White;
pub const DIM_TEXT: Color     = Color::Rgb(160, 165, 170);
pub const INPUT: Color        = Color::White;
pub const ERROR: Color        = Color::Rgb(255, 100, 100);
pub const WARN: Color         = Color::Rgb(240, 200, 80);
pub const BG_HIGHLIGHT: Color = Color::Rgb(30, 50, 50);

// ── Semantic styles ──────────────────────────────────────────────────────────
pub fn border_focused()   -> Style { Style::default().fg(TEAL) }
pub fn border_unfocused() -> Style { Style::default().fg(MUTED) }
pub fn border_danger()    -> Style { Style::default().fg(ERROR) }
pub fn title()            -> Style { Style::default().fg(TEAL).add_modifier(Modifier::BOLD) }
pub fn selected()         -> Style { Style::default().fg(MINT).add_modifier(Modifier::BOLD) }
pub fn highlight_row()    -> Style { Style::default().fg(TEXT).bg(BG_HIGHLIGHT) }
pub fn muted()            -> Style { Style::default().fg(MUTED) }
pub fn text()             -> Style { Style::default().fg(TEXT) }
pub fn dim_text()         -> Style { Style::default().fg(DIM_TEXT) }
pub fn input_style()      -> Style { Style::default().fg(INPUT) }
pub fn error()            -> Style { Style::default().fg(ERROR) }
pub fn success()          -> Style { Style::default().fg(MINT) }
pub fn branch()           -> Style { Style::default().fg(LIGHT_BLUE) }
pub fn warn()             -> Style { Style::default().fg(WARN) }
pub fn status_clean()     -> Style { Style::default().fg(MINT) }
