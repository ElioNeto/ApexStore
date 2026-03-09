//! ApexStore — Interactive TUI Dashboard
//!
//! Run : `cargo run --bin apexstore-tui`
//! Quit: `q`, `quit`, `exit`, or Esc / Ctrl-C
//!
//! Commands (identical to CLI):
//!   SET <key> <value> | GET <key> | DEL <key>
//!   SEARCH <q> [--prefix] | SCAN <prefix> | ALL | KEYS | COUNT
//!   STATS [ALL] | BATCH <n> | BATCH SET <file> | DEMO | CLEAR | HELP

use apexstore::{core::engine::LsmStats, infra::config::LsmConfig, LsmEngine};
use chrono::Local;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{BarChart, Block, Borders, Clear, Gauge, List, ListItem, Padding, Paragraph, Wrap},
    Frame, Terminal,
};
use std::{
    collections::VecDeque,
    io, panic,
    path::PathBuf,
    time::{Duration, Instant},
};
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;

// ─── Palette ──────────────────────────────────────────────────────────────────
const C_ORANGE: Color = Color::Rgb(255, 110, 30);
const C_AMBER: Color = Color::Rgb(255, 185, 0);
const C_DEEP: Color = Color::Rgb(10, 14, 26);
const C_PANEL: Color = Color::Rgb(18, 22, 38);
const C_BORDER: Color = Color::Rgb(55, 65, 100);
const C_ACTIVE: Color = Color::Rgb(100, 140, 240);
const C_TEXT: Color = Color::Rgb(220, 225, 245);
const C_DIM: Color = Color::Rgb(90, 100, 140);
const C_OK: Color = Color::Rgb(80, 220, 130);
const C_WARN: Color = Color::Rgb(255, 200, 50);
const C_ERR: Color = Color::Rgb(255, 80, 80);
const C_CLOCK: Color = Color::Rgb(130, 200, 255);
const C_BAR: Color = Color::Rgb(255, 110, 30);
const C_BAR2: Color = Color::Rgb(100, 180, 255);

// ─── Focus ────────────────────────────────────────────────────────────────────
#[derive(PartialEq, Clone, Copy)]
enum Focus {
    Stats,
    Log,
    Input,
}

// ─── App ──────────────────────────────────────────────────────────────────────
struct App {
    engine: LsmEngine,
    focus: Focus,
    input: Input,
    log: VecDeque<(String, Color)>,
    stats: Option<LsmStats>,
    ops_count: u64,
    ops_last_count: u64,
    ops_last_sample: Instant,
    ops_per_sec: f64,
    ops_history: VecDeque<u64>,
    start: Instant,
    uptime: u64,
    mouse_pos: (u16, u16),
    should_quit: bool,
}

impl App {
    fn new(engine: LsmEngine) -> Self {
        let mut log = VecDeque::with_capacity(300);
        log.push_back((
            "ApexStore TUI Dashboard \u{2014} engine ready.".into(),
            C_AMBER,
        ));
        log.push_back(("Type HELP for available commands.".into(), C_DIM));
        log.push_back(("\u{2500}".repeat(54), C_BORDER));
        let mut ops_history = VecDeque::with_capacity(24);
        for _ in 0..24 {
            ops_history.push_back(0u64);
        }
        Self {
            engine,
            focus: Focus::Input,
            input: Input::default(),
            log,
            stats: None,
            ops_count: 0,
            ops_last_count: 0,
            ops_last_sample: Instant::now(),
            ops_per_sec: 0.0,
            ops_history,
            start: Instant::now(),
            uptime: 0,
            mouse_pos: (0, 0),
            should_quit: false,
        }
    }

    fn tick(&mut self) {
        self.uptime = self.start.elapsed().as_secs();
        self.stats = self.engine.stats_all().ok();

        let elapsed = self.ops_last_sample.elapsed().as_secs_f64();
        if elapsed >= 0.25 {
            let delta = self.ops_count.saturating_sub(self.ops_last_count);
            self.ops_per_sec = delta as f64 / elapsed;
            self.ops_last_count = self.ops_count;
            self.ops_last_sample = Instant::now();
            if self.ops_history.len() >= 24 {
                self.ops_history.pop_front();
            }
            self.ops_history.push_back(self.ops_per_sec as u64);
        }
    }

