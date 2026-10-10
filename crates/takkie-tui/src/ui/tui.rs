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
    widgets::{Block, Borders, Clear, LineGauge, List, ListItem, Paragraph},
};
use takkie_core::ptt::{PttChange, PttController, PttInput, PttMode};
use takkie_core::{ChannelId, PeerId};
use takkie_engine::{Direction as Device, Engine, EngineEvent, EngineSnapshot, PeerInfo};

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

/// Every key the app answers to, as the help popup lists them.
const BINDINGS: [(&str, &str); 8] = [
    ("SPACE", "talk (hold or toggle, see the footer)"),
    ("T", "switch between hold and toggle"),
    ("M", "mute or unmute what you hear"),
    ("+ / -", "volume up or down"),
    ("D", "show the microphone and speaker in use"),
    ("1-9, 0", "channel 1 to 10 (not yet: use --channel)"),
    ("?", "this help"),
    ("Q / Esc", "quit"),
];

#[derive(Debug, PartialEq, Eq)]
enum Key {
    Quit,
    Space(PttInput),
    SwitchMode,
    Help,
    Mute,
    Volume(i8),
    Devices,
    Channel,
}

fn key_action(code: KeyCode, kind: KeyEventKind) -> Option<Key> {
    match (code, kind) {
        (KeyCode::Char(' '), KeyEventKind::Press) => Some(Key::Space(PttInput::Press)),
        (KeyCode::Char(' '), KeyEventKind::Repeat) => Some(Key::Space(PttInput::Repeat)),
        (KeyCode::Char(' '), KeyEventKind::Release) => Some(Key::Space(PttInput::Release)),
        (_, KeyEventKind::Release) => None,
        (KeyCode::Char('q' | 'Q') | KeyCode::Esc, _) => Some(Key::Quit),
        (KeyCode::Char('t' | 'T'), _) => Some(Key::SwitchMode),
        (KeyCode::Char('?'), _) => Some(Key::Help),
        (KeyCode::Char('m' | 'M'), _) => Some(Key::Mute),
        (KeyCode::Char('+' | '='), _) => Some(Key::Volume(1)),
        (KeyCode::Char('-' | '_'), _) => Some(Key::Volume(-1)),
        (KeyCode::Char('d' | 'D'), _) => Some(Key::Devices),
        (KeyCode::Char('0'..='9'), _) => Some(Key::Channel),
        _ => None,
    }
}

/// One 10% step up or down, kept between 0% and 200%.
fn stepped(volume: f32, steps: i8) -> f32 {
    let tenths = (volume * 10.0).round() + f32::from(steps);
    tenths.clamp(0.0, 20.0) / 10.0
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
    help: bool,
    mic: Option<String>,
    speaker: Option<String>,
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
            help: false,
            mic: None,
            speaker: None,
        }
    }

    fn remember_device(&mut self, event: &EngineEvent) {
        let (direction, state) = match event {
            EngineEvent::DeviceStarted {
                direction,
                description,
            } => (*direction, description.clone()),
            EngineEvent::DeviceLost(direction) => (*direction, "lost, retrying".to_owned()),
            _ => return,
        };
        match direction {
            Device::Input => self.mic = Some(state),
            Device::Output => self.speaker = Some(state),
        }
    }

    pub fn note(&mut self, text: impl AsRef<str>) {
        self.log.push(text);
    }
}

