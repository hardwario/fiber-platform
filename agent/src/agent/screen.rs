//! The front panel's pages and menu (`docs/front-panel.md`): what the LCD shows, drawn into a
//! [`Picture`], and what the buttons do to it. [`Facts`] is what the agent gathers for it; the
//! drawing is a pure function of the two, so the tests compare pictures.

use std::net::Ipv4Addr;

use tower_hmi::fonts::{SPLEEN_5X8, SPLEEN_6X12, SPLEEN_8X16, SPLEEN_12X24};
use tower_hmi::input::{ButtonEvent, Kind};
use tower_hmi::menu::{Item, Menu, MenuState, MenuStyle, Nav, Outcome, Table};
use tower_hmi::{Align, Canvas, Color, Image, Rect, TextStyle};

use super::buttons::{DOWN, ENTER, UP};
use super::display::{Picture, draw_message};
use super::icons;

/// The pages, in order.
pub const PAGES: [&str; 5] = ["FIBER", "Components", "Power", "Thermometers", "Radio"];
/// The radio's page.
const RADIO_PAGE: usize = 4;

/// The menu's actions.
const ABOUT: u16 = 1;
const RESTART: u16 = 2;
const PAIR: u16 = 3;

static RESTART_MENU: Menu = Menu::new(
    "Restart the FIBER?",
    &[Item::back("No"), Item::action("Yes, restart", RESTART)],
);
static MENU: Menu = Menu::new(
    "Menu",
    &[
        Item::action("Pair a node", PAIR),
        Item::action("About", ABOUT),
        Item::submenu("Restart", &RESTART_MENU),
        Item::back("Back"),
    ],
);

/// What the screen asks the agent to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    Restart,
    /// Open a pairing window (T26).
    Pair,
}

/// What the radio's page and the pairing overlays show (T26).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RadioFacts {
    /// The north bridge answers.
    pub answering: bool,
    pub channel: u8,
    pub nodes: usize,
    /// The node heard last: its name (or address), its strength, when.
    pub last: Option<(String, i16, u64)>,
    /// A pairing window is open: the seconds left.
    pub pairing: Option<u64>,
    /// A node paired, shown until a press or 60 s: its address and the code.
    pub paired: Option<(u32, u32)>,
}

/// What the pages show, as the agent gathers it once a second.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    pub name: String,
    /// The IPv4 addresses, each with the size of its network (`10.0.0.108/24`); the first is
    /// shown.
    pub addresses: Vec<String>,
    /// Unix time, and the local zone at it: its offset from UTC in seconds and its abbreviation
    /// (`UTC` and 0 when there is none).
    pub ts: u64,
    pub utc_offset: i32,
    pub zone: String,
    /// Each component that is on, and how its last apply ended.
    pub components: Vec<(String, String)>,
    /// `PoE`, `battery` or `not known`, and the two voltages when they are known; the battery's
    /// none when no cell is fitted.
    pub source: String,
    pub volts: Option<(f64, Option<f64>)>,
    /// Port, id and millidegrees of each probe.
    pub thermometers: Vec<(u8, String, Option<i32>)>,
    /// The apply that runs: its component, its step and the number of steps.
    pub apply: Option<(String, usize, usize)>,
    /// A client's message.
    pub message: Option<String>,
    /// For "About": the program, the south bridge's firmware, the TOWER revision.
    pub versions: Vec<(&'static str, String)>,
    /// The radio; none without one.
    pub radio: Option<RadioFacts>,
}

/// What the screen shows under the overlays.
pub enum View {
    Page(usize),
    Menu(MenuState),
    About,
    /// The restart was asked for; the FIBER goes down soon.
    Restarting,
}

pub struct Screen {
    pub view: View,
}

impl Default for Screen {
    fn default() -> Self {
        Screen { view: View::Page(0) }
    }
}