    fn log_push(&mut self, msg: impl Into<String>, color: Color) {
        if self.log.len() >= 300 {
            self.log.pop_front();
        }
        self.log.push_back((msg.into(), color));
    }

    fn incr_ops(&mut self) {
        self.ops_count += 1;
    }

    // ── Command dispatcher ────────────────────────────────────────────────────
    fn execute(&mut self, raw: &str) {
        let cmd = raw.trim();
        if cmd.is_empty() {
            return;
        }
        self.log_push(format!("\u{203a} {}", cmd), C_TEXT);

        let parts: Vec<&str> = cmd.splitn(3, ' ').collect();
        match parts[0].to_uppercase().as_str() {
            // SET ──────────────────────────────────────────────────────────────
            "SET" => {
                if parts.len() < 3 {
                    self.log_push("\u{274c} Usage: SET <key> <value>", C_ERR);
                    return;
                }
                let key = parts[1].to_string();
                let val = parts[2].as_bytes().to_vec();
                match self.engine.set(key.clone(), val) {
                    Ok(_) => {
                        self.log_push(format!("\u{2713} SET '{}' OK", key), C_OK);
                        self.incr_ops();
                    }
                    Err(e) => self.log_push(format!("\u{274c} {}", e), C_ERR),
                }
            }

            // GET ──────────────────────────────────────────────────────────────
            "GET" => {
                if parts.len() < 2 {
                    self.log_push("\u{274c} Usage: GET <key>", C_ERR);
                    return;
                }
                match self.engine.get(parts[1]) {
                    Ok(Some(v)) => {
                        self.log_push(
                            format!(
                                "\u{2713} '{}' = '{}'",
                                parts[1],
                                String::from_utf8_lossy(&v)
                            ),
                            C_OK,
                        );
                        self.incr_ops();
                    }
                    Ok(None) => {
                        self.log_push(format!("\u{26a0}  Key '{}' not found", parts[1]), C_WARN)
                    }
                    Err(e) => self.log_push(format!("\u{274c} {}", e), C_ERR),
                }
            }

            // DEL / DELETE ─────────────────────────────────────────────────────
            "DEL" | "DELETE" => {
                if parts.len() < 2 {
                    self.log_push("\u{274c} Usage: DEL <key>", C_ERR);
                    return;
                }
                let key = parts[1].to_string();
                match self.engine.delete(key.clone()) {
                    Ok(_) => {
                        self.log_push(format!("\u{2713} DEL '{}' (tombstone written)", key), C_OK);
                        self.incr_ops();
                    }
                    Err(e) => self.log_push(format!("\u{274c} {}", e), C_ERR),
                }
            }

            // SEARCH ───────────────────────────────────────────────────────────
            "SEARCH" => {
                if parts.len() < 2 {
                    self.log_push("\u{274c} Usage: SEARCH <query> [--prefix]", C_ERR);
                    return;
                }
                let query = parts[1];
                let prefix_mode = parts.len() > 2 && parts[2] == "--prefix";
                let result = if prefix_mode {
                    self.engine.search_prefix(query)
                } else {
                    self.engine.search(query)
                };
                match result {
                    Ok(rows) if rows.is_empty() => {
                        self.log_push("\u{26a0}  No records found", C_WARN)
                    }
                    Ok(rows) => {
                        self.log_push(format!("\u{2713} {} record(s) found:", rows.len()), C_OK);
                        for (k, v) in rows.iter().take(20) {
                            self.log_push(
                                format!("  {} = {}", k, String::from_utf8_lossy(v)),
                                C_TEXT,
                            );
                        }
                        if rows.len() > 20 {
                            self.log_push(format!("  ... and {} more", rows.len() - 20), C_DIM);
                        }
                        self.incr_ops();
                    }
                    Err(e) => self.log_push(format!("\u{274c} {}", e), C_ERR),
                }
            }

            // SCAN ─────────────────────────────────────────────────────────────
            "SCAN" => {
                if parts.len() < 2 {
                    self.log_push("\u{274c} Usage: SCAN <prefix>", C_ERR);
                    return;
                }
                match self.engine.search_prefix(parts[1]) {
                    Ok(rows) if rows.is_empty() => self.log_push(
                        format!("\u{26a0}  No records with prefix '{}'", parts[1]),
                        C_WARN,
                    ),
                    Ok(rows) => {
                        self.log_push(
                            format!("\u{2713} {} record(s) [prefix='{}']:", rows.len(), parts[1]),
                            C_OK,
                        );
                        for (k, v) in rows.iter().take(20) {
                            self.log_push(
                                format!("  {} = {}", k, String::from_utf8_lossy(v)),
                                C_TEXT,
                            );
                        }
                        if rows.len() > 20 {
                            self.log_push(format!("  ... and {} more", rows.len() - 20), C_DIM);
                        }
                        self.incr_ops();
                    }
                    Err(e) => self.log_push(format!("\u{274c} {}", e), C_ERR),
                }
            }

            // ALL ──────────────────────────────────────────────────────────────
            "ALL" => match self.engine.scan() {
                Ok(rows) if rows.is_empty() => self.log_push("\u{26a0}  Database is empty", C_WARN),
                Ok(rows) => {
                    self.log_push(format!("\u{2713} {} record(s):", rows.len()), C_OK);
                    for (k, v) in rows.iter().take(30) {
                        self.log_push(format!("  {} = {}", k, String::from_utf8_lossy(v)), C_TEXT);
                    }
                    if rows.len() > 30 {
                        self.log_push(format!("  ... and {} more", rows.len() - 30), C_DIM);
                    }
                    self.incr_ops();
                }
                Err(e) => self.log_push(format!("\u{274c} {}", e), C_ERR),
            },

            // KEYS ─────────────────────────────────────────────────────────────
            "KEYS" => match self.engine.keys() {
                Ok(keys) if keys.is_empty() => self.log_push("\u{26a0}  No keys found", C_WARN),
                Ok(keys) => {
                    self.log_push(format!("\u{2713} {} key(s):", keys.len()), C_OK);
                    for (i, k) in keys.iter().enumerate().take(30) {
                        self.log_push(format!("  {}. {}", i + 1, k), C_TEXT);
                    }
                    if keys.len() > 30 {
                        self.log_push(format!("  ... and {} more", keys.len() - 30), C_DIM);
                    }
                    self.incr_ops();
                }
                Err(e) => self.log_push(format!("\u{274c} {}", e), C_ERR),
            },

            // COUNT ────────────────────────────────────────────────────────────
            "COUNT" => match self.engine.count() {
                Ok(n) => {
                    self.log_push(format!("\u{2713} Total active records: {}", n), C_OK);
                    self.incr_ops();
                }
                Err(e) => self.log_push(format!("\u{274c} {}", e), C_ERR),
            },

            // STATS ────────────────────────────────────────────────────────────
            "STATS" => {
                let all_mode = parts.len() > 1 && parts[1].to_uppercase() == "ALL";
                if all_mode {
                    match self.engine.stats_all() {
                        Ok(s) => {
                            self.log_push("\u{2500}\u{2500}\u{2500} Detailed Statistics \u{2500}\u{2500}\u{2500}".to_string(), C_ORANGE);
                            self.log_push(
                                format!("  MemTable records : {}", s.mem_records),
                                C_TEXT,
                            );
                            self.log_push(
                                format!(
                                    "  MemTable size    : {} KB / {} KB",
                                    s.mem_kb, s.memtable_max_size
                                ),
                                C_TEXT,
                            );
                            self.log_push(format!("  SSTable files    : {}", s.sst_files), C_TEXT);
                            self.log_push(
                                format!("  SSTable records  : {}", s.sst_records),
                                C_TEXT,
                            );
                            self.log_push(format!("  SSTable size     : {} KB", s.sst_kb), C_TEXT);
                            self.log_push(format!("  WAL size         : {} KB", s.wal_kb), C_TEXT);
                            self.log_push(
                                format!("  Total records    : {}", s.total_records),
                                C_TEXT,
                            );
                        }
                        Err(e) => self.log_push(format!("\u{274c} {}", e), C_ERR),
                    }
                } else {
                    for line in self.engine.stats().lines() {
                        self.log_push(line.to_string(), C_TEXT);
                    }
                }
            }

            // BATCH ────────────────────────────────────────────────────────────
            "BATCH" => {
                if parts.len() >= 3 && parts[1].to_uppercase() == "SET" {
                    let file_path = parts[2];
                    match std::fs::read_to_string(file_path) {
                        Ok(content) => {
                            let (mut ok, mut err) = (0usize, 0usize);
                            let t = Instant::now();
                            for (line_no, line) in content.lines().enumerate() {
                                let line = line.trim();
                                if line.is_empty() || line.starts_with('#') {
                                    continue;
                                }
                                if let Some((k, v)) = line.split_once('=') {
                                    match self
                                        .engine
                                        .set(k.trim().to_string(), v.trim().as_bytes().to_vec())
                                    {
                                        Ok(_) => {
                                            ok += 1;
                                            self.incr_ops();
                                        }
                                        Err(e) => {
                                            self.log_push(
                                                format!("  line {}: {}", line_no + 1, e),
                                                C_ERR,
                                            );
                                            err += 1;
                                        }
                                    }
                                } else {
                                    self.log_push(
                                        format!(
                                            "  line {}: bad format (expected key=value)",
                                            line_no + 1
                                        ),
                                        C_WARN,
                                    );
                                    err += 1;
                                }
                            }
                            self.log_push(
                                format!(
                                    "\u{2713} {} imported, {} errors  [{:.1?}]",
                                    ok,
                                    err,
                                    t.elapsed()
                                ),
                                C_OK,
                            );
                        }
                        Err(e) => self.log_push(
                            format!("\u{274c} Cannot read '{}': {}", file_path, e),
                            C_ERR,
                        ),
                    }
                } else if parts.len() >= 2 {
                    match parts[1].parse::<usize>() {
                        Ok(n) => {
                            let t = Instant::now();
                            self.log_push(format!("Inserting {} records...", n), C_DIM);
                            let mut errs = 0usize;
                            for i in 0..n {
                                match self.engine.set(
                                    format!("batch:{:06}", i),
                                    format!("value_{}", i).into_bytes(),
                                ) {
                                    Ok(_) => {
                                        self.incr_ops();
                                    }
                                    Err(_) => {
                                        errs += 1;
                                    }
                                }
                            }
                            let elapsed = t.elapsed();
                            let rate = n as f64 / elapsed.as_secs_f64();
                            self.log_push(
                                format!(
                                    "\u{2713} {} records in {:.2?}  ({:.0} ops/s)  errors={}",
                                    n, elapsed, rate, errs
                                ),
                                C_OK,
                            );
                        }
                        Err(_) => self.log_push("\u{274c} BATCH: invalid count".to_string(), C_ERR),
                    }
                } else {
                    self.log_push("\u{274c} Usage: BATCH <n>  |  BATCH SET <file>", C_ERR);
                }
            }

            // DEMO ─────────────────────────────────────────────────────────────
            "DEMO" => {
                self.log_push(
                    "\u{2500}\u{2500}\u{2500} Running Demo \u{2500}\u{2500}\u{2500}".to_string(),
                    C_ORANGE,
                );
                let t = Instant::now();
                for i in 0..100 {
                    let _ = self.engine.set(
                        format!("demo:{:04}", i),
                        format!("demo-value-{}", i).into_bytes(),
                    );
                    self.incr_ops();
                }
                self.log_push("  100 SET ops done".to_string(), C_TEXT);
                for i in (0..100).step_by(10) {
                    let _ = self.engine.get(&format!("demo:{:04}", i));
                    self.incr_ops();
                }
                self.log_push("  10 GET ops done".to_string(), C_TEXT);
                for i in 0..10 {
                    let _ = self.engine.delete(format!("demo:{:04}", i));
                    self.incr_ops();
                }
                self.log_push("  10 DEL ops done".to_string(), C_TEXT);
                let count = self.engine.count().unwrap_or(0);
                self.log_push(
                    format!(
                        "\u{2713} Demo done in {:.2?}  active keys={}",
                        t.elapsed(),
                        count
                    ),
                    C_OK,
                );
            }

            // CLEAR ────────────────────────────────────────────────────────────
            "CLEAR" => {
                self.log.clear();
                self.log_push("Log cleared.".to_string(), C_DIM);
            }

            // HELP ─────────────────────────────────────────────────────────────
            "HELP" | "?" => {
                self.log_push("\u{2500} Available Commands \u{2500}".to_string(), C_ORANGE);
                for line in [
                    "  SET <key> <value>         insert/update",
                    "  GET <key>                 retrieve value",
                    "  DEL <key>                 delete (tombstone)",
                    "  SEARCH <q> [--prefix]     search records",
                    "  SCAN <prefix>             scan by prefix",
                    "  ALL                       list all records",
                    "  KEYS                      list all keys",
                    "  COUNT                     count active records",
                    "  STATS [ALL]               engine statistics",
                    "  BATCH <n>                 insert N test records",
                    "  BATCH SET <file>          import key=value file",
                    "  DEMO                      run quick demo",
                    "  CLEAR                     clear this log",
                    "  HELP                      show this help",
                    "  Q / QUIT / EXIT           quit dashboard",
                ] {
                    self.log_push(line.to_string(), C_DIM);
                }
            }

            // QUIT ─────────────────────────────────────────────────────────────
            "Q" | "QUIT" | "EXIT" => {
                self.should_quit = true;
            }

            unknown => {
                self.log_push(
                    format!("\u{274c} Unknown command '{}'. Type HELP.", unknown),
                    C_ERR,
                );
            }
        }
    }
}

