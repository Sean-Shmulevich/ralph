//! ratatui TUI dashboard for `ralph watch`.
//!
//! Layout:
//! ```text
//! ╔═══════════════════════════════════════════════════════════╗
//! ║ Ralph — 2 loops active                                    ║
//! ╠═══════════════════════════════════════════════════════════╣
//! ║ auth-system  │ claude │ T3/8  │ ████░░░░ 37% │  12m ago  ║
//! ║ api-refactor │ gemini │ T1/5  │ ██░░░░░░ 20% │  running  ║
//! ╠═══════════════════════════════════════════════════════════╣
//! ║ [auth-system] Implementing OAuth2 callback handler...     ║
//! ║ > Created src/auth/callback.rs                            ║
//! ╚═══════════════════════════════════════════════════════════╝
//! ```

use std::io::{self, Stdout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame, Terminal,
};

use crate::state::{LoopState, SharedLoopStatus};

// ── TUI state ─────────────────────────────────────────────────────────────────

struct TuiApp {
    loops: Vec<SharedLoopStatus>,
    selected: usize,
    log_scroll: u16,
    /// Cached row of the table for keyboard navigation.
    table_state: TableState,
}

impl TuiApp {
    fn new(loops: Vec<SharedLoopStatus>) -> Self {
        let mut table_state = TableState::default();
        if !loops.is_empty() {
            table_state.select(Some(0));
        }
        Self {
            loops,
            selected: 0,
            log_scroll: 0,
            table_state,
        }
    }

    fn select_next(&mut self) {
        if self.loops.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.loops.len();
        self.table_state.select(Some(self.selected));
        self.log_scroll = 0;
    }

    fn select_prev(&mut self) {
        if self.loops.is_empty() {
            return;
        }
        self.selected = self.selected.saturating_sub(1);
        if self.selected == 0 && self.loops.len() > 1 {
            // allow wrapping up
        }
        self.table_state.select(Some(self.selected));
        self.log_scroll = 0;
    }

    fn scroll_down(&mut self) {
        self.log_scroll = self.log_scroll.saturating_add(3);
    }

    fn scroll_up(&mut self) {
        self.log_scroll = self.log_scroll.saturating_sub(3);
    }

    fn scroll_to_bottom(&mut self) {
        // Large value — ratatui will clamp to actual content height
        self.log_scroll = u16::MAX;
    }
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Run the TUI dashboard in the current thread (blocking).
/// This should be called from a dedicated `std::thread::spawn`.
///
/// Returns when the user presses `q`/`Q`/`Ctrl-C` or `cancel_flag` becomes true.
pub fn run_tui(loops: Vec<SharedLoopStatus>, cancel_flag: Arc<AtomicBool>) -> anyhow::Result<()> {
    if loops.is_empty() {
        return Ok(());
    }

    let mut terminal = setup_terminal()?;
    let mut app = TuiApp::new(loops);
    // Start scrolled to bottom so users see latest logs immediately
    app.scroll_to_bottom();

    let tick_rate = Duration::from_millis(200);

    let result = run_loop(&mut terminal, &mut app, &cancel_flag, tick_rate);

    // Always restore terminal, even on error
    let _ = restore_terminal(&mut terminal);

    result
}

// ── Main event loop ───────────────────────────────────────────────────────────

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut TuiApp,
    cancel_flag: &Arc<AtomicBool>,
    tick_rate: Duration,
) -> anyhow::Result<()> {
    loop {
        // Exit if all loops are finished or cancel was requested externally
        if cancel_flag.load(Ordering::Relaxed) {
            break;
        }

        terminal.draw(|f| render(f, app))?;

        // Poll for keyboard events with a short timeout so we keep redrawing
        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                match (key.code, key.modifiers) {
                    // Quit
                    (KeyCode::Char('q'), _)
                    | (KeyCode::Char('Q'), _)
                    | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                        cancel_flag.store(true, Ordering::Relaxed);
                        break;
                    }
                    // Navigate loops
                    (KeyCode::Tab, _) | (KeyCode::Right, _) => app.select_next(),
                    (KeyCode::BackTab, _) | (KeyCode::Left, _) => app.select_prev(),
                    (KeyCode::Up, _) => {
                        app.select_prev();
                    }
                    (KeyCode::Down, _) => {
                        app.select_next();
                    }
                    // Scroll logs
                    (KeyCode::Char('j'), _) | (KeyCode::PageDown, _) => app.scroll_down(),
                    (KeyCode::Char('k'), _) | (KeyCode::PageUp, _) => app.scroll_up(),
                    (KeyCode::Char('G'), _) | (KeyCode::End, _) => app.scroll_to_bottom(),
                    (KeyCode::Char('g'), _) | (KeyCode::Home, _) => app.log_scroll = 0,
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

// ── Rendering ─────────────────────────────────────────────────────────────────

fn render(frame: &mut Frame, app: &mut TuiApp) {
    let area = frame.area();

    // Count active (non-terminal) loops
    let active_count = app
        .loops
        .iter()
        .filter(|ls| {
            ls.lock()
                .map(|s| {
                    !matches!(
                        s.state,
                        LoopState::Complete | LoopState::Failed(_) | LoopState::Stopped
                    )
                })
                .unwrap_or(false)
        })
        .count();

    // Table height: header border + 1 header row + N data rows + footer border = N+3
    let table_height = (app.loops.len() as u16 + 3).min(area.height.saturating_sub(6));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),               // title bar
            Constraint::Length(table_height),    // loops table
            Constraint::Min(3),                  // log view
        ])
        .split(area);

    render_title(frame, chunks[0], active_count, app.loops.len());
    render_table(frame, chunks[1], app);
    render_logs(frame, chunks[2], app);
}