impl Screen {
    /// What a gesture of the buttons does, and what it asks of the agent.
    pub fn press(&mut self, event: ButtonEvent) -> Option<Ask> {
        // Up and down step as the button goes down, and again on each repeat of a hold; enter
        // acts on the click (on the release), to tell it from a hold.
        let step = matches!(event.kind, Kind::Press | Kind::Repeat);
        match &mut self.view {
            View::Page(page) => match event.buttons {
                UP if step => *page = (*page + PAGES.len() - 1) % PAGES.len(),
                DOWN if step => *page = (*page + 1) % PAGES.len(),
                ENTER if event.kind == Kind::Click => self.view = View::Menu(MenuState::new(&MENU)),
                _ => {}
            },
            View::Menu(menu) => {
                let nav = match event.buttons {
                    UP if step => Nav::Prev,
                    DOWN if step => Nav::Next,
                    ENTER if event.kind == Kind::Click => Nav::Enter,
                    ENTER if event.kind == Kind::Hold => Nav::Back,
                    _ => return None,
                };
                match menu.handle(nav, &mut Table::new([])) {
                    Some(Outcome::Exit) => self.view = View::Page(0),
                    Some(Outcome::Action(ABOUT)) => self.view = View::About,
                    Some(Outcome::Action(PAIR)) => {
                        // The window shows over the pages; the radio's page is under it.
                        self.view = View::Page(RADIO_PAGE);
                        return Some(Ask::Pair);
                    }
                    Some(Outcome::Action(RESTART)) => {
                        self.view = View::Restarting;
                        return Some(Ask::Restart);
                    }
                    _ => {}
                }
            }
            View::About if event.kind == Kind::Click => self.view = View::Menu(MenuState::new(&MENU)),
            View::About | View::Restarting => {}
        }
        None
    }
}

const SMALL: TextStyle = TextStyle::new(&SPLEEN_5X8);
const STYLE: TextStyle = TextStyle::new(&SPLEEN_6X12);

/// The characters of a line after an icon, in [`STYLE`].
const LINE: usize = 19;
const MEDIUM: TextStyle = TextStyle::new(&SPLEEN_8X16);
const BIG: TextStyle = TextStyle::new(&SPLEEN_12X24);

/// The title bar: white on black, an icon, the title, and on the right a dot for each page, the
/// one shown filled.
fn title(canvas: &mut Canvas<'_>, icon: &Image, text: &str, page: Option<usize>) {
    canvas.fill_rect(Rect::new(0, 0, 128, 12), Color::Black);
    canvas.image(2, 2, icon, Color::White, None);
    canvas.text(13, 0, text, &STYLE.inverted());
    if let Some(shown) = page {
        let first = 128 - 6 * PAGES.len() as i32;
        for i in 0..PAGES.len() {
            let x = first + 6 * i as i32;
            if i == shown {
                canvas.fill_rect(Rect::new(x, 4, 4, 4), Color::White);
            } else {
                canvas.fill_rect(Rect::new(x + 1, 5, 2, 2), Color::White);
            }
        }
    }
}

/// Lines of text below the title bar, at most four.
fn lines(canvas: &mut Canvas<'_>, lines: &[String]) {
    for (row, line) in lines.iter().take(4).enumerate() {
        let line: String = line.chars().take(21).collect();
        canvas.text(0, 14 + row as i32 * 12, &line, &STYLE);
    }
}

/// An icon and a line of text beside it, in the row that starts at `y`.
fn line_with_icon(canvas: &mut Canvas<'_>, y: i32, icon: &Image, text: &str) {
    canvas.image(1, y + 2, icon, Color::Black, None);
    let text: String = text.chars().take(LINE).collect();
    canvas.text(11, y, &text, &STYLE);
}

/// The date of a Unix time (or of a local time counted the same way): (year, month, day, weekday
/// from Sunday = 0).
fn date(ts: u64) -> (i64, usize, u64, usize) {
    let days = (ts / 86_400) as i64;
    // 1 January 1970 was a Thursday.
    let weekday = (days + 4).rem_euclid(7) as usize;
    // Days to the civil date (H. Hinnant, "chrono-Compatible Low-Level Date Algorithms").
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u64;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as usize;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day, weekday)
}