// ─── Terminal setup / restore ─────────────────────────────────────────────────

fn setup() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    Terminal::new(CrosstermBackend::new(out))
}

fn restore(t: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(t.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    t.show_cursor()
}

// ─── Main ─────────────────────────────────────────────────────────────────────

fn main() -> io::Result<()> {
    // Panic hook: always restore terminal before printing the panic
    let original = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        original(info);
    }));

    let config = LsmConfig::builder()
        .dir_path(PathBuf::from("./.lsm_data"))
        .memtable_max_size(64 * 1024) // 64 KB
        .build()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    let engine =
        LsmEngine::new(config).map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    let mut terminal = setup()?;
    let mut app = App::new(engine);
    let tick = Duration::from_millis(250);

    loop {
        terminal.draw(|f| ui(f, &mut app))?;

        if event::poll(tick)? {
            match event::read()? {
                Event::Key(k) => {
                    if matches!(k.code, KeyCode::Char('c'))
                        && k.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        app.should_quit = true;
                    } else if matches!(k.code, KeyCode::Esc) {
                        app.should_quit = true;
                    } else if app.focus == Focus::Input {
                        match k.code {
                            KeyCode::Enter => {
                                let cmd = app.input.value().to_string();
                                app.input.reset();
                                app.execute(&cmd);
                            }
                            KeyCode::Tab => app.focus = Focus::Log,
                            _ => {
                                app.input.handle_event(&Event::Key(k));
                            }
                        }
                    } else {
                        match k.code {
                            KeyCode::Tab | KeyCode::Enter => app.focus = Focus::Input,
                            KeyCode::Char('1') => app.focus = Focus::Stats,
                            KeyCode::Char('2') => app.focus = Focus::Log,
                            KeyCode::Char('3') => app.focus = Focus::Input,
                            _ => {}
                        }
                    }
                }
                Event::Mouse(m) => {
                    app.mouse_pos = (m.column, m.row);
                    if m.kind == MouseEventKind::Down(event::MouseButton::Left) {
                        app.focus = Focus::Input;
                    }
                }
                _ => {}
            }
        } else {
            app.tick();
        }

        if app.should_quit {
            break;
        }
    }

    restore(&mut terminal)
}

