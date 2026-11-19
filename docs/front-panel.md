# The front panel

The LCD (128 x 64 pixels, white backlight), the three buttons beside it (*up*, *enter*, *down*)
and the buzzer. The agent drives them. A program of your own sees the buttons and can put a
message on the display through MQTT ([`mqtt-api.md`](mqtt-api.md)).

**Status: a draft (T20, T21).** The pages are drawn and compared with stored pictures in the
tests. On a FIBER (2026-11-19) the pages show on the LCD, upright, the buttons move through them,
the backlight lights and dims after a minute, and the buzzer sounds. The panel is mounted upside
down, so the driver (`agent/src/agent/lcd.rs`) turns the picture by 180 degrees.

## The pages

Each page has a title bar, white on black: the page's icon and title, and on the right a dot
for each page, the one shown as a filled square.

1. **FIBER:** its name, its first IPv4 address (`+1` when there are more), and below a line the
   local time in big digits, with the zone's abbreviation and the day beside it (`CET`,
   `Thu 19 Nov`). The zone is the system's, `/etc/localtime`, which FIBER Setup sets and
   `sudo timedatectl set-timezone` changes; the page follows a change within a second. Without
   one, UTC.
2. **Components:** each component that is on, with a check when its last apply ended `ok`, a
   cross and `failed`, or a ring and how it ended (`skipped`); four at most, the last saying how
   many more there are.
3. **Power:** PoE or battery with its icon, the input voltage in big digits, and a battery
   gauge filled from 3.0 V (empty) to 4.2 V (full) with the voltage, or "No battery" when there
   is no cell (`battery` is `null`).
4. **Thermometers:** each probe's port in a black badge, the end of its id and its temperature;
   four at most. A single probe in big digits; with none, a word on how to plug one in.

The pictures are drawn with the HMI crate of the pinned TOWER revision (Spleen fonts in four
sizes) and icons of our own (`agent/src/agent/icons.rs`), and every one is kept as a stored
picture that the tests compare (`agent/src/agent/pictures/`).

The radio's page comes with the north bridge (T26).

## The buttons

- **Up** and **down** go to the page before and after, round the end, as the button goes down.
  Held, they keep going every 0.2 s after 0.8 s.
- **Enter** opens the menu, when it comes up again: a hold of *enter* is something else (in the
  menu it goes back).
- When the backlight has dimmed, the first press only lights it: nothing else of that press acts.
- **Each press clicks** the buzzer (25 ms) as the button goes down, the one that only lights the
  backlight too. A click never sounds over a pattern a client asked for: a press does not cut an
  alarm short.

In the menu, *up* and *down* move, *enter* chooses, and *Back* leaves.

**How fast it answers.** The LCD's controller takes 72 µs for each of its instructions, and a
full picture is 1152 of them: about 83 ms by its datasheet, 91 ms as the agent sends it (measured
on a FIBER, 2026-11-19). The agent therefore sends only the 16-pixel words that changed, row by
row, and turns the page as the button goes down, not when it comes up.

## The menu

- **About:** the versions: the `fiber` program, the south bridge's firmware, the pinned TOWER
  revision. Any button goes back.
- **Restart** asks first ("No" is the first choice), then restarts the FIBER. The agent has no
  right to do that itself: it writes `/run/fiber-agent/restart`, and `fiber-restart.path` has
  the system restart (D23).
- **Set up from the phone** comes with the Bluetooth profile (T29, [`ble.md`](ble.md)).

## Over the pages

- **An apply that runs** (`fiber platform apply`) shows its progress: the component, a bar and
  the step, until it ends.
- **A client's message** (`display.message`, `fiber display message`) shows until it is cleared
  or its time is up. The buttons go on working below it.