fn render_title(frame: &mut Frame, area: ratatui::layout::Rect, active: usize, total: usize) {
    let title = format!(" Ralph — {}/{} loops active  [Tab] switch  [↑↓jk] scroll  [q] quit ",
        active, total);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let paragraph = Paragraph::new(title)
        .block(block)
        .style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD));
    frame.render_widget(paragraph, area);
}

fn render_table(frame: &mut Frame, area: ratatui::layout::Rect, app: &mut TuiApp) {
    let header_cells = ["Name", "Agent", "Progress", "Status", "Time"]
        .iter()
        .map(|h| {
            Cell::from(*h).style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        });
    let header = Row::new(header_cells).height(1).bottom_margin(0);

    let rows: Vec<Row> = app
        .loops
        .iter()
        .map(|ls| {
            let s = match ls.lock() {
                Ok(s) => s,
                Err(_) => return Row::new(vec![Cell::from("error")]),
            };

            let name_cell = Cell::from(s.name.clone());
            let agent_cell = Cell::from(s.agent.clone());
            let progress_cell = Cell::from(make_progress_bar(s.tasks_done, s.tasks_total, 12));
            let (status_text, status_color) = state_display(&s.state);
            let status_cell =
                Cell::from(status_text).style(Style::default().fg(status_color));
            let time_cell = Cell::from(s.elapsed_str());

            Row::new(vec![
                name_cell,
                agent_cell,
                progress_cell,
                status_cell,
                time_cell,
            ])
            .height(1)
        })
        .collect();

    let selected_style = Style::default()
        .bg(Color::DarkGray)
        .add_modifier(Modifier::BOLD);

    let table = Table::new(
        rows,
        [
            Constraint::Min(16),    // name
            Constraint::Length(8),  // agent
            Constraint::Length(18), // progress bar
            Constraint::Length(12), // status
            Constraint::Length(8),  // time
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(" Loops "),
    )
    .row_highlight_style(selected_style);

    frame.render_stateful_widget(table, area, &mut app.table_state);
}

fn render_logs(frame: &mut Frame, area: ratatui::layout::Rect, app: &mut TuiApp) {
    let (loop_name, log_lines) = match app.loops.get(app.selected) {
        None => ("<none>".to_string(), vec![]),
        Some(ls) => match ls.lock() {
            Err(_) => ("<lock error>".to_string(), vec![]),
            Ok(s) => {
                let name = s.name.clone();
                let lines: Vec<Line> = s
                    .recent_logs
                    .iter()
                    .map(|l| Line::from(Span::raw(strip_ansi(l))))
                    .collect();
                (name, lines)
            }
        },
    };

    // Auto-scroll: if user hasn't manually scrolled up, keep at bottom
    let content_height = log_lines.len() as u16;
    let view_height = area.height.saturating_sub(2); // minus borders
    let max_scroll = content_height.saturating_sub(view_height);

    // Clamp scroll to valid range
    if app.log_scroll > max_scroll {
        app.log_scroll = max_scroll;
    }

    let title = format!(" [{loop_name}] Logs  (line {}/{})", app.log_scroll + view_height, content_height);
    let paragraph = Paragraph::new(log_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(title),
        )
        .scroll((app.log_scroll, 0));

    frame.render_widget(paragraph, area);
}

// ── Terminal setup/restore ────────────────────────────────────────────────────

fn setup_terminal() -> anyhow::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> anyhow::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

// ── Rendering helpers ─────────────────────────────────────────────────────────

/// Build a progress bar string like `████░░░░ 50%` of the given width.
fn make_progress_bar(done: u32, total: u32, bar_width: usize) -> String {
    if total == 0 {
        return format!("{} ---%", "░".repeat(bar_width));
    }
    let pct = (done as f32 / total as f32).min(1.0);
    let filled = (pct * bar_width as f32).round() as usize;
    let empty = bar_width.saturating_sub(filled);
    format!(
        "{}{} {:3.0}%",
        "█".repeat(filled),
        "░".repeat(empty),
        pct * 100.0
    )
}

fn state_display(state: &LoopState) -> (String, Color) {
    match state {
        LoopState::Starting => ("starting".to_string(), Color::DarkGray),
        LoopState::Parsing => ("parsing…".to_string(), Color::Yellow),
        LoopState::Running => ("running".to_string(), Color::Green),
        LoopState::Complete => ("complete".to_string(), Color::Cyan),
        LoopState::Failed(_) => ("failed".to_string(), Color::Red),
        LoopState::Stopped => ("stopped".to_string(), Color::Gray),
    }
}

/// Strip ANSI escape sequences from a string for clean terminal rendering.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // ESC [ ... final_byte  (CSI sequence)
            if chars.peek() == Some(&'[') {
                chars.next();
                for c2 in chars.by_ref() {
                    if c2.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else if chars.peek() == Some(&']') {
                // OSC sequence: ESC ] ... ST (ESC \ or BEL)
                chars.next();
                let mut prev = '\0';
                for c2 in chars.by_ref() {
                    if c2 == '\x07' || (prev == '\x1b' && c2 == '\\') {
                        break;
                    }
                    prev = c2;
                }
            }
            // else: skip lone ESC
        } else {
            out.push(c);
        }
    }
    out
}