// ─── UI root ──────────────────────────────────────────────────────────────────

fn ui(f: &mut Frame, app: &mut App) {
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(C_DEEP)), area);

    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(10),
        Constraint::Length(1),
    ])
    .split(area);

    render_title(f, rows[0], app);
    render_body(f, rows[1], app);
    render_statusbar(f, rows[2], app);
}

// ─── Title ────────────────────────────────────────────────────────────────────

fn render_title(f: &mut Frame, area: Rect, app: &App) {
    let now_str = Local::now().format("%Y-%m-%d  %H:%M:%S").to_string();
    let uptime_str = fmt_uptime(app.uptime);
    let total = app.stats.as_ref().map(|s| s.total_records).unwrap_or(0);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(C_ORANGE))
        .style(Style::default().bg(C_PANEL))
        .title(Line::from(vec![
            Span::styled(
                " \u{26a1} ",
                Style::default().fg(C_ORANGE).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "APEXSTORE",
                Style::default().fg(C_ORANGE).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" TUI DASHBOARD ", Style::default().fg(C_DIM)),
        ]))
        .title_bottom(Line::from(vec![
            Span::styled(" \u{1f552} ", Style::default()),
            Span::styled(
                &now_str,
                Style::default().fg(C_CLOCK).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  \u{23f1} ", Style::default().fg(C_DIM)),
            Span::styled(uptime_str, Style::default().fg(C_AMBER)),
            Span::styled("  records: ", Style::default().fg(C_DIM)),
            Span::styled(
                format!("{}", total),
                Style::default().fg(C_OK).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ops/s: ", Style::default().fg(C_DIM)),
            Span::styled(
                format!("{:.0}", app.ops_per_sec),
                Style::default().fg(C_ORANGE).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", Style::default()),
        ]));

    f.render_widget(block, area);
}

// ─── Body ─────────────────────────────────────────────────────────────────────

fn render_body(f: &mut Frame, area: Rect, app: &mut App) {
    let cols =
        Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)]).split(area);
    render_left(f, cols[0], app);
    render_right(f, cols[1], app);
}