/// Takes over the terminal until the user quits. The terminal is put back
/// even if something panics.
pub fn run(
    mut app: App,
    engine: &Engine,
    events: &Receiver<EngineEvent>,
    panel: &Receiver<String>,
) -> io::Result<()> {
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
    let result = run_app(&mut terminal, &mut app, engine, events, panel, releases);
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
    panel: &Receiver<String>,
    releases: KeyReleases,
) -> io::Result<()> {
    let tick_rate = Duration::from_millis(50);
    loop {
        let snapshot = engine.snapshot();
        for event in events.try_iter() {
            app.remember_device(&event);
            app.log.push(describe(&event, &snapshot));
        }
        for line in panel.try_iter() {
            app.log.push(line);
        }
        terminal.draw(|f| ui(f, app, &snapshot))?;

        if event::poll(tick_rate)?
            && let Event::Key(key) = event::read()?
        {
            match key_action(key.code, key.kind) {
                Some(Key::Space(input)) => feed(app, engine, input),
                _ if app.help => {
                    if key.kind != KeyEventKind::Release {
                        app.help = false;
                    }
                }
                Some(Key::Quit) => return Ok(()),
                Some(Key::SwitchMode) => {
                    set_talking(app, engine, false);
                    app.mode = other_mode(app.mode, releases);
                    app.ptt = PttController::new(app.mode);
                    app.log.push(format!("🔁 PTT: {}", mode_label(app.mode)));
                }
                Some(Key::Help) => app.help = true,
                Some(Key::Mute) => {
                    let muted = !snapshot.muted;
                    engine.set_muted(muted);
                    app.log.push(if muted { "🔇 Muted" } else { "🔊 Unmuted" });
                }
                Some(Key::Volume(steps)) => {
                    let volume = stepped(snapshot.volume, steps);
                    engine.set_volume(volume);
                    app.log.push(format!("🔉 Volume {:.0}%", volume * 100.0));
                }
                Some(Key::Devices) => {
                    let shown = |device: &Option<String>| {
                        device.clone().unwrap_or_else(|| "not running".to_owned())
                    };
                    app.log.push(format!("🎤 Mic: {}", shown(&app.mic)));
                    app.log.push(format!("🔈 Speaker: {}", shown(&app.speaker)));
                }
                Some(Key::Channel) => app
                    .log
                    .push("🔢 Channel keys aren't active yet, start with --channel"),
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
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(f.area());
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(rows[1]);
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(3),
            Constraint::Length(5),
        ])
        .split(columns[0]);

    render_header(f, rows[0], app, snapshot);
    render_peers(f, left[0], snapshot, Instant::now());
    render_ptt_status(f, left[1], app);
    render_levels(f, left[2], snapshot);
    render_events(f, columns[1], app);
    render_footer(f, rows[2], app.mode);
    if app.help {
        render_help(f, f.area());
    }
}

fn render_help(f: &mut Frame, screen: Rect) {
    let width = 62.min(screen.width);
    let height = (BINDINGS.len() as u16 + 2).min(screen.height);
    let area = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + (screen.height - height) / 2,
        width,
        height,
    );
    let lines: Vec<Line> = BINDINGS
        .iter()
        .map(|(keys, what)| {
            Line::from(vec![
                Span::styled(
                    format!(" {keys:<9}"),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(*what),
            ])
        })
        .collect();
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title("❓ Keys (any key closes)")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        ),
        area,
    );
}

fn render_header(f: &mut Frame, area: Rect, app: &App, snapshot: &EngineSnapshot) {
    let gray = Style::default().fg(Color::Gray);
    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            "🎵 local-takkie   ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            app.name.clone(),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("   channel ", gray),
        Span::styled(
            snapshot.channel.to_string(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("   port ", gray),
        Span::styled(app.port.to_string(), Style::default().fg(Color::Yellow)),
        Span::styled(format!("   {}", app.local_ip), gray),
    ]))
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(header, area);
}

fn render_ptt_status(f: &mut Frame, area: Rect, app: &App) {
    let on = app.transmitting;

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
    f.render_widget(ptt, area);
}

/// A peak level as a meter fill, on a -60 to 0 dB scale.
fn meter(peak: f32) -> f64 {
    if peak <= 0.001 {
        return 0.0;
    }
    f64::from((20.0 * peak.log10() + 60.0) / 60.0).clamp(0.0, 1.0)
}

