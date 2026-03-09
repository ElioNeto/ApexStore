//! ApexStore Interactive TUI Dashboard
//!
//! Run with: `cargo run --bin apexstore-tui`
//! Quit with: `q` or `Esc`
//!
//! Commands available in the input panel:
//!   get <key>            - Simulate a GET operation
//!   set <key> <value>    - Simulate a SET operation
//!   del <key>            - Simulate a DELETE operation
//!   stats                - Print current statistics
//!   clear                - Clear the command log
//!   help                 - Show help

use chrono::Local;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        BarChart, Block, Borders, Clear, Gauge, List, ListItem, Padding, Paragraph, Wrap,
    },
    Frame, Terminal,
};
use std::{
    collections::VecDeque,
    io,
    panic,
    time::{Duration, Instant},
};
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;

// ─── Color Palette ─────────────────────────────────────────────────────────────
const C_APEX_ORANGE: Color = Color::Rgb(255, 110, 30);
const C_APEX_AMBER: Color = Color::Rgb(255, 180, 0);
const C_DEEP_SPACE: Color = Color::Rgb(10, 14, 26);
const C_PANEL_BG: Color = Color::Rgb(18, 22, 38);
const C_BORDER_DIM: Color = Color::Rgb(55, 65, 100);
const C_BORDER_ACTIVE: Color = Color::Rgb(100, 130, 220);
const C_TEXT_PRIMARY: Color = Color::Rgb(220, 225, 245);
const C_TEXT_DIM: Color = Color::Rgb(100, 110, 150);
const C_SUCCESS: Color = Color::Rgb(80, 220, 130);
const C_WARNING: Color = Color::Rgb(255, 200, 50);
const C_ERROR: Color = Color::Rgb(255, 80, 80);
const C_CLOCK: Color = Color::Rgb(130, 200, 255);
const C_GAUGE_LOW: Color = Color::Rgb(80, 220, 130);
const C_GAUGE_MED: Color = Color::Rgb(255, 200, 50);
const C_GAUGE_HIGH: Color = Color::Rgb(255, 80, 80);

// ─── App State ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum FocusedPanel {
    Stats,
    Log,
    Input,
}

struct LsmStats {
    ops_per_sec: f64,
    writes_per_sec: f64,
    reads_per_sec: f64,
    memtable_usage_pct: u8,
    compaction_pct: u8,
    bloom_filter_hits: u64,
    sstable_count: u64,
    total_keys: u64,
    write_amplification: f64,
    read_amplification: f64,
    // Ring buffers for sparkline history (last N samples)
    ops_history: VecDeque<u64>,
    write_history: VecDeque<u64>,
}

impl LsmStats {
    fn new() -> Self {
        let mut ops_history = VecDeque::with_capacity(20);
        let mut write_history = VecDeque::with_capacity(20);
        // Seed with initial mock values
        for i in 0..20usize {
            let base = 800.0 + (i as f64 * 7.3).sin() * 200.0;
            ops_history.push_back(base as u64);
            write_history.push_back((base * 0.6) as u64);
        }
        Self {
            ops_per_sec: 950.0,
            writes_per_sec: 570.0,
            reads_per_sec: 380.0,
            memtable_usage_pct: 34,
            compaction_pct: 12,
            bloom_filter_hits: 14_302,
            sstable_count: 8,
            total_keys: 50_230,
            write_amplification: 1.8,
            read_amplification: 1.2,
            ops_history,
            write_history,
        }
    }