// ─── Left: Stats + Clock ──────────────────────────────────────────────────────

fn render_left(f: &mut Frame, area: Rect, app: &App) {
    let rows =
        Layout::vertical([Constraint::Percentage(65), Constraint::Percentage(35)]).split(area);
    render_stats(f, rows[0], app);
    render_clock(f, rows[1], app);
}

fn render_stats(f: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus == Focus::Stats;
    let border_col = if focused { C_ACTIVE } else { C_BORDER };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_col))
        .style(Style::default().bg(C_PANEL))
        .padding(Padding::horizontal(1))
        .title(Line::from(vec![
            Span::styled(" \u{1f4ca} ", Style::default()),
            Span::styled(
                "LSM-Tree Statistics ",
                Style::default().fg(C_AMBER).add_modifier(Modifier::BOLD),
            ),
        ]));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::vertical([
        Constraint::Length(5), // ops/s bar chart
        Constraint::Length(1), // sst/wal text
        Constraint::Length(3), // memtable gauge
        Constraint::Length(3), // sstable gauge
        Constraint::Min(3),    // metrics text
    ])
    .split(inner);

    // Ops/s history bar chart
    let hist_data: Vec<(&str, u64)> = app
        .ops_history
        .iter()
        .enumerate()
        .map(|(i, &v)| (HIST_LABELS[i % HIST_LABELS.len()], v))
        .collect();

    f.render_widget(
        BarChart::default()
            .data(&hist_data)
            .bar_width(2)
            .bar_gap(0)
            .bar_style(Style::default().fg(C_BAR))
            .value_style(Style::default().fg(C_DEEP).bg(C_BAR))
            .label_style(Style::default().fg(Color::Reset))
            .block(
                Block::default()
                    .title(Span::styled(
                        " Ops/s (last 6s) ",
                        Style::default().fg(C_DIM).add_modifier(Modifier::ITALIC),
                    ))
                    .borders(Borders::NONE),
            ),
        rows[0],
    );

    // SST / WAL sizes
    let st = app.stats.as_ref();
    let sst_kb = st.map(|s| s.sst_kb).unwrap_or(0);
    let wal_kb = st.map(|s| s.wal_kb).unwrap_or(0);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  SST: ", Style::default().fg(C_DIM)),
            Span::styled(format!("{} KB", sst_kb), Style::default().fg(C_BAR2)),
            Span::styled("   WAL: ", Style::default().fg(C_DIM)),
            Span::styled(format!("{} KB", wal_kb), Style::default().fg(C_BAR2)),
        ]))
        .style(Style::default().bg(C_PANEL)),
        rows[1],
    );

    // MemTable gauge
    let (mem_pct, mem_kb, mem_max, mem_recs) = st.map_or((0.0, 0, 0, 0), |s| {
        let pct = if s.memtable_max_size > 0 {
            (s.mem_kb as f64 / s.memtable_max_size as f64).min(1.0)
        } else {
            0.0
        };
        (pct, s.mem_kb, s.memtable_max_size, s.mem_records)
    });
    f.render_widget(
        Gauge::default()
            .block(
                Block::default()
                    .title(Span::styled(
                        " MemTable ",
                        Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::LEFT | Borders::RIGHT)
                    .border_style(Style::default().fg(C_BORDER)),
            )
            .gauge_style(
                Style::default()
                    .fg(pct_color((mem_pct * 100.0) as u8))
                    .bg(Color::Rgb(25, 30, 50))
                    .add_modifier(Modifier::BOLD),
            )
            .ratio(mem_pct)
            .label(Span::styled(
                format!(" {} KB / {} KB  ({} records) ", mem_kb, mem_max, mem_recs),
                Style::default().fg(C_TEXT),
            )),
        rows[2],
    );

    // SSTable gauge
    let sst_files = st.map(|s| s.sst_files).unwrap_or(0);
    let sst_records = st.map(|s| s.sst_records).unwrap_or(0);
    let sst_ratio = (sst_files as f64 / 20.0_f64).min(1.0);
    f.render_widget(
        Gauge::default()
            .block(
                Block::default()
                    .title(Span::styled(
                        " SSTables ",
                        Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::LEFT | Borders::RIGHT)
                    .border_style(Style::default().fg(C_BORDER)),
            )
            .gauge_style(
                Style::default()
                    .fg(pct_color((sst_ratio * 100.0) as u8))
                    .bg(Color::Rgb(25, 30, 50))
                    .add_modifier(Modifier::BOLD),
            )
            .ratio(sst_ratio)
            .label(Span::styled(
                format!(" {} files  ({} records) ", sst_files, sst_records),
                Style::default().fg(C_TEXT),
            )),
        rows[3],
    );

    // Key metrics
    let total = st.map(|s| s.total_records).unwrap_or(0);
    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("  Total records : ", Style::default().fg(C_DIM)),
                Span::styled(
                    format!("{}", total),
                    Style::default().fg(C_OK).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Live ops/s   : ", Style::default().fg(C_DIM)),
                Span::styled(
                    format!("{:.1}", app.ops_per_sec),
                    Style::default().fg(C_ORANGE).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Cumul. ops   : ", Style::default().fg(C_DIM)),
                Span::styled(format!("{}", app.ops_count), Style::default().fg(C_BAR2)),
            ]),
        ])
        .style(Style::default().bg(C_PANEL)),
        rows[4],
    );
}

