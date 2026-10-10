use std::collections::VecDeque;
use std::io;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::supports_keyboard_enhancement;
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Wrap},
};
use takkie_core::PeerId;
use takkie_core::ptt::{PttChange, PttController, PttInput, PttMode};
use takkie_engine::{Engine, EngineEvent, EngineSnapshot};

use crate::settings::PttMode as PttChoice;

const MAX_EVENTS: usize = 100;
// Longer than the OS key-repeat delay, or a held key flickers off before
// the first repeat arrives.
const RELEASE_GUESS: Duration = Duration::from_millis(700);

/// Whether this terminal tells us when a key is let go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyReleases {
    /// The Windows console always does.
    Native,
    /// Through the kitty keyboard protocol, switched on while we run.
    Enhanced,
    /// Neither; a held key only shows up as repeats.
    Missing,
}

impl KeyReleases {
    fn detect(windows: bool, enhancement: io::Result<bool>) -> Self {
        if windows {
            Self::Native
        } else if enhancement.unwrap_or(false) {
            Self::Enhanced
        } else {
            Self::Missing
        }
    }

    pub fn reported(self) -> bool {
        self != Self::Missing
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Native => "⌨️ Key releases: reported by Windows",
            Self::Enhanced => "⌨️ Key releases: reported (kitty keyboard protocol)",
            Self::Missing => "⌨️ Key releases: not reported by this terminal",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Key {
    Quit,
    Space(PttInput),
    SwitchMode,
}

fn key_action(code: KeyCode, kind: KeyEventKind) -> Option<Key> {
    match (code, kind) {
        (KeyCode::Char(' '), KeyEventKind::Press) => Some(Key::Space(PttInput::Press)),
        (KeyCode::Char(' '), KeyEventKind::Repeat) => Some(Key::Space(PttInput::Repeat)),
        (KeyCode::Char(' '), KeyEventKind::Release) => Some(Key::Space(PttInput::Release)),
        (_, KeyEventKind::Release) => None,
        (KeyCode::Char('q' | 'Q') | KeyCode::Esc, _) => Some(Key::Quit),
        (KeyCode::Char('t' | 'T'), _) => Some(Key::SwitchMode),
        _ => None,
    }
}

/// Hold where the terminal reports releases, toggle where it doesn't,
/// unless the user asked for one.
fn ptt_mode(choice: PttChoice, releases: KeyReleases) -> PttMode {
    match (choice, releases.reported()) {
        (PttChoice::Toggle, _) | (PttChoice::Auto, false) => PttMode::Toggle,
        (PttChoice::Auto | PttChoice::Hold, true) => PttMode::Hold,
        (PttChoice::Hold, false) => PttMode::HoldWithTimeout {
            timeout: RELEASE_GUESS,
        },
    }
}

fn other_mode(mode: PttMode, releases: KeyReleases) -> PttMode {
    match mode {
        PttMode::Toggle => ptt_mode(PttChoice::Hold, releases),
        PttMode::Hold | PttMode::HoldWithTimeout { .. } => PttMode::Toggle,
    }
}

fn mode_label(mode: PttMode) -> &'static str {
    match mode {
        PttMode::Hold => "HOLD SPACE to talk",
        PttMode::HoldWithTimeout { .. } => "HOLD SPACE to talk (release guessed)",
        PttMode::Toggle => "SPACE starts and stops talking",
    }
}

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
    choice: PttChoice,
    mode: PttMode,
    ptt: PttController,
}

impl App {
    pub fn new(name: String, local_ip: String, port: u16, choice: PttChoice) -> Self {
        Self {
            name,
            local_ip,
            port,
            log: EventLog::default(),
            transmitting: false,
            choice,
            mode: PttMode::Toggle,
            ptt: PttController::new(PttMode::Toggle),
        }
    }

    pub fn note(&mut self, text: impl AsRef<str>) {
        self.log.push(text);
    }
}

/// Takes over the terminal until the user quits. The terminal is put back
/// even if something panics.
pub fn run(mut app: App, engine: &Engine, events: &Receiver<EngineEvent>) -> io::Result<()> {
    let mut terminal = ratatui::try_init()?;
    let windows = cfg!(windows);
    let releases = KeyReleases::detect(
        windows,
        if windows {
            Ok(false)
        } else {
            supports_keyboard_enhancement()
        },
    );
    if releases == KeyReleases::Enhanced {
        execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
        )?;
    }
    tracing::info!(?releases, "key releases");
    app.note(releases.describe());
    app.mode = ptt_mode(app.choice, releases);
    app.ptt = PttController::new(app.mode);
    app.note(format!("🎤 PTT: {}", mode_label(app.mode)));
    let result = run_app(&mut terminal, &mut app, engine, events, releases);
    if releases == KeyReleases::Enhanced {
        let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
    }
    ratatui::restore();
    result
}