/// `Thu 19 Nov`.
fn day(ts: u64) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let (_, month, day, weekday) = date(ts);
    format!("{} {day} {}", DAYS[weekday], MONTHS[month - 1])
}

/// `HH:MM` of a Unix time (or of a local time counted the same way).
fn clock(ts: u64) -> String {
    let seconds = ts % 86_400;
    format!("{:02}:{:02}", seconds / 3600, seconds / 60 % 60)
}

/// The first page: the name, the address, and the local time in big digits with its zone and the
/// day beside it.
fn page_fiber(canvas: &mut Canvas<'_>, facts: &Facts) {
    line_with_icon(canvas, 13, &icons::NAME, &facts.name);
    let address = match facts.addresses.as_slice() {
        [] => "no address".to_owned(),
        [one] => one.clone(),
        // The address first: `+N` only when the line has room for it.
        [first, rest @ ..] => {
            let both = format!("{first} +{}", rest.len());
            if both.chars().count() <= LINE {
                both
            } else {
                first.clone()
            }
        }
    };
    line_with_icon(canvas, 25, &icons::NETWORK, &address);
    canvas.hline(0, 38, 128, Color::Black);
    let local = facts.ts.saturating_add_signed(i64::from(facts.utc_offset));
    let zone = if facts.zone.is_empty() {
        "UTC"
    } else {
        facts.zone.as_str()
    };
    canvas.text(1, 40, &clock(local), &BIG);
    canvas.text(66, 41, zone, &STYLE);
    canvas.text(66, 52, &day(local), &STYLE);
}

/// How a component's last apply ended, as its mark shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Ok,
    Failed,
    Other,
}

/// The rows of the components' page: each component's mark, name and, when it is not up, how
/// its apply ended; at most four, the last one saying how many more there are.
pub fn component_rows(facts: &Facts) -> Vec<(Mark, String, String)> {
    let mut rows: Vec<(Mark, String, String)> = facts
        .components
        .iter()
        .map(|(name, result)| {
            let mark = match result.as_str() {
                "ok" => Mark::Ok,
                "failed" => Mark::Failed,
                _ => Mark::Other,
            };
            let shown = if mark == Mark::Ok {
                String::new()
            } else {
                result.clone()
            };
            (mark, name.clone(), shown)
        })
        .collect();
    if rows.len() > 4 {
        let more = rows.len() - 3;
        rows.truncate(3);
        rows.push((Mark::Other, format!("and {more} more"), String::new()));
    }
    rows
}

fn page_components(canvas: &mut Canvas<'_>, facts: &Facts) {
    let rows = component_rows(facts);
    if rows.is_empty() {
        lines(canvas, &["none applied yet".into()]);
        return;
    }
    for (row, (mark, name, result)) in rows.iter().enumerate() {
        let y = 14 + row as i32 * 12;
        let icon = match mark {
            Mark::Ok => &icons::OK,
            Mark::Failed => &icons::FAILED,
            Mark::Other => &icons::OTHER,
        };
        canvas.image(1, y + 2, icon, Color::Black, None);
        canvas.text(11, y, name, &STYLE);
        if !result.is_empty() {
            canvas.text(127, y, result, &STYLE.align(Align::Right));
        }
    }
}

/// How full a battery at `volts` is drawn, from 3.0 V (empty) to 4.2 V (full): 0 to `width`.
pub fn battery_fill(volts: f64, width: i32) -> i32 {
    (((volts - 3.0) / 1.2).clamp(0.0, 1.0) * f64::from(width)).round() as i32
}