    fn tick(&mut self, elapsed_ms: u64) {
        use std::f64::consts::PI;
        let t = elapsed_ms as f64 / 1000.0;

        // Simulate fluctuating metrics
        self.ops_per_sec = 900.0 + (t * 0.7).sin() * 300.0 + (t * 1.3).cos() * 100.0;
        self.writes_per_sec = self.ops_per_sec * 0.6 + (t * 2.1).sin() * 50.0;
        self.reads_per_sec = self.ops_per_sec - self.writes_per_sec;

        self.memtable_usage_pct = {
            let v = 30.0 + (t * 0.3).sin() * 25.0 + (t * 0.9).cos() * 10.0;
            v.clamp(5.0, 95.0) as u8
        };
        self.compaction_pct = {
            let v = 10.0 + (t * 0.15 + PI).sin() * 15.0;
            v.clamp(0.0, 100.0) as u8
        };
        self.total_keys += (self.writes_per_sec * elapsed_ms as f64 / 1000.0) as u64;
        self.bloom_filter_hits +=
            (self.reads_per_sec * 0.92 * elapsed_ms as f64 / 1000.0) as u64;
        self.write_amplification = 1.5 + (t * 0.2).cos() * 0.5;
        self.read_amplification = 1.0 + (t * 0.4).sin().abs() * 0.4;

        // Push to history rings
        if self.ops_history.len() >= 20 {
            self.ops_history.pop_front();
            self.write_history.pop_front();
        }
        self.ops_history.push_back(self.ops_per_sec as u64);
        self.write_history.push_back(self.writes_per_sec as u64);
    }
}

struct App {
    focus: FocusedPanel,
    input: Input,
    log: VecDeque<(String, Color)>, // (message, color)
    stats: LsmStats,
    start: Instant,
    last_tick: Instant,
    should_quit: bool,
    mouse_pos: (u16, u16),
    total_ops: u64,
    uptime_secs: u64,
}

impl App {
    fn new() -> Self {
        let mut log = VecDeque::with_capacity(200);
        log.push_back((
            "Welcome to ApexStore TUI Dashboard".into(),
            C_APEX_AMBER,
        ));
        log.push_back(("Type 'help' for available commands.".into(), C_TEXT_DIM));
        log.push_back(("─".repeat(50), C_BORDER_DIM));
        Self {
            focus: FocusedPanel::Input,
            input: Input::default(),
            log,
            stats: LsmStats::new(),
            start: Instant::now(),
            last_tick: Instant::now(),
            should_quit: false,
            mouse_pos: (0, 0),
            total_ops: 0,
            uptime_secs: 0,
        }
    }

    fn on_tick(&mut self) {
        let elapsed_ms = self.last_tick.elapsed().as_millis() as u64;
        self.last_tick = Instant::now();
        self.uptime_secs = self.start.elapsed().as_secs();
        self.total_ops += (self.stats.ops_per_sec * elapsed_ms as f64 / 1000.0) as u64;
        self.stats.tick(self.start.elapsed().as_millis() as u64);
    }

    fn push_log(&mut self, msg: impl Into<String>, color: Color) {
        if self.log.len() >= 200 {
            self.log.pop_front();
        }
        self.log.push_back((msg.into(), color));
    }