fn render_levels(f: &mut Frame, area: Rect, snapshot: &EngineSnapshot) {
    let block = Block::default().title("🔊 Levels").borders(Borders::ALL);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
    let gauge = |label: &'static str, peak: f32, color: Color| {
        LineGauge::default()
            .label(label)
            .ratio(meter(peak))
            .filled_symbol("█")
            .unfilled_symbol("░")
            .filled_style(Style::default().fg(color))
            .unfilled_style(Style::default().fg(Color::DarkGray))
    };
    f.render_widget(gauge("Mic     ", snapshot.mic.peak, Color::Green), rows[0]);
    f.render_widget(
        gauge("Speaker ", snapshot.speaker.peak, Color::Cyan),
        rows[1],
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Buffer  ", Style::default().fg(Color::Gray)),
            Span::raw(format!("{} ms", snapshot.buffer_ms)),
            Span::styled("    Volume  ", Style::default().fg(Color::Gray)),
            Span::raw(format!("{:.0}%", snapshot.volume * 100.0)),
            Span::styled(
                if snapshot.muted { "  MUTED" } else { "" },
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
        ])),
        rows[2],
    );
}

/// How long ago, as a peer list shows it.
fn ago(elapsed: Duration) -> String {
    match elapsed.as_secs() {
        0 => "now".to_owned(),
        seconds @ 1..=59 => format!("{seconds} s ago"),
        seconds => format!("{} min ago", seconds / 60),
    }
}

/// Our channel first, then by name, nameless ones last.
fn ordered(peers: &[PeerInfo], mine: ChannelId) -> Vec<&PeerInfo> {
    let mut peers: Vec<&PeerInfo> = peers.iter().collect();
    peers.sort_by_key(|peer| {
        (
            peer.channel != mine,
            peer.name.is_empty(),
            peer.name.to_lowercase(),
            peer.id,
        )
    });
    peers
}

fn peer_line(peer: &PeerInfo, mine: ChannelId, now: Instant) -> Line<'static> {
    let here = peer.channel == mine;
    let text = Style::default().fg(if here { Color::White } else { Color::DarkGray });
    let (dot, dot_style) = if peer.talking {
        (
            "●",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        ("○", Style::default().fg(Color::DarkGray))
    };
    let name = if peer.name.is_empty() {
        peer.id.to_string()
    } else {
        peer.name.clone()
    };
    Line::from(vec![
        Span::styled(format!("{dot} "), dot_style),
        Span::styled(format!("{name:<20}"), text),
        Span::styled(format!(" ch {:<3}", peer.channel.get()), text),
        Span::styled(
            ago(now.saturating_duration_since(peer.last_seen)),
            Style::default().fg(Color::Gray),
        ),
    ])
}