/// The power page: the source with its icon, the input in big digits, the battery's gauge.
fn page_power(canvas: &mut Canvas<'_>, facts: &Facts) {
    let icon = match facts.source.as_str() {
        "PoE" => &icons::PLUG,
        "Battery" => &icons::BATTERY,
        _ => &icons::UNKNOWN,
    };
    line_with_icon(canvas, 13, icon, &facts.source);
    let Some((input, battery)) = facts.volts else {
        canvas.text(0, 30, "Voltages not known", &STYLE);
        return;
    };
    canvas.text(127, 16, "input", &SMALL.align(Align::Right));
    canvas.text(1, 26, &format!("{input:5.2}"), &BIG);
    canvas.text(66, 32, "V", &MEDIUM);
    // The battery's gauge: an outline with its knob, filled to the level.
    canvas.rect(Rect::new(1, 53, 13, 8), Color::Black);
    canvas.fill_rect(Rect::new(14, 55, 2, 4), Color::Black);
    match battery {
        Some(volts) => {
            let fill = battery_fill(volts, 9);
            if fill > 0 {
                canvas.fill_rect(Rect::new(3, 55, fill, 4), Color::Black);
            }
            canvas.text(20, 51, &format!("Battery {volts:.2} V"), &STYLE);
        }
        None => {
            canvas.line(3, 59, 11, 54, Color::Black);
            canvas.text(20, 51, "No battery", &STYLE);
        }
    }
}

/// A port's number in a black badge, in the row that starts at `y`.
fn port_badge(canvas: &mut Canvas<'_>, y: i32, port: u8) {
    canvas.fill_rect(Rect::new(0, y + 1, 9, 10), Color::Black);
    canvas.text(2, y + 2, &port.to_string(), &SMALL.inverted());
}

/// The thermometers' page: one probe in big digits, more as a list, none with a hint.
fn page_thermometers(canvas: &mut Canvas<'_>, facts: &Facts) {
    let tail = |id: &str| id[id.len().saturating_sub(4)..].to_owned();
    match facts.thermometers.as_slice() {
        [] => {
            canvas.image(1, 17, &icons::THERMOMETER, Color::Black, None);
            canvas.text(11, 15, "No thermometers", &STYLE);
            canvas.text(0, 32, "Plug a DS18B20 into", &STYLE);
            canvas.text(0, 44, "any port to see it.", &STYLE);
        }
        [(port, id, millis)] => {
            port_badge(canvas, 13, *port);
            canvas.text(12, 16, &format!("port {port}, probe {}", tail(id)), &SMALL);
            match millis {
                Some(m) => {
                    let degrees = format!("{:.1}", super::thermometers::degrees(*m));
                    let width = SPLEEN_12X24.text_width(&degrees) + 4 + SPLEEN_8X16.text_width("°C");
                    let x = (128 - width) / 2;
                    canvas.text(x, 30, &degrees, &BIG);
                    canvas.text(x + SPLEEN_12X24.text_width(&degrees) + 4, 36, "°C", &MEDIUM);
                }
                None => {
                    canvas.text(64, 36, "no reading", &STYLE.align(Align::Center));
                }
            }
        }
        probes => {
            for (row, (port, id, millis)) in probes.iter().take(4).enumerate() {
                let y = 14 + row as i32 * 12;
                port_badge(canvas, y, *port);
                canvas.text(12, y + 3, &tail(id), &SMALL);
                let reading = match millis {
                    Some(m) => format!("{:.2} °C", super::thermometers::degrees(*m)),
                    None => "no reading".into(),
                };
                canvas.text(127, y, &reading, &STYLE.align(Align::Right));
            }
        }
    }
}

/// The radio's page: the radio, the nodes, the node heard last, and where pairing is.
fn page_radio(canvas: &mut Canvas<'_>, facts: &Facts) {
    let Some(radio) = &facts.radio else {
        canvas.text(1, 15, "No radio", &STYLE);
        return;
    };
    let first = if radio.answering {
        format!("TOWER, channel {}", radio.channel)
    } else {
        "North bridge silent".to_owned()
    };
    line_with_icon(canvas, 13, &icons::RADIO, &first);
    line_with_icon(
        canvas,
        25,
        &icons::COMPONENTS,
        &format!("{} of 16 nodes", radio.nodes),
    );
    let last = match &radio.last {
        Some((name, rssi, ts)) => {
            let local = ts.saturating_add_signed(i64::from(facts.utc_offset));
            let seconds = local % 86_400;
            format!("{name} {rssi} dBm {:02}:{:02}", seconds / 3600, seconds / 60 % 60)
        }
        None => "No node heard yet".to_owned(),
    };
    canvas.text(1, 39, &last.chars().take(21).collect::<String>(), &STYLE);
    canvas.text(1, 52, "Pair: Menu", &SMALL);
}