    fn execute_command(&mut self, raw: &str) {
        let cmd = raw.trim().to_string();
        if cmd.is_empty() {
            return;
        }
        self.push_log(format!("› {}", cmd), C_TEXT_PRIMARY);

        let parts: Vec<&str> = cmd.splitn(3, ' ').collect();
        match parts[0].to_lowercase().as_str() {
            "help" => {
                self.push_log("Available commands:", C_APEX_AMBER);
                self.push_log("  get <key>            - Retrieve a value", C_TEXT_DIM);
                self.push_log("  set <key> <value>    - Store a key-value pair", C_TEXT_DIM);
                self.push_log("  del <key>            - Delete a key", C_TEXT_DIM);
                self.push_log("  stats                - Show current statistics", C_TEXT_DIM);
                self.push_log("  clear                - Clear command log", C_TEXT_DIM);
                self.push_log("  quit / q             - Exit dashboard", C_TEXT_DIM);
            }
            "get" => {
                if parts.len() < 2 {
                    self.push_log("Error: missing <key>", C_ERROR);
                } else {
                    let key = parts[1];
                    self.push_log(
                        format!("GET {} → \"mock_value_{}\"", key, &key[..key.len().min(4)]),
                        C_SUCCESS,
                    );
                    self.stats.reads_per_sec += 1.0;
                }
            }
            "set" => {
                if parts.len() < 3 {
                    self.push_log("Error: usage: set <key> <value>", C_ERROR);
                } else {
                    self.push_log(
                        format!("SET {} = {} → OK", parts[1], parts[2]),
                        C_SUCCESS,
                    );
                    self.stats.total_keys += 1;
                    self.stats.writes_per_sec += 1.0;
                }
            }
            "del" | "delete" => {
                if parts.len() < 2 {
                    self.push_log("Error: missing <key>", C_ERROR);
                } else {
                    self.push_log(format!("DEL {} → OK (tombstone written)", parts[1]), C_WARNING);
                    self.stats.writes_per_sec += 1.0;
                }
            }
            "stats" => {
                self.push_log("─── Current Statistics ───".to_string(), C_APEX_ORANGE);
                self.push_log(
                    format!("  OPS/s: {:.0}", self.stats.ops_per_sec),
                    C_TEXT_PRIMARY,
                );
                self.push_log(
                    format!("  Writes/s: {:.0}", self.stats.writes_per_sec),
                    C_TEXT_PRIMARY,
                );
                self.push_log(
                    format!("  Reads/s:  {:.0}", self.stats.reads_per_sec),
                    C_TEXT_PRIMARY,
                );
                self.push_log(
                    format!("  Total Keys: {}", self.stats.total_keys),
                    C_TEXT_PRIMARY,
                );
                self.push_log(
                    format!("  MemTable: {}%", self.stats.memtable_usage_pct),
                    C_TEXT_PRIMARY,
                );
                self.push_log(
                    format!("  SSTable files: {}", self.stats.sstable_count),
                    C_TEXT_PRIMARY,
                );
                self.push_log(
                    format!(
                        "  Write Amp: {:.2}x  Read Amp: {:.2}x",
                        self.stats.write_amplification, self.stats.read_amplification
                    ),
                    C_TEXT_PRIMARY,
                );
            }
            "clear" => {
                self.log.clear();
                self.push_log("Log cleared.".to_string(), C_TEXT_DIM);
            }
            "quit" | "q" | "exit" => {
                self.should_quit = true;
            }
            unknown => {
                self.push_log(
                    format!("Unknown command: '{}'. Type 'help' for usage.", unknown),
                    C_ERROR,
                );
            }
        }
    }
}

// ─── Terminal Setup / Teardown ──────────────────────────────────────────────────

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()
}

// ─── Main ───────────────────────────────────────────────────────────────────────

fn main() -> io::Result<()> {
    // Install panic hook so terminal is always restored on panic
    let original_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(stdout, LeaveAlternateScreen, DisableMouseCapture);
        original_hook(info);
    }));

    let mut terminal = setup_terminal()?;
    let mut app = App::new();
    let tick_rate = Duration::from_millis(250);

    loop {
        terminal.draw(|f| ui(f, &mut app))?;

        if event::poll(tick_rate)? {
            match event::read()? {
                Event::Key(key) => {
                    // Global quit shortcuts
                    if matches!(key.code, KeyCode::Char('c'))
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        app.should_quit = true;
                    } else if matches!(key.code, KeyCode::Esc) {
                        app.should_quit = true;
                    } else if app.focus == FocusedPanel::Input {
                        match key.code {
                            KeyCode::Enter => {
                                let cmd = app.input.value().to_string();
                                app.input.reset();
                                app.execute_command(&cmd);
                            }
                            KeyCode::Tab => {
                                app.focus = FocusedPanel::Log;
                            }
                            _ => {
                                app.input.handle_event(&Event::Key(key));
                            }
                        }
                    } else {
                        match key.code {
                            KeyCode::Tab | KeyCode::Enter => {
                                app.focus = FocusedPanel::Input;
                            }
                            KeyCode::Char('1') => app.focus = FocusedPanel::Stats,
                            KeyCode::Char('2') => app.focus = FocusedPanel::Log,
                            KeyCode::Char('3') => app.focus = FocusedPanel::Input,
                            _ => {}
                        }
                    }
                }
                Event::Mouse(mouse) => {
                    app.mouse_pos = (mouse.column, mouse.row);
                    if mouse.kind == MouseEventKind::Down(event::MouseButton::Left) {
                        // Clicking anywhere re-focuses input
                        app.focus = FocusedPanel::Input;
                    }
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        } else {
            // No event – run tick to update live metrics
            app.on_tick();
        }

        if app.should_quit {
            break;
        }
    }

    restore_terminal(&mut terminal)?;
    Ok(())
}