fn run_app(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    engine: &Engine,
    events: &Receiver<EngineEvent>,
    releases: KeyReleases,
) -> io::Result<()> {
    let tick_rate = Duration::from_millis(50);
    loop {
        let snapshot = engine.snapshot();
        for event in events.try_iter() {
            app.log.push(describe(&event, &snapshot));
        }
        terminal.draw(|f| ui(f, app, &snapshot))?;

        if event::poll(tick_rate)?
            && let Event::Key(key) = event::read()?
        {
            match key_action(key.code, key.kind) {
                Some(Key::Quit) => return Ok(()),
                Some(Key::Space(input)) => feed(app, engine, input),
                Some(Key::SwitchMode) => {
                    set_talking(app, engine, false);
                    app.mode = other_mode(app.mode, releases);
                    app.ptt = PttController::new(app.mode);
                    app.log.push(format!("🔁 PTT: {}", mode_label(app.mode)));
                }
                None => {}
            }
        }
        feed(app, engine, PttInput::Tick);
    }
}

fn feed(app: &mut App, engine: &Engine, input: PttInput) {
    match app.ptt.handle(input, Instant::now()) {
        Some(PttChange::Started) => set_talking(app, engine, true),
        Some(PttChange::Stopped) => set_talking(app, engine, false),
        None => {}
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
    render_footer(f, chunks[2], app.mode);
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
            .title(format!("🎤 Push-to-Talk ({})", mode_label(app.mode)))
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

fn render_footer(f: &mut Frame, area: Rect, mode: PttMode) {
    let footer_text = Paragraph::new(format!(
        "{} | T switch hold/toggle | Q or ESC quit",
        mode_label(mode)
    ))
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
    fn windows_always_reports_releases_and_others_need_the_protocol() {
        assert_eq!(KeyReleases::detect(true, Ok(false)), KeyReleases::Native);
        assert_eq!(KeyReleases::detect(false, Ok(true)), KeyReleases::Enhanced);
        assert_eq!(KeyReleases::detect(false, Ok(false)), KeyReleases::Missing);
        let no_answer = Err(io::Error::other("no reply"));
        assert_eq!(KeyReleases::detect(false, no_answer), KeyReleases::Missing);
        assert!(KeyReleases::Native.reported());
        assert!(!KeyReleases::Missing.reported());
    }

    #[test]
    fn only_space_cares_about_releases() {
        use KeyEventKind::{Press, Release, Repeat};
        let space = |kind| key_action(KeyCode::Char(' '), kind);
        assert_eq!(space(Press), Some(Key::Space(PttInput::Press)));
        assert_eq!(space(Repeat), Some(Key::Space(PttInput::Repeat)));
        assert_eq!(space(Release), Some(Key::Space(PttInput::Release)));
        assert_eq!(key_action(KeyCode::Char('q'), Press), Some(Key::Quit));
        assert_eq!(key_action(KeyCode::Esc, Press), Some(Key::Quit));
        assert_eq!(key_action(KeyCode::Char('T'), Press), Some(Key::SwitchMode));
        assert_eq!(key_action(KeyCode::Char('q'), Release), None);
        assert_eq!(key_action(KeyCode::Char('t'), Release), None);
        assert_eq!(key_action(KeyCode::Char('x'), Press), None);
    }

    #[test]
    fn auto_holds_where_releases_arrive_and_toggles_elsewhere() {
        use KeyReleases::{Enhanced, Missing, Native};
        let guess = PttMode::HoldWithTimeout {
            timeout: RELEASE_GUESS,
        };
        assert_eq!(ptt_mode(PttChoice::Auto, Native), PttMode::Hold);
        assert_eq!(ptt_mode(PttChoice::Auto, Enhanced), PttMode::Hold);
        assert_eq!(ptt_mode(PttChoice::Auto, Missing), PttMode::Toggle);
        assert_eq!(ptt_mode(PttChoice::Toggle, Native), PttMode::Toggle);
        assert_eq!(ptt_mode(PttChoice::Hold, Native), PttMode::Hold);
        assert_eq!(ptt_mode(PttChoice::Hold, Missing), guess);
    }

    #[test]
    fn t_swaps_between_hold_and_toggle() {
        use KeyReleases::{Missing, Native};
        assert_eq!(other_mode(PttMode::Hold, Native), PttMode::Toggle);
        assert_eq!(other_mode(PttMode::Toggle, Native), PttMode::Hold);
        assert_eq!(
            other_mode(PttMode::Toggle, Missing),
            PttMode::HoldWithTimeout {
                timeout: RELEASE_GUESS
            }
        );
        assert_eq!(
            other_mode(
                PttMode::HoldWithTimeout {
                    timeout: RELEASE_GUESS
                },
                Missing
            ),
            PttMode::Toggle
        );
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