fn render_peers(f: &mut Frame, area: Rect, snapshot: &EngineSnapshot, now: Instant) {
    let here = snapshot
        .peers
        .iter()
        .filter(|peer| peer.channel == snapshot.channel)
        .count();
    let items: Vec<ListItem> = ordered(&snapshot.peers, snapshot.channel)
        .into_iter()
        .map(|peer| ListItem::new(peer_line(peer, snapshot.channel, now)))
        .collect();
    let list = List::new(items).block(
        Block::default()
            .title(format!(
                "👥 Peers ({here} on your channel, {} in all)",
                snapshot.peers.len()
            ))
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
    let footer_text = Paragraph::new(format!("{} | ? keys | Q or ESC quit", mode_label(mode)))
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
            last_seen: Instant::now(),
        }
    }

    fn peer(id: u64, name: &str, channel: u8, talking: bool) -> PeerInfo {
        PeerInfo {
            id: PeerId::new(id),
            name: name.into(),
            channel: ChannelId::try_from(channel).unwrap(),
            talking,
            ..kitchen()
        }
    }

    #[test]
    fn every_key_maps_to_its_action() {
        use KeyEventKind::{Press, Release};
        let press = |c| key_action(KeyCode::Char(c), Press);
        assert_eq!(press('?'), Some(Key::Help));
        assert_eq!(press('m'), Some(Key::Mute));
        assert_eq!(press('+'), Some(Key::Volume(1)));
        assert_eq!(press('='), Some(Key::Volume(1)));
        assert_eq!(press('-'), Some(Key::Volume(-1)));
        assert_eq!(press('D'), Some(Key::Devices));
        assert_eq!(press('1'), Some(Key::Channel));
        assert_eq!(press('0'), Some(Key::Channel));
        assert_eq!(key_action(KeyCode::Char('m'), Release), None);
    }

    #[test]
    fn volume_moves_in_ten_percent_steps_within_limits() {
        assert!((stepped(1.0, 1) - 1.1).abs() < 1e-6);
        assert!((stepped(1.0, -1) - 0.9).abs() < 1e-6);
        assert_eq!(stepped(2.0, 1), 2.0);
        assert_eq!(stepped(0.0, -1), 0.0);
        assert!((stepped(0.30000004, 1) - 0.4).abs() < 1e-6);
    }

    fn screen(app: &App, snapshot: &EngineSnapshot) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
        terminal.draw(|f| ui(f, app, snapshot)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .flat_map(|y| (0..buffer.area.width).map(move |x| (x, y)))
            .map(|at| buffer[at].symbol())
            .collect()
    }

    fn app() -> App {
        App::new(
            "Kitchen".into(),
            "192.168.1.2".into(),
            5000,
            PttChoice::Auto,
        )
    }

    #[test]
    fn the_help_popup_lists_every_binding() {
        let mut app = app();
        let quiet = snapshot(Vec::new());
        assert!(!screen(&app, &quiet).contains("any key closes"));
        app.help = true;
        let shown = screen(&app, &quiet);
        for (keys, what) in BINDINGS {
            assert!(shown.contains(keys), "help is missing {keys}");
            assert!(shown.contains(what), "help is missing: {what}");
        }
    }

    #[test]
    fn volume_and_mute_show_in_the_levels_box() {
        let mut state = snapshot(Vec::new());
        state.volume = 0.7;
        state.muted = true;
        let shown = screen(&app(), &state);
        assert!(shown.contains("Volume  70%"), "{shown}");
        assert!(shown.contains("MUTED"));
    }

    #[test]
    fn device_events_are_remembered_for_the_d_key() {
        let mut app = app();
        app.remember_device(&EngineEvent::DeviceStarted {
            direction: Device::Input,
            description: "Headset (48000 Hz)".into(),
        });
        app.remember_device(&EngineEvent::DeviceLost(Device::Output));
        assert_eq!(app.mic.as_deref(), Some("Headset (48000 Hz)"));
        assert_eq!(app.speaker.as_deref(), Some("lost, retrying"));
    }

    #[test]
    fn meters_use_a_decibel_scale() {
        assert_eq!(meter(0.0), 0.0);
        assert_eq!(meter(0.0005), 0.0);
        assert!((meter(1.0) - 1.0).abs() < 1e-6);
        assert!((meter(0.1) - 2.0 / 3.0).abs() < 1e-3);
        assert!((meter(0.01) - 1.0 / 3.0).abs() < 1e-3);
        assert_eq!(meter(4.0), 1.0);
    }

    #[test]
    fn last_heard_reads_naturally() {
        assert_eq!(ago(Duration::from_millis(400)), "now");
        assert_eq!(ago(Duration::from_secs(7)), "7 s ago");
        assert_eq!(ago(Duration::from_secs(59)), "59 s ago");
        assert_eq!(ago(Duration::from_secs(150)), "2 min ago");
    }

    #[test]
    fn our_channel_comes_first_then_names() {
        let mine = ChannelId::try_from(2).unwrap();
        let peers = [
            peer(1, "zed", 2, false),
            peer(2, "Amy", 5, false),
            peer(4, "", 2, false),
            peer(3, "bob", 2, false),
        ];
        let names: Vec<&str> = ordered(&peers, mine)
            .iter()
            .map(|peer| peer.name.as_str())
            .collect();
        assert_eq!(names, ["bob", "zed", "", "Amy"]);
    }

    #[test]
    fn a_peer_line_shows_talking_name_channel_and_last_heard() {
        let mine = ChannelId::try_from(2).unwrap();
        let now = Instant::now();
        let mut talking = peer(1, "Kitchen", 2, true);
        talking.last_seen = now - Duration::from_secs(3);
        let line = peer_line(&talking, mine, now).to_string();
        assert!(line.starts_with("● Kitchen"), "{line}");
        assert!(line.contains("ch 2"), "{line}");
        assert!(line.ends_with("3 s ago"), "{line}");
        let quiet = peer_line(&peer(9, "", 5, false), mine, now).to_string();
        assert!(
            quiet.starts_with(&format!("○ {}", PeerId::new(9))),
            "{quiet}"
        );
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