// ─── UI Rendering ───────────────────────────────────────────────────────────────

fn ui(f: &mut Frame, app: &mut App) {
    let area = f.area();

    // Fill background
    f.render_widget(
        Block::default().style(Style::default().bg(C_DEEP_SPACE)),
        area,
    );

    // ─ Outer vertical split: title bar + body + status bar
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // title
            Constraint::Min(10),   // body
            Constraint::Length(1), // status
        ])
        .split(area);

    render_title_bar(f, root[0], app);
    render_body(f, root[1], app);
    render_status_bar(f, root[2], app);
}

// ─── Title Bar ──────────────────────────────────────────────────────────────────

fn render_title_bar(f: &mut Frame, area: Rect, app: &App) {
    let now = Local::now().format("%Y-%m-%d  %H:%M:%S").to_string();
    let uptime = format_uptime(app.uptime_secs);

    let title = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(C_APEX_ORANGE))
        .style(Style::default().bg(C_PANEL_BG))
        .title(Line::from(vec![
            Span::styled(" ⚡ ", Style::default().fg(C_APEX_ORANGE).add_modifier(Modifier::BOLD)),
            Span::styled(
                "APEXSTORE",
                Style::default()
                    .fg(C_APEX_ORANGE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" TUI DASHBOARD ", Style::default().fg(C_TEXT_DIM)),
        ]))
        .title_bottom(Line::from(vec![
            Span::styled(" 🕒 ", Style::default().fg(C_CLOCK)),
            Span::styled(now, Style::default().fg(C_CLOCK).add_modifier(Modifier::BOLD)),
            Span::styled("  ⏱ up ", Style::default().fg(C_TEXT_DIM)),
            Span::styled(uptime, Style::default().fg(C_APEX_AMBER)),
            Span::styled("  ops: ", Style::default().fg(C_TEXT_DIM)),
            Span::styled(
                format!("{}", app.total_ops),
                Style::default().fg(C_SUCCESS).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", Style::default()),
        ]));

    f.render_widget(title, area);
}

// ─── Body ───────────────────────────────────────────────────────────────────────

fn render_body(f: &mut Frame, area: Rect, app: &mut App) {
    // Horizontal split: [left stats column] | [right: log + input]
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(area);

    render_left_column(f, columns[0], app);
    render_right_column(f, columns[1], app);
}

// ─── Left Column: Stats + Clock Gadget ──────────────────────────────────────────

fn render_left_column(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(area);

    render_stats_panel(f, rows[0], app);
    render_clock_gadget(f, rows[1], app);
}

fn render_stats_panel(f: &mut Frame, area: Rect, app: &App) {
    let is_focused = app.focus == FocusedPanel::Stats;
    let border_color = if is_focused { C_BORDER_ACTIVE } else { C_BORDER_DIM };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(C_PANEL_BG))
        .padding(Padding::horizontal(1))
        .title(Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled("📊", Style::default()),
            Span::styled(
                " LSM-Tree Statistics ",
                Style::default()
                    .fg(C_APEX_AMBER)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Sub-layout inside the stats panel
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // Throughput bar chart
            Constraint::Length(2), // Spacer label
            Constraint::Length(3), // MemTable gauge
            Constraint::Length(3), // Compaction gauge
            Constraint::Min(2),    // Key metrics text
        ])
        .split(inner);

    // Throughput BarChart
    let ops_data: Vec<(&str, u64)> = vec![
        ("OPS", app.stats.ops_per_sec as u64),
        ("WR", app.stats.writes_per_sec as u64),
        ("RD", app.stats.reads_per_sec as u64),
        ("BF", (app.stats.bloom_filter_hits % 1500)),
    ];

    let bar_chart = BarChart::default()
        .data(&ops_data)
        .bar_width(5)
        .bar_gap(2)
        .bar_style(Style::default().fg(C_APEX_ORANGE))
        .value_style(
            Style::default()
                .fg(C_DEEP_SPACE)
                .bg(C_APEX_ORANGE)
                .add_modifier(Modifier::BOLD),
        )
        .label_style(Style::default().fg(C_TEXT_DIM));
    f.render_widget(bar_chart, rows[0]);

    // Section header
    let section_label = Paragraph::new(Line::from(vec![
        Span::styled("  ● ", Style::default().fg(C_APEX_ORANGE)),
        Span::styled(
            "Resource Usage",
            Style::default().fg(C_TEXT_DIM).add_modifier(Modifier::ITALIC),
        ),
    ]))
    .style(Style::default().bg(C_PANEL_BG));
    f.render_widget(section_label, rows[1]);

    // MemTable Gauge
    let mem_pct = app.stats.memtable_usage_pct as f64 / 100.0;
    let mem_color = gauge_color(app.stats.memtable_usage_pct);
    let mem_gauge = Gauge::default()
        .block(
            Block::default()
                .title(Span::styled(
                    " MemTable ",
                    Style::default().fg(C_TEXT_PRIMARY).add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::LEFT | Borders::RIGHT)
                .border_style(Style::default().fg(C_BORDER_DIM)),
        )
        .gauge_style(
            Style::default()
                .fg(mem_color)
                .bg(Color::Rgb(30, 35, 55))
                .add_modifier(Modifier::BOLD),
        )
        .ratio(mem_pct)
        .label(Span::styled(
            format!(" {}%  ({:.0} MB used) ", app.stats.memtable_usage_pct, mem_pct * 64.0),
            Style::default().fg(C_TEXT_PRIMARY),
        ));
    f.render_widget(mem_gauge, rows[2]);

    // Compaction Gauge
    let cmp_pct = app.stats.compaction_pct as f64 / 100.0;
    let cmp_color = gauge_color(app.stats.compaction_pct);
    let cmp_gauge = Gauge::default()
        .block(
            Block::default()
                .title(Span::styled(
                    " Compaction ",
                    Style::default().fg(C_TEXT_PRIMARY).add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::LEFT | Borders::RIGHT)
                .border_style(Style::default().fg(C_BORDER_DIM)),
        )
        .gauge_style(
            Style::default()
                .fg(cmp_color)
                .bg(Color::Rgb(30, 35, 55))
                .add_modifier(Modifier::BOLD),
        )
        .ratio(cmp_pct)
        .label(Span::styled(
            format!(" {}%  ({} SSTables) ", app.stats.compaction_pct, app.stats.sstable_count),
            Style::default().fg(C_TEXT_PRIMARY),
        ));
    f.render_widget(cmp_gauge, rows[3]);

    // Key Metrics text grid
    let metrics = vec![
        Line::from(vec![
            Span::styled("  Keys    : ", Style::default().fg(C_TEXT_DIM)),
            Span::styled(
                format!("{:>10}", app.stats.total_keys),
                Style::default().fg(C_SUCCESS).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  BF Hits : ", Style::default().fg(C_TEXT_DIM)),
            Span::styled(
                format!("{:>10}", app.stats.bloom_filter_hits),
                Style::default().fg(C_CLOCK),
            ),
        ]),
        Line::from(vec![
            Span::styled("  W-Amp   : ", Style::default().fg(C_TEXT_DIM)),
            Span::styled(
                format!("{:>8.2}x", app.stats.write_amplification),
                Style::default().fg(C_WARNING),
            ),
            Span::styled("  R-Amp: ", Style::default().fg(C_TEXT_DIM)),
            Span::styled(
                format!("{:.2}x", app.stats.read_amplification),
                Style::default().fg(C_WARNING),
            ),
        ]),
    ];

    let metrics_widget = Paragraph::new(metrics).style(Style::default().bg(C_PANEL_BG));
    f.render_widget(metrics_widget, rows[4]);
}

fn render_clock_gadget(f: &mut Frame, area: Rect, _app: &App) {
    let now = Local::now();
    let time_str = now.format("%H:%M:%S").to_string();
    let date_str = now.format("%A, %B %d %Y").to_string();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(C_BORDER_DIM))
        .style(Style::default().bg(C_PANEL_BG))
        .title(Line::from(vec![
            Span::styled(" 🕐 ", Style::default()),
            Span::styled(
                "System Clock ",
                Style::default().fg(C_CLOCK).add_modifier(Modifier::BOLD),
            ),
        ]));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let clock_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(inner);

    let time_widget = Paragraph::new(time_str)
        .style(
            Style::default()
                .fg(C_CLOCK)
                .add_modifier(Modifier::BOLD),
        )
        .alignment(ratatui::layout::Alignment::Center);
    f.render_widget(time_widget, clock_rows[1]);

    let date_widget = Paragraph::new(date_str)
        .style(Style::default().fg(C_TEXT_DIM))
        .alignment(ratatui::layout::Alignment::Center);
    f.render_widget(date_widget, clock_rows[2]);
}

// ─── Right Column: Log + Input ──────────────────────────────────────────────────

fn render_right_column(f: &mut Frame, area: Rect, app: &mut App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(5)])
        .split(area);

    render_log_panel(f, rows[0], app);
    render_input_panel(f, rows[1], app);
}

