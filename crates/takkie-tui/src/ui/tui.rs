use std::collections::VecDeque;
use std::io;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
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
use takkie_core::PeerId;
use takkie_engine::{Engine, EngineEvent, EngineSnapshot};

const MAX_EVENTS: usize = 100;
// Without key-release events, a held key only shows up as repeats, and the
// first repeat comes after the OS repeat delay (often 500 ms).
const RELEASE_GUESS: Duration = Duration::from_millis(600);

/// The newest event lines, oldest dropped first.
#[derive(Debug, Default)]
pub struct EventLog {
    lines: VecDeque<String>,
}

impl EventLog {
    pub fn push(&mut self, text: impl AsRef<str>) {
        if self.lines.len() == MAX_EVENTS {
            self.lines.pop_front();
        }
        self.lines.push_back(format!(
            "[{}] {}",
            chrono::Local::now().format("%H:%M:%S"),
            text.as_ref()
        ));
    }

    pub fn lines(&self) -> std::collections::vec_deque::Iter<'_, String> {
        self.lines.iter()
    }
}

pub struct App {
    name: String,
    local_ip: String,
    port: u16,
    log: EventLog,
    transmitting: bool,
}

impl App {
    pub fn new(name: String, local_ip: String, port: u16) -> Self {
        Self {
            name,
            local_ip,
            port,
            log: EventLog::default(),
            transmitting: false,
        }
    }

    pub fn note(&mut self, text: impl AsRef<str>) {
        self.log.push(text);
    }
}

pub fn run(mut app: App, engine: &Engine, events: &Receiver<EngineEvent>) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let result = run_app(&mut terminal, &mut app, engine, events);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    result
}

fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    engine: &Engine,
    events: &Receiver<EngineEvent>,
) -> io::Result<()> {
    let tick_rate = Duration::from_millis(50);
    let mut last_space = Instant::now();
    let mut release_works = cfg!(windows);
    loop {
        let snapshot = engine.snapshot();
        for event in events.try_iter() {
            app.log.push(describe(&event, &snapshot));
        }
        terminal.draw(|f| ui(f, app, &snapshot))?;

        if event::poll(tick_rate)?
            && let Event::Key(key) = event::read()?
        {
            match key.code {
                KeyCode::Char('q' | 'Q') | KeyCode::Esc => return Ok(()),
                KeyCode::Char(' ') => match key.kind {
                    KeyEventKind::Press | KeyEventKind::Repeat => {
                        last_space = Instant::now();
                        set_talking(app, engine, true);
                    }
                    KeyEventKind::Release => {
                        release_works = true;
                        set_talking(app, engine, false);
                    }
                },
                _ => {}
            }
        }

        if !release_works && app.transmitting && last_space.elapsed() > RELEASE_GUESS {
            set_talking(app, engine, false);
        }
    }
}

fn set_talking(app: &mut App, engine: &Engine, on: bool) {
    if app.transmitting == on {
        return;
    }
    app.transmitting = on;
    engine.set_transmitting(on);
    app.log.push(if on {
        "🔴 PTT ACTIVE - Transmitting"
    } else {
        "⚫ PTT OFF"
    });
}

fn label(snapshot: &EngineSnapshot, id: PeerId) -> String {
    snapshot
        .peers
        .iter()
        .find(|peer| peer.id == id && !peer.name.is_empty())
        .map_or_else(|| id.to_string(), |peer| peer.name.clone())
}

/// One log line for an engine event.
pub fn describe(event: &EngineEvent, snapshot: &EngineSnapshot) -> String {
    match event {
        EngineEvent::PeerJoined(peer) => format!("✅ {} joined (ch {})", peer.name, peer.channel),
        EngineEvent::PeerUpdated(peer) => format!("✏️ {} is on ch {}", peer.name, peer.channel),
        EngineEvent::PeerLeft(id) => format!("👋 {} left", label(snapshot, *id)),
        EngineEvent::TalkStarted(id) => format!("🗣️ {} is talking", label(snapshot, *id)),
        EngineEvent::TalkStopped(id) => format!("🤐 {} stopped", label(snapshot, *id)),
        EngineEvent::DeviceStarted {
            direction,
            description,
        } => format!("🎧 {direction}: {description}"),
        EngineEvent::DeviceLost(direction) => format!("⚠️ {direction} lost, retrying"),
        EngineEvent::Warning(warning) => format!("⚠️ {warning}"),
        other => format!("{other:?}"),
    }
}

fn ui(f: &mut Frame, app: &App, snapshot: &EngineSnapshot) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(f.area());

    render_header(f, chunks[0], app);

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(chunks[1]);
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Min(5),
        ])
        .split(main_chunks[0]);

    render_connection_status(f, left_chunks[0], app, snapshot);
    render_ptt_status(f, left_chunks[1], app, snapshot);
    render_peers(f, left_chunks[2], snapshot);
    render_events(f, main_chunks[1], app);
    render_footer(f, chunks[2]);
}

