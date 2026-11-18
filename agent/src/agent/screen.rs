//! The front panel's pages and menu (`docs/front-panel.md`): what the LCD shows, drawn into a
//! [`Picture`], and what the buttons do to it. [`Facts`] is what the agent gathers for it; the
//! drawing is a pure function of the two, so the tests compare pictures.

use tower_hmi::fonts::SPLEEN_6X12;
use tower_hmi::input::{ButtonEvent, Kind};
use tower_hmi::menu::{Item, Menu, MenuState, MenuStyle, Nav, Outcome, Table};
use tower_hmi::{Align, Canvas, Color, Rect, TextStyle};

use super::buttons::{DOWN, ENTER, UP};
use super::display::{Picture, draw_message};

/// The pages, in order.
pub const PAGES: [&str; 4] = ["FIBER", "Components", "Power", "Thermometers"];

/// The menu's actions.
const ABOUT: u16 = 1;

static MENU: Menu = Menu::new("Menu", &[Item::action("About", ABOUT), Item::back("Back")]);

/// What the pages show, as the agent gathers it once a second.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    pub name: String,
    /// IPv4 addresses, the first two shown.
    pub addresses: Vec<String>,
    /// Unix time.
    pub ts: u64,
    /// Each component that is on, and how its last apply ended.
    pub components: Vec<(String, String)>,
    /// `PoE`, `battery` or `not known`, and the two voltages when they are known.
    pub source: String,
    pub volts: Option<(f64, f64)>,
    /// Port, id and millidegrees of each probe.
    pub thermometers: Vec<(u8, String, Option<i32>)>,
    /// The apply that runs: its component, its step and the number of steps.
    pub apply: Option<(String, usize, usize)>,
    /// A client's message.
    pub message: Option<String>,
    /// For "About": the program, the south bridge's firmware, the TOWER revision.
    pub versions: Vec<(&'static str, String)>,
}

/// What the screen shows under the overlays.
pub enum View {
    Page(usize),
    Menu(MenuState),
    About,
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
    /// What a gesture of the buttons does.
    pub fn press(&mut self, event: ButtonEvent) {
        let step = matches!(event.kind, Kind::Click | Kind::Repeat);
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
                    _ => return,
                };
                match menu.handle(nav, &mut Table::new([])) {
                    Some(Outcome::Exit) => self.view = View::Page(0),
                    Some(Outcome::Action(ABOUT)) => self.view = View::About,
                    _ => {}
                }
            }
            View::About if event.kind == Kind::Click => self.view = View::Menu(MenuState::new(&MENU)),
            View::About => {}
        }
    }
}

const STYLE: TextStyle = TextStyle::new(&SPLEEN_6X12);

/// The title bar: white on black, the page's number on the right.
fn title(canvas: &mut Canvas<'_>, text: &str, number: Option<usize>) {
    canvas.fill_rect(Rect::new(0, 0, 128, 12), Color::Black);
    canvas.text(1, 0, text, &STYLE.inverted());
    if let Some(n) = number {
        let n = format!("{}/{}", n + 1, PAGES.len());
        canvas.text(127, 0, &n, &STYLE.inverted().align(Align::Right));
    }
}

/// The four lines below the title bar.
fn lines(canvas: &mut Canvas<'_>, lines: &[String]) {
    for (row, line) in lines.iter().take(4).enumerate() {
        let line: String = line.chars().take(21).collect();
        canvas.text(0, 14 + row as i32 * 12, &line, &STYLE);
    }
}

/// `HH:MM UTC`.
fn clock(ts: u64) -> String {
    let seconds = ts % 86_400;
    format!("{:02}:{:02} UTC", seconds / 3600, seconds / 60 % 60)
}

/// The text of a page.
pub fn page_lines(page: usize, facts: &Facts) -> Vec<String> {
    match page {
        0 => {
            let mut out = vec![facts.name.clone()];
            out.extend(facts.addresses.iter().take(2).cloned());
            if facts.addresses.is_empty() {
                out.push("no address".into());
            }
            out.push(clock(facts.ts));
            out
        }
        1 if facts.components.is_empty() => vec!["none applied yet".into()],
        1 => {
            let mut out: Vec<String> = facts
                .components
                .iter()
                .take(4)
                .map(|(name, result)| format!("{name:<14}{result}"))
                .collect();
            if facts.components.len() > 4 {
                out[3] = format!("and {} more", facts.components.len() - 3);
            }
            out
        }
        2 => {
            let mut out = vec![facts.source.clone()];
            if let Some((input, battery)) = facts.volts {
                out.push(format!("Input   {input:5.2} V"));
                out.push(format!("Battery {battery:5.2} V"));
            }
            out
        }
        _ if facts.thermometers.is_empty() => vec!["no thermometers".into()],
        _ => facts
            .thermometers
            .iter()
            .take(4)
            .map(|(port, id, millis)| {
                let end = &id[id.len().saturating_sub(4)..];
                match millis {
                    Some(m) => format!("{port} {end} {:7.2} °C", super::thermometers::degrees(*m)),
                    None => format!("{port} {end}  no reading"),
                }
            })
            .collect(),
    }
}

