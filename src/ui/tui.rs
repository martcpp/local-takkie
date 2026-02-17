use crossterm::{

    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},

    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Wrap},
};
use std::{
    io,
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub struct AppState {
    pub instance_name: String,
    pub local_ip: String,
    pub port: u16,
    pub peers: Arc<Mutex<Vec<SocketAddr>>>,
    pub buffer_size: Arc<Mutex<usize>>,
    pub ptt_active: Arc<AtomicBool>,
    pub events: Arc<Mutex<Vec<String>>>,
    pub running: Arc<AtomicBool>,
}

impl AppState {
    pub fn new(
        instance_name: String,
        local_ip: String,
        port: u16,
        peers: Arc<Mutex<Vec<SocketAddr>>>,
        buffer_size: Arc<Mutex<usize>>,
    ) -> Self {
        Self {
            instance_name,
            local_ip,
            port,
            peers,
            buffer_size,
            ptt_active: Arc::new(AtomicBool::new(false)),
            events: Arc::new(Mutex::new(Vec::new())),
            running: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn add_event(&self, event: String) {
        let mut events = self.events.lock().unwrap();
        events.push(format!(
            "[{}] {}",
            chrono::Local::now().format("%H:%M:%S"),
            event
        ));
        if events.len() > 100 {
            events.remove(0);
        }
    }
}

pub fn run_tui(state: Arc<AppState>) -> Result<(), io::Error> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    state.add_event("TUI started".to_string());

    let res = run_app(&mut terminal, state);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("Error: {:?}", err);
    }

    Ok(())
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, state: Arc<AppState>) -> io::Result<()> {
    let tick_rate = Duration::from_millis(100);
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|f| ui(f, &state))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if event::poll(timeout)?
            && let Event::Key(key) = event::read()?
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                    state.running.store(false, Ordering::Relaxed);
                    return Ok(());
                }
                KeyCode::Char('t') | KeyCode::Char('T') => {
                    // Handle push-to-talk: press activates, release deactivates
                    match key.kind {
                        KeyEventKind::Press => {
                            state.ptt_active.store(true, Ordering::Relaxed);
                            state.add_event("🔴 PTT ACTIVE - Transmitting".to_string());
                        }
                        KeyEventKind::Release => {
                            state.ptt_active.store(false, Ordering::Relaxed);
                            state.add_event("⚫ PTT OFF - Not transmitting".to_string());
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }
    }
}

fn ui(f: &mut Frame, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(10),   // Main content
            Constraint::Length(3), // Footer
        ])
        .split(f.area());

    // Header
    render_header(f, chunks[0], state);

    // Main content area
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(chunks[1]);

    // Left side: Status and Peers
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(10), // Connection status
            Constraint::Length(8),  // PTT status
            Constraint::Min(5),     // Peers
        ])
        .split(main_chunks[0]);

    render_connection_status(f, left_chunks[0], state);
    render_ptt_status(f, left_chunks[1], state);
    render_peers(f, left_chunks[2], state);

    // Right side: Events log
    render_events(f, main_chunks[1], state);

    // Footer
    render_footer(f, chunks[2]);
}

fn render_header(f: &mut Frame, area: Rect, state: &AppState) {
    let title = Paragraph::new(format!(
        "🎵 VideoLAN Audio Streamer - {} ({}:{})",
        state.instance_name, state.local_ip, state.port
    ))
    .style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(title, area);
}

fn render_connection_status(f: &mut Frame, area: Rect, state: &AppState) {
    let peers_count = state.peers.lock().unwrap().len();
    let buffer_size = *state.buffer_size.lock().unwrap();

    let status_text = vec![
        Line::from(vec![
            Span::styled("Instance: ", Style::default().fg(Color::Gray)),
            Span::styled(
                &state.instance_name,
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Local IP: ", Style::default().fg(Color::Gray)),
            Span::styled(&state.local_ip, Style::default().fg(Color::Yellow)),
        ]),
        Line::from(vec![
            Span::styled("Port: ", Style::default().fg(Color::Gray)),
            Span::styled(state.port.to_string(), Style::default().fg(Color::Yellow)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Connected Peers: ", Style::default().fg(Color::Gray)),
            Span::styled(
                peers_count.to_string(),
                Style::default().fg(if peers_count > 0 {
                    Color::Green
                } else {
                    Color::Red
                }),
            ),
        ]),
        Line::from(vec![
            Span::styled("Buffer Size: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{} samples", buffer_size),
                Style::default().fg(if buffer_size > 0 {
                    Color::Green
                } else {
                    Color::Gray
                }),
            ),
        ]),
    ];

    let paragraph = Paragraph::new(status_text)
        .block(
            Block::default()
                .title("📡 Connection Status")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::White)),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(paragraph, area);
}

fn render_ptt_status(f: &mut Frame, area: Rect, state: &AppState) {
    let ptt_active = state.ptt_active.load(Ordering::Relaxed);

    let status_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(3)])
        .split(area);

    // PTT indicator
    let ptt_text = if ptt_active {
        "🔴 TRANSMITTING"
    } else {
        "⚫ STANDBY"
    };

    let ptt_paragraph = Paragraph::new(ptt_text)
        .style(
            Style::default()
                .fg(if ptt_active { Color::Red } else { Color::Gray })
                .add_modifier(Modifier::BOLD),
        )
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .title("🎤 Push-to-Talk")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(if ptt_active {
                    Color::Red
                } else {
                    Color::White
                })),
        );
    f.render_widget(ptt_paragraph, status_chunks[0]);

    // Audio level gauge (simulated)
    let audio_level = if ptt_active { 100 } else { 0 };
    let gauge = Gauge::default()
        .block(
            Block::default()
                .title("🔊 Audio Level")
                .borders(Borders::ALL),
        )
        .gauge_style(Style::default().fg(if ptt_active {
            Color::Green
        } else {
            Color::Gray
        }))
        .percent(audio_level);
    f.render_widget(gauge, status_chunks[1]);
}

fn render_peers(f: &mut Frame, area: Rect, state: &AppState) {
    let peers = state.peers.lock().unwrap();
    let items: Vec<ListItem> = peers
        .iter()
        .enumerate()
        .map(|(i, peer)| {
            ListItem::new(Line::from(vec![
                Span::styled(format!("{}. ", i + 1), Style::default().fg(Color::Gray)),
                Span::styled(format!("📱 {}", peer), Style::default().fg(Color::Green)),
            ]))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title(format!("👥 Connected Peers ({})", peers.len()))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White)),
    );
    f.render_widget(list, area);
}

fn render_events(f: &mut Frame, area: Rect, state: &AppState) {
    let events = state.events.lock().unwrap();
    let items: Vec<ListItem> = events
        .iter()
        .rev()
        .take(area.height.saturating_sub(2) as usize)
        .rev()
        .map(|event| ListItem::new(event.as_str()))
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title("📋 Events Log")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White)),
    );
    f.render_widget(list, area);
}

fn render_footer(f: &mut Frame, area: Rect) {
    let footer_text = Paragraph::new("Press and HOLD 'T' to transmit | 'Q' or ESC to quit")
        .style(Style::default().fg(Color::Gray))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer_text, area);
}