fn render_log_panel(f: &mut Frame, area: Rect, app: &App) {
    let is_focused = app.focus == FocusedPanel::Log;
    let border_color = if is_focused { C_BORDER_ACTIVE } else { C_BORDER_DIM };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(C_PANEL_BG))
        .padding(Padding::horizontal(1))
        .title(Line::from(vec![
            Span::styled(" 📋 ", Style::default()),
            Span::styled(
                "Command Log ",
                Style::default()
                    .fg(C_TEXT_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("({} entries) ", app.log.len()),
                Style::default().fg(C_TEXT_DIM),
            ),
        ]));

    let inner_height = block.inner(area).height as usize;
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Show only the last N lines that fit
    let items: Vec<ListItem> = app
        .log
        .iter()
        .rev()
        .take(inner_height)
        .rev()
        .map(|(msg, color)| {
            ListItem::new(Line::from(Span::styled(
                msg.as_str(),
                Style::default().fg(*color),
            )))
        })
        .collect();

    let list = List::new(items).style(Style::default().bg(C_PANEL_BG));
    f.render_widget(list, inner);
}

fn render_input_panel(f: &mut Frame, area: Rect, app: &App) {
    let is_focused = app.focus == FocusedPanel::Input;
    let border_color = if is_focused { C_APEX_ORANGE } else { C_BORDER_DIM };
    let label_color = if is_focused { C_APEX_ORANGE } else { C_TEXT_DIM };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(C_PANEL_BG))
        .padding(Padding::horizontal(1))
        .title(Line::from(vec![
            Span::styled(" ⌨  ", Style::default()),
            Span::styled(
                "Command Input ",
                Style::default().fg(label_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if is_focused { "(active) " } else { "(Tab to focus) " },
                Style::default().fg(C_TEXT_DIM).add_modifier(Modifier::ITALIC),
            ),
        ]))
        .title_bottom(Line::from(Span::styled(
            " Enter: run  |  Esc: quit  |  Tab: switch panel ",
            Style::default().fg(C_TEXT_DIM),
        )));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Input rows: prompt + input box
    let input_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(inner);

    // Prompt hint
    let hint = Paragraph::new(Line::from(vec![
        Span::styled("apex", Style::default().fg(C_APEX_ORANGE).add_modifier(Modifier::BOLD)),
        Span::styled("://", Style::default().fg(C_TEXT_DIM)),
        Span::styled("db ", Style::default().fg(C_APEX_AMBER)),
        Span::styled("→", Style::default().fg(C_BORDER_ACTIVE)),
    ]))
    .style(Style::default().bg(C_PANEL_BG));
    f.render_widget(hint, input_rows[0]);

    // Input value
    let input_value = app.input.value();
    let cursor_offset = app.input.visual_cursor();

    let input_widget = Paragraph::new(Line::from(vec![
        Span::styled(
            "  ",
            Style::default().fg(C_TEXT_DIM),
        ),
        Span::styled(
            input_value,
            Style::default()
                .fg(C_TEXT_PRIMARY)
                .bg(Color::Rgb(25, 30, 50)),
        ),
        if is_focused {
            Span::styled("█", Style::default().fg(C_APEX_ORANGE))
        } else {
            Span::styled("", Style::default())
        },
    ]))
    .style(Style::default().bg(Color::Rgb(25, 30, 50)))
    .wrap(Wrap { trim: false });
    f.render_widget(input_widget, input_rows[1]);

    // Set cursor position for crossterm
    if is_focused {
        f.set_cursor_position((
            input_rows[1].x + 2 + cursor_offset as u16,
            input_rows[1].y,
        ));
    }

    let _ = cursor_offset; // suppress lint
}