/// A pairing window that is open: the seconds left.
fn draw_pairing(left: u64) -> Picture {
    let mut picture = Picture::new();
    let mut canvas = picture.canvas();
    title(&mut canvas, &icons::RADIO, "Pairing", None);
    canvas.text(1, 15, &format!("{left} s"), &BIG);
    canvas.text(1, 41, "Put the node into", &STYLE);
    canvas.text(1, 52, "pairing mode.", &STYLE);
    picture
}

/// A node that paired: its address and the code to compare, in big digits.
fn draw_paired(node: u32, sas: u32) -> Picture {
    let mut picture = Picture::new();
    let mut canvas = picture.canvas();
    title(&mut canvas, &icons::RADIO, "Paired", None);
    canvas.text(1, 14, &format!("0x{node:08x}"), &STYLE);
    canvas.text(1, 27, &format!("{sas:06}"), &BIG);
    canvas.text(1, 52, "Same code on the node?", &SMALL);
    picture
}

/// The picture: an apply that runs, else a node that paired, a pairing window, a client's message,
/// else the view.
pub fn draw(screen: &mut Screen, facts: &Facts) -> Picture {
    if let Some((component, step, steps)) = &facts.apply {
        let mut picture = Picture::new();
        let mut canvas = picture.canvas();
        title(&mut canvas, &icons::GEAR, "Applying", None);
        canvas.text(1, 16, component, &MEDIUM);
        // The progress bar: the steps done of all of them.
        canvas.rect(Rect::new(0, 36, 128, 10), Color::Black);
        let done = (*step).saturating_sub(1).min(*steps);
        if *steps > 0 && done > 0 {
            let width = (124 * done / *steps) as i32;
            canvas.fill_rect(Rect::new(2, 38, width, 6), Color::Black);
        }
        canvas.text(1, 50, &format!("step {step} of {steps}"), &STYLE);
        return picture;
    }
    if let Some(radio) = &facts.radio {
        if let Some((node, sas)) = radio.paired {
            return draw_paired(node, sas);
        }
        if let Some(left) = radio.pairing {
            return draw_pairing(left);
        }
    }
    if let Some(message) = &facts.message {
        return draw_message(message);
    }
    let mut picture = Picture::new();
    let mut canvas = picture.canvas();
    match &mut screen.view {
        View::Page(page) => {
            let page = (*page).min(PAGES.len() - 1);
            let icon = [
                &icons::FIBER,
                &icons::COMPONENTS,
                &icons::POWER,
                &icons::THERMOMETER,
                &icons::RADIO,
            ][page];
            title(&mut canvas, icon, PAGES[page], Some(page));
            match page {
                0 => page_fiber(&mut canvas, facts),
                1 => page_components(&mut canvas, facts),
                2 => page_power(&mut canvas, facts),
                3 => page_thermometers(&mut canvas, facts),
                _ => page_radio(&mut canvas, facts),
            }
        }
        View::Menu(menu) => menu.render(&mut canvas, &Table::new([]), &MenuStyle::new(&SPLEEN_6X12)),
        View::Restarting => {
            title(&mut canvas, &icons::RESTART, "Restarting", None);
            lines(
                &mut canvas,
                &["The FIBER restarts.".into(), "Back in a minute.".into()],
            );
        }
        View::About => {
            title(&mut canvas, &icons::INFO, "About", None);
            let about: Vec<String> = facts
                .versions
                .iter()
                .map(|(what, version)| format!("{what:<7}{version}"))
                .collect();
            lines(&mut canvas, &about);
        }
    }
    picture
}