/// The picture: an apply that runs, else a client's message, else the view.
pub fn draw(screen: &mut Screen, facts: &Facts) -> Picture {
    if let Some((component, step, steps)) = &facts.apply {
        let mut picture = Picture::new();
        let mut canvas = picture.canvas();
        title(&mut canvas, "Applying", None);
        lines(
            &mut canvas,
            &[component.clone(), format!("step {step} of {steps}")],
        );
        return picture;
    }
    if let Some(message) = &facts.message {
        return draw_message(message);
    }
    let mut picture = Picture::new();
    let mut canvas = picture.canvas();
    match &mut screen.view {
        View::Page(page) => {
            let page = (*page).min(PAGES.len() - 1);
            title(&mut canvas, PAGES[page], Some(page));
            lines(&mut canvas, &page_lines(page, facts));
        }
        View::Menu(menu) => menu.render(&mut canvas, &Table::new([]), &MenuStyle::new(&SPLEEN_6X12)),
        View::About => {
            title(&mut canvas, "About", None);
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

/// The local IPv4 addresses in `/proc/net/fib_trie`, without the loopback: each `|-- a.b.c.d`
/// followed by `/32 host LOCAL`.
pub fn local_addresses(fib_trie: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut last = None;
    for line in fib_trie.lines() {
        let line = line.trim();
        if let Some(address) = line.strip_prefix("|-- ") {
            last = Some(address.to_owned());
        } else if line.starts_with("/32 host LOCAL")
            && let Some(address) = last.take()
            && !address.starts_with("127.")
            && !out.contains(&address)
        {
            out.push(address);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::display::as_text;

    fn facts() -> Facts {
        Facts {
            name: "fiber-abcdef12".into(),
            addresses: vec!["192.168.1.23".into()],
            ts: 1_794_952_515,
            components: vec![
                ("base".into(), "ok".into()),
                ("mosquitto".into(), "ok".into()),
                ("agent".into(), "ok".into()),
                ("node-red".into(), "failed".into()),
            ],
            source: "PoE".into(),
            volts: Some((12.08, 4.11)),
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

    #[test]
    fn every_page() {
        let mut screen = Screen::default();
        for (n, name) in ["page-fiber", "page-components", "page-power", "page-thermometers"]
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
        facts.apply = Some(("node-red".into(), 4, 6));
        same_as_stored("applying", &draw(&mut screen, &facts));
    }

    #[test]
    fn the_buttons() {
        let mut screen = Screen::default();
        screen.press(click(UP));
        assert!(matches!(screen.view, View::Page(3)));
        screen.press(click(DOWN));
        screen.press(click(DOWN));
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

        // The menu, About, and back to the pages.
        screen.press(click(ENTER));
        same_as_stored("menu", &draw(&mut screen, &facts()));
        screen.press(click(ENTER));
        assert!(matches!(screen.view, View::About));
        same_as_stored("about", &draw(&mut screen, &facts()));
        screen.press(click(UP));
        assert!(matches!(screen.view, View::Menu(_)));
        screen.press(click(DOWN));
        screen.press(click(ENTER));
        assert!(matches!(screen.view, View::Page(0)));
    }

    #[test]
    fn many_components_and_long_names() {
        let mut facts = facts();
        facts.components.push(("chirpstack".into(), "skipped".into()));
        let lines = page_lines(1, &facts);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[3], "and 2 more");
        assert_eq!(
            page_lines(
                2,
                &Facts {
                    source: "not known".into(),
                    ..Facts::default()
                }
            ),
            ["not known"]
        );
        assert_eq!(page_lines(0, &Facts::default())[1], "no address");
    }

    #[test]
    fn addresses_from_the_fib_trie() {
        let fib_trie = "Main:\n  +-- 0.0.0.0/0 3 0 5\n     |-- 0.0.0.0\n        /0 universe UNICAST\n\
                        \x20    +-- 127.0.0.0/8 2 0 2\n        |-- 127.0.0.1\n           /32 host LOCAL\n\
                        \x20    |-- 192.168.1.23\n        /32 host LOCAL\n     |-- 192.168.1.255\n\
                        \x20       /32 link BROADCAST\nLocal:\n     |-- 192.168.1.23\n        /32 host LOCAL\n\
                        \x20    |-- 10.0.0.5\n        /32 host LOCAL\n";
        assert_eq!(local_addresses(fib_trie), ["192.168.1.23", "10.0.0.5"]);
        assert!(local_addresses("").is_empty());
    }
}