// ─── Status Bar ─────────────────────────────────────────────────────────────────

fn render_status_bar(f: &mut Frame, area: Rect, app: &App) {
    let mouse = format!("mouse: ({}, {})", app.mouse_pos.0, app.mouse_pos.1);
    let focus_str = match app.focus {
        FocusedPanel::Stats => "[STATS]",
        FocusedPanel::Log => "[LOG]",
        FocusedPanel::Input => "[INPUT]",
    };
    let ops = format!(" {:.0} ops/s ", app.stats.ops_per_sec);

    let status = Paragraph::new(Line::from(vec![
        Span::styled(" ApexStore v2.1.0 ", Style::default().fg(C_APEX_ORANGE).add_modifier(Modifier::BOLD)),
        Span::styled("|", Style::default().fg(C_BORDER_DIM)),
        Span::styled(
            format!(" Focus: {} ", focus_str),
            Style::default().fg(C_BORDER_ACTIVE),
        ),
        Span::styled("|", Style::default().fg(C_BORDER_DIM)),
        Span::styled(ops, Style::default().fg(C_SUCCESS)),
        Span::styled("|", Style::default().fg(C_BORDER_DIM)),
        Span::styled(format!(" {} ", mouse), Style::default().fg(C_TEXT_DIM)),
    ]))
    .style(Style::default().bg(Color::Rgb(12, 16, 30)));
    f.render_widget(status, area);

    // Clear edge widgets if needed
    f.render_widget(Clear, Rect::new(area.right().saturating_sub(1), area.y, 1, 1));
}

// ─── Helpers ────────────────────────────────────────────────────────────────────

fn gauge_color(pct: u8) -> Color {
    if pct < 60 {
        C_GAUGE_LOW
    } else if pct < 80 {
        C_GAUGE_MED
    } else {
        C_GAUGE_HIGH
    }
}

fn format_uptime(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{}h {:02}m {:02}s", h, m, s)
    } else if m > 0 {
        format!("{}m {:02}s", m, s)
    } else {
        format!("{}s", s)
    }
}