fn render_clock(f: &mut Frame, area: Rect, _app: &App) {
    let now = Local::now();
    let time_str = now.format("%H:%M:%S").to_string();
    let date_str = now.format("%A, %d %B %Y").to_string();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(C_BORDER))
        .style(Style::default().bg(C_PANEL))
        .title(Line::from(vec![
            Span::styled(" \u{1f550} ", Style::default()),
            Span::styled(
                "System Clock ",
                Style::default().fg(C_CLOCK).add_modifier(Modifier::BOLD),
            ),
        ]));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(inner);

    f.render_widget(
        Paragraph::new(time_str)
            .alignment(Alignment::Center)
            .style(Style::default().fg(C_CLOCK).add_modifier(Modifier::BOLD)),
        rows[1],
    );
    f.render_widget(
        Paragraph::new(date_str)
            .alignment(Alignment::Center)
            .style(Style::default().fg(C_DIM)),
        rows[2],
    );
}

// ─── Right: Log + Input ───────────────────────────────────────────────────────

fn render_right(f: &mut Frame, area: Rect, app: &mut App) {
    let rows = Layout::vertical([Constraint::Min(5), Constraint::Length(5)]).split(area);
    render_log(f, rows[0], app);
    render_input(f, rows[1], app);
}

fn render_log(f: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus == Focus::Log;
    let border_col = if focused { C_ACTIVE } else { C_BORDER };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_col))
        .style(Style::default().bg(C_PANEL))
        .padding(Padding::horizontal(1))
        .title(Line::from(vec![
            Span::styled(" \u{1f4cb} ", Style::default()),
            Span::styled(
                "Command Log ",
                Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("({} lines) ", app.log.len()),
                Style::default().fg(C_DIM),
            ),
        ]));

    let inner_h = block.inner(area).height as usize;
    let inner = block.inner(area);
    f.render_widget(block, area);

    let items: Vec<ListItem> = app
        .log
        .iter()
        .rev()
        .take(inner_h)
        .rev()
        .map(|(msg, col)| {
            ListItem::new(Line::from(Span::styled(
                msg.as_str(),
                Style::default().fg(*col),
            )))
        })
        .collect();

    f.render_widget(List::new(items).style(Style::default().bg(C_PANEL)), inner);
}