/// The local IPv4 addresses in `/proc/net/fib_trie`, without the loopback, each with the size of
/// its network: `a.b.c.d/n`. An address is a leaf `|-- a.b.c.d` with a `/32 host LOCAL` line under
/// it; its network, the longest leaf with a `/n link UNICAST` line that holds it (the route the
/// kernel made for the address). An address in no such network is shown bare.
pub fn local_addresses(fib_trie: &str) -> Vec<String> {
    let mut locals: Vec<Ipv4Addr> = Vec::new();
    let mut networks: Vec<(u32, u32)> = Vec::new();
    let mut leaf = None;
    for line in fib_trie.lines() {
        let line = line.trim();
        if let Some(address) = line.strip_prefix("|-- ") {
            leaf = address.parse::<Ipv4Addr>().ok();
        } else if line.starts_with("+--") {
            leaf = None;
        } else if let (Some(address), Some(rest)) = (leaf, line.strip_prefix('/')) {
            let mut words = rest.split_whitespace();
            let (Some(Ok(bits)), Some(scope), Some(kind)) =
                (words.next().map(str::parse::<u32>), words.next(), words.next())
            else {
                continue;
            };
            match (bits, scope, kind) {
                (32, "host", "LOCAL") if !address.is_loopback() && !locals.contains(&address) => {
                    locals.push(address)
                }
                (1..=31, "link", "UNICAST") => networks.push((u32::from(address), bits)),
                _ => {}
            }
        }
    }
    locals
        .into_iter()
        .map(|address| {
            let bits = networks
                .iter()
                .filter(|(network, bits)| {
                    let mask = u32::MAX << (32 - bits);
                    u32::from(address) & mask == *network
                })
                .map(|(_, bits)| *bits)
                .max();
            match bits {
                Some(bits) => format!("{address}/{bits}"),
                None => address.to_string(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::display::as_text;

    fn facts() -> Facts {
        Facts {
            name: "fiber-abcdef12".into(),
            addresses: vec!["192.168.1.23/24".into()],
            ts: 1_794_952_515,
            utc_offset: 3600,
            zone: "CET".into(),
            components: vec![
                ("base".into(), "ok".into()),
                ("mosquitto".into(), "ok".into()),
                ("agent".into(), "ok".into()),
                ("node-red".into(), "failed".into()),
            ],
            source: "PoE".into(),
            volts: Some((12.08, Some(4.11))),
            thermometers: vec![
                (3, "28-0000034a5b1c".into(), Some(21437)),
                (5, "28-00000b1c2d3e".into(), None),
            ],
            apply: None,
            message: None,
            versions: vec![
                ("fiber", "0.1.0".into()),
                ("south", "0.1.0".into()),
                ("tower", "cb3b25c".into()),
            ],
            radio: Some(RadioFacts {
                answering: true,
                channel: 0,
                nodes: 2,
                last: Some(("climate:0".into(), -71, 1_794_952_480)),
                pairing: None,
                paired: None,
            }),
        }
    }

    /// Compares with `src/agent/pictures/<name>.txt`, as `display`'s tests do.
    fn same_as_stored(name: &str, picture: &Picture) {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("src/agent/pictures/{name}.txt"));
        let text = as_text(picture);
        if std::env::var_os("FIBER_WRITE_PICTURES").is_some() {
            std::fs::write(&path, &text).unwrap();
        }
        let stored = std::fs::read_to_string(&path).unwrap();
        assert!(stored == text, "{name} differs from {}:\n{text}", path.display());
    }

    fn click(buttons: u8) -> ButtonEvent {
        ButtonEvent {
            buttons,
            kind: Kind::Click,
        }
    }

    /// A button going down: up and down step on it.
    fn down(buttons: u8) -> ButtonEvent {
        ButtonEvent {
            buttons,
            kind: Kind::Press,
        }
    }

    #[test]
    fn every_page() {
        let mut screen = Screen::default();
        for (n, name) in [
            "page-fiber",
            "page-components",
            "page-power",
            "page-thermometers",
            "page-radio",
        ]
        .into_iter()
        .enumerate()
        {
            screen.view = View::Page(n);
            same_as_stored(name, &draw(&mut screen, &facts()));
        }
    }

    #[test]
    fn what_covers_the_pages() {
        let mut screen = Screen::default();
        let mut facts = facts();
        facts.message = Some("Back in\n5 minutes".into());
        same_as_stored("message", &draw(&mut screen, &facts));
        // A pairing window, and a node that paired, over a message.
        let radio = facts.radio.clone().unwrap();
        facts.radio = Some(RadioFacts {
            pairing: Some(42),
            ..radio.clone()
        });
        same_as_stored("pairing", &draw(&mut screen, &facts));
        facts.radio = Some(RadioFacts {
            paired: Some((0x4444_4444, 4321)),
            ..radio.clone()
        });
        same_as_stored("paired", &draw(&mut screen, &facts));
        facts.apply = Some(("node-red".into(), 4, 6));
        same_as_stored("applying", &draw(&mut screen, &facts));
        // The radio's page without the north bridge, and without a radio.
        facts.apply = None;
        facts.message = None;
        screen.view = View::Page(RADIO_PAGE);
        facts.radio = Some(RadioFacts {
            answering: false,
            last: None,
            ..radio
        });
        same_as_stored("page-radio-silent", &draw(&mut screen, &facts));
        facts.radio = None;
        same_as_stored("page-no-radio", &draw(&mut screen, &facts));
    }

    #[test]
    fn the_buttons() {
        let mut screen = Screen::default();
        screen.press(down(UP));
        assert!(matches!(screen.view, View::Page(4)));
        // The click that follows on the release does not step again.
        screen.press(click(UP));
        assert!(matches!(screen.view, View::Page(4)));
        screen.press(down(DOWN));
        screen.press(down(DOWN));
        assert!(matches!(screen.view, View::Page(1)));
        // A hold alone does not turn a page; its repeats do.
        screen.press(ButtonEvent {
            buttons: DOWN,
            kind: Kind::Hold,
        });
        assert!(matches!(screen.view, View::Page(1)));
        screen.press(ButtonEvent {
            buttons: DOWN,
            kind: Kind::Repeat,
        });
        assert!(matches!(screen.view, View::Page(2)));

        // The menu: pairing a node first; the radio's page is under the window.
        screen.press(click(ENTER));
        same_as_stored("menu", &draw(&mut screen, &facts()));
        assert_eq!(screen.press(click(ENTER)), Some(Ask::Pair));
        assert!(matches!(screen.view, View::Page(RADIO_PAGE)));
        // About, and back to the pages.
        screen.press(click(ENTER));
        screen.press(down(DOWN));
        screen.press(click(ENTER));
        assert!(matches!(screen.view, View::About));
        same_as_stored("about", &draw(&mut screen, &facts()));
        screen.press(click(UP));
        assert!(matches!(screen.view, View::Menu(_)));
        screen.press(down(DOWN));
        screen.press(down(DOWN));
        screen.press(down(DOWN));
        assert_eq!(screen.press(click(ENTER)), None);
        assert!(matches!(screen.view, View::Page(0)));

        // Restart asks first: "No" is the first choice, "Yes, restart" the second.
        screen.press(click(ENTER));
        screen.press(down(DOWN));
        screen.press(down(DOWN));
        assert_eq!(screen.press(click(ENTER)), None);
        same_as_stored("menu-restart", &draw(&mut screen, &facts()));
        assert_eq!(screen.press(click(ENTER)), None);
        assert!(matches!(screen.view, View::Menu(_)));
        screen.press(click(ENTER));
        screen.press(down(DOWN));
        assert_eq!(screen.press(click(ENTER)), Some(Ask::Restart));
        assert!(matches!(screen.view, View::Restarting));
        same_as_stored("restarting", &draw(&mut screen, &facts()));
        // Nothing moves it any more.
        assert_eq!(screen.press(click(ENTER)), None);
        assert!(matches!(screen.view, View::Restarting));
    }

    #[test]
    fn many_components_and_long_names() {
        let mut facts = facts();
        facts.components.push(("chirpstack".into(), "skipped".into()));
        let rows = component_rows(&facts);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0], (Mark::Ok, "base".into(), String::new()));
        assert_eq!(rows[3], (Mark::Other, "and 2 more".into(), String::new()));
        facts.components.truncate(4);
        assert_eq!(
            component_rows(&facts)[3],
            (Mark::Failed, "node-red".into(), "failed".into())
        );
    }

    #[test]
    fn dates_and_the_battery_gauge() {
        assert_eq!(date(0), (1970, 1, 1, 4));
        // 2026-11-19 07:55:15 UTC, a Thursday; 29 February 2024.
        assert_eq!(date(1_795_074_915), (2026, 11, 19, 4));
        assert_eq!(day(1_795_074_915), "Thu 19 Nov");
        assert_eq!(date(1_709_208_000), (2024, 2, 29, 4));
        assert_eq!(battery_fill(4.2, 9), 9);
        assert_eq!(battery_fill(3.6, 9), 5);
        assert_eq!(battery_fill(2.5, 9), 0);
    }

    #[test]
    fn pages_with_little_on_them() {
        let mut screen = Screen::default();
        let mut facts = facts();
        facts.thermometers.truncate(1);
        facts.volts = Some((13.62, None));
        screen.view = View::Page(2);
        same_as_stored("page-power-no-battery", &draw(&mut screen, &facts));
        screen.view = View::Page(3);
        same_as_stored("page-thermometer", &draw(&mut screen, &facts));
        facts.thermometers.clear();
        same_as_stored("page-no-thermometers", &draw(&mut screen, &facts));
    }

    #[test]
    fn addresses_from_the_fib_trie() {
        let fib_trie = "Main:\n  +-- 0.0.0.0/0 3 0 5\n     |-- 0.0.0.0\n        /0 universe UNICAST\n\
                        \x20    +-- 127.0.0.0/8 2 0 2\n        |-- 127.0.0.1\n           /32 host LOCAL\n\
                        \x20    |-- 192.168.1.23\n        /32 host LOCAL\n     |-- 192.168.1.255\n\
                        \x20       /32 link BROADCAST\nLocal:\n     |-- 192.168.1.23\n        /32 host LOCAL\n\
                        \x20    |-- 10.0.0.5\n        /32 host LOCAL\n";
        // No `link UNICAST` route in this one: the addresses stay bare.
        assert_eq!(local_addresses(fib_trie), ["192.168.1.23", "10.0.0.5"]);
        assert!(local_addresses("").is_empty());
    }

    /// The bench's FIBER on Wi-Fi (2026-11-19), and a second network, on a cable, of another size.
    #[test]
    fn addresses_with_their_network_size() {
        let fiber = include_str!("fib-trie-2026-11-19.txt");
        assert_eq!(local_addresses(fiber), ["10.0.0.108/24"]);
        let two = "Main:\n  +-- 0.0.0.0/0 3 0 5\n     |-- 0.0.0.0\n        /0 universe UNICAST\n\
                   \x20    +-- 172.16.0.0/20 2 0 2\n        |-- 172.16.0.0\n           /20 link UNICAST\n\
                   \x20       |-- 172.16.3.7\n           /32 host LOCAL\n\
                   \x20    +-- 192.168.100.0/24 2 0 2\n        |-- 192.168.100.0\n           /24 link UNICAST\n\
                   \x20       |-- 192.168.100.200\n           /32 host LOCAL\n\
                   \x20       |-- 192.168.100.255\n           /32 link BROADCAST\n";
        assert_eq!(local_addresses(two), ["172.16.3.7/20", "192.168.100.200/24"]);
    }

    #[test]
    fn the_address_line_keeps_the_address() {
        let mut screen = Screen::default();
        let mut facts = facts();
        // `+1` fits after a short address, not after a long one.
        facts.addresses = vec!["10.0.0.108/24".into(), "172.16.3.7/20".into()];
        let short = as_text(&draw(&mut screen, &facts));
        facts.addresses = vec!["192.168.100.200/24".into(), "172.16.3.7/20".into()];
        let long = as_text(&draw(&mut screen, &facts));
        facts.addresses = vec!["192.168.100.200/24".into()];
        assert_eq!(long, as_text(&draw(&mut screen, &facts)));
        assert_ne!(short, long);
    }
}