fn render_header(f: &mut Frame, area: Rect, app: &App) {
    let title = Paragraph::new(format!(
        "🎵 local-takkie - {} ({}:{})",
        app.name, app.local_ip, app.port
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

fn render_connection_status(f: &mut Frame, area: Rect, app: &App, snapshot: &EngineSnapshot) {
    let peers = snapshot.peers.len();
    let row = |name: &'static str, value: String, color: Color| {
        Line::from(vec![
            Span::styled(name, Style::default().fg(Color::Gray)),
            Span::styled(value, Style::default().fg(color)),
        ])
    };
    let status_text = vec![
        Line::from(vec![
            Span::styled("Instance: ", Style::default().fg(Color::Gray)),
            Span::styled(
                app.name.as_str(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        row("Local IP: ", app.local_ip.clone(), Color::Yellow),
        row("Port: ", app.port.to_string(), Color::Yellow),
        row("Channel: ", snapshot.channel.to_string(), Color::Yellow),
        row(
            "Connected Peers: ",
            peers.to_string(),
            if peers > 0 { Color::Green } else { Color::Red },
        ),
        row(
            "Buffer: ",
            format!("{} ms", snapshot.buffer_ms),
            Color::Gray,
        ),
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

fn render_ptt_status(f: &mut Frame, area: Rect, app: &App, snapshot: &EngineSnapshot) {
    let on = app.transmitting;
    let status_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(3)])
        .split(area);

    let ptt = Paragraph::new(if on {
        "🔴 TRANSMITTING"
    } else {
        "⚫ STANDBY"
    })
    .style(
        Style::default()
            .fg(if on { Color::Red } else { Color::Gray })
            .add_modifier(Modifier::BOLD),
    )
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .title("🎤 Push-to-Talk (Hold SPACE)")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if on { Color::Red } else { Color::White })),
    );
    f.render_widget(ptt, status_chunks[0]);

    let percent = (snapshot.mic.peak.clamp(0.0, 1.0) * 100.0).round() as u16;
    let gauge = Gauge::default()
        .block(Block::default().title("🔊 Mic Level").borders(Borders::ALL))
        .gauge_style(Style::default().fg(if on { Color::Green } else { Color::Gray }))
        .percent(percent);
    f.render_widget(gauge, status_chunks[1]);
}

fn render_peers(f: &mut Frame, area: Rect, snapshot: &EngineSnapshot) {
    let items: Vec<ListItem> = snapshot
        .peers
        .iter()
        .enumerate()
        .map(|(i, peer)| {
            let name = if peer.name.is_empty() {
                peer.id.to_string()
            } else {
                peer.name.clone()
            };
            let talking = if peer.talking { " 🗣️" } else { "" };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{}. ", i + 1), Style::default().fg(Color::Gray)),
                Span::styled(
                    format!("📱 {name} (ch {}){talking}", peer.channel),
                    Style::default().fg(Color::Green),
                ),
                Span::styled(format!("  {}", peer.addr), Style::default().fg(Color::Gray)),
            ]))
        })
        .collect();
    let list = List::new(items).block(
        Block::default()
            .title(format!("👥 Connected Peers ({})", snapshot.peers.len()))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White)),
    );
    f.render_widget(list, area);
}

fn render_events(f: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app
        .log
        .lines()
        .rev()
        .take(area.height.saturating_sub(2) as usize)
        .rev()
        .map(|line| ListItem::new(line.as_str()))
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
    let footer_text = Paragraph::new("HOLD SPACEBAR to transmit | 'Q' or ESC to quit")
        .style(Style::default().fg(Color::Gray))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer_text, area);
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use takkie_core::ChannelId;
    use takkie_core::dsp::Level;
    use takkie_engine::{EngineStats, PeerInfo};

    use super::*;

    fn kitchen() -> PeerInfo {
        PeerInfo {
            id: PeerId::new(7),
            name: "Kitchen".into(),
            channel: ChannelId::try_from(2).unwrap(),
            addr: SocketAddr::from(([192, 168, 1, 5], 40_000)),
            talking: false,
        }
    }

    fn snapshot(peers: Vec<PeerInfo>) -> EngineSnapshot {
        EngineSnapshot {
            id: PeerId::new(1),
            channel: ChannelId::try_from(2).unwrap(),
            transmitting: false,
            muted: false,
            volume: 1.0,
            mic: Level::default(),
            speaker: Level::default(),
            buffer_ms: 0,
            peers,
            stats: EngineStats::default(),
        }
    }

    #[test]
    fn the_log_keeps_the_newest_lines_with_a_time() {
        let mut log = EventLog::default();
        for i in 0..150 {
            log.push(format!("event {i}"));
        }
        let lines: Vec<&String> = log.lines().collect();
        assert_eq!(lines.len(), MAX_EVENTS);
        assert!(lines[0].ends_with("] event 50"));
        assert!(lines[0].starts_with('['));
    }

    #[test]
    fn events_read_with_peer_names() {
        let known = snapshot(vec![kitchen()]);
        let id = kitchen().id;
        assert_eq!(
            describe(&EngineEvent::PeerJoined(kitchen()), &known),
            "✅ Kitchen joined (ch 2)"
        );
        assert_eq!(
            describe(&EngineEvent::TalkStarted(id), &known),
            "🗣️ Kitchen is talking"
        );
        assert_eq!(
            describe(&EngineEvent::PeerLeft(id), &known),
            "👋 Kitchen left"
        );
    }

    #[test]
    fn unknown_peers_show_their_id() {
        let id = PeerId::new(0xab);
        assert_eq!(
            describe(&EngineEvent::PeerLeft(id), &snapshot(Vec::new())),
            format!("👋 {id} left")
        );
    }
}