fn render_input(f: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus == Focus::Input;
    let border_col = if focused { C_ORANGE } else { C_BORDER };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_col))
        .style(Style::default().bg(C_PANEL))
        .padding(Padding::horizontal(1))
        .title(Line::from(vec![
            Span::styled(" \u{2328}  ", Style::default()),
            Span::styled(
                "Command Input ",
                Style::default()
                    .fg(if focused { C_ORANGE } else { C_DIM })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if focused {
                    "(active) "
                } else {
                    "(Tab to focus) "
                },
                Style::default().fg(C_DIM).add_modifier(Modifier::ITALIC),
            ),
        ]))
        .title_bottom(Line::from(Span::styled(
            " Enter: run  |  Esc: quit  |  Tab: switch panel ",
            Style::default().fg(C_DIM),
        )));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let input_rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(inner);

    // Prompt line
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "apex",
                Style::default().fg(C_ORANGE).add_modifier(Modifier::BOLD),
            ),
            Span::styled("://db ", Style::default().fg(C_DIM)),
            Span::styled("\u{2192}", Style::default().fg(C_ACTIVE)),
        ]))
        .style(Style::default().bg(C_PANEL)),
        input_rows[0],
    );

    // Input text + cursor block
    let value = app.input.value();
    let cursor = app.input.visual_cursor();
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                value,
                Style::default().fg(C_TEXT).bg(Color::Rgb(25, 30, 50)),
            ),
            if focused {
                Span::styled("\u{2588}", Style::default().fg(C_ORANGE))
            } else {
                Span::raw("")
            },
        ]))
        .wrap(Wrap { trim: false })
        .style(Style::default().bg(Color::Rgb(25, 30, 50))),
        input_rows[1],
    );

    if focused {
        f.set_cursor_position((input_rows[1].x + 2 + cursor as u16, input_rows[1].y));
    }
}

// ─── Status bar ───────────────────────────────────────────────────────────────

fn render_statusbar(f: &mut Frame, area: Rect, app: &App) {
    let focus_str = match app.focus {
        Focus::Stats => "[STATS]",
        Focus::Log => "  [LOG]",
        Focus::Input => "[INPUT]",
    };
    let bar = Paragraph::new(Line::from(vec![
        Span::styled(
            " ApexStore v2.1.0 ",
            Style::default().fg(C_ORANGE).add_modifier(Modifier::BOLD),
        ),
        Span::styled("| ", Style::default().fg(C_BORDER)),
        Span::styled(format!("{} ", focus_str), Style::default().fg(C_ACTIVE)),
        Span::styled("| ", Style::default().fg(C_BORDER)),
        Span::styled(
            format!(" {:.0} ops/s ", app.ops_per_sec),
            Style::default().fg(C_OK),
        ),
        Span::styled("| ", Style::default().fg(C_BORDER)),
        Span::styled(" data: .lsm_data ", Style::default().fg(C_DIM)),
        Span::styled("| ", Style::default().fg(C_BORDER)),
        Span::styled(
            format!(" mouse ({},{}) ", app.mouse_pos.0, app.mouse_pos.1),
            Style::default().fg(C_DIM),
        ),
    ]))
    .style(Style::default().bg(Color::Rgb(12, 16, 30)));

    f.render_widget(bar, area);
    f.render_widget(
        Clear,
        Rect::new(area.right().saturating_sub(1), area.y, 1, 1),
    );
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

const HIST_LABELS: &[&str] = &[
    "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "",
];

fn pct_color(pct: u8) -> Color {
    if pct < 60 {
        Color::Rgb(80, 220, 130)
    } else if pct < 80 {
        Color::Rgb(255, 200, 50)
    } else {
        Color::Rgb(255, 80, 80)
    }
}

fn fmt_uptime(secs: u64) -> String {
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
