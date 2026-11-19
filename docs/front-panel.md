# The front panel

The LCD (128 x 64 pixels, white backlight), the three buttons beside it (*up*, *enter*, *down*)
and the buzzer. The agent drives them. A program of your own sees the buttons and can put a
message on the display through MQTT ([`mqtt-api.md`](mqtt-api.md)).

**Status: a draft (T20, T21).** The pages are drawn and compared with stored pictures in the
tests. The driver that sends them to the LCD is written (`agent/src/agent/lcd.rs`), and on a
FIBER the pages show (2026-11-19): first upside down, with the 180-degree turn of the earlier
software, now sent as drawn. That they are upright now waits for a second look; the buttons have
not been pressed.

## The pages

Each page has a title bar with its number, and four lines below it.

1. **FIBER:** its name, its addresses (IPv4, at most two), the time in UTC.
2. **Components:** each component that is on, and how its last apply ended (`ok`, `failed`,
   `skipped`).
3. **Power:** PoE or battery, the input and the battery voltage.
4. **Thermometers:** each probe's port, the end of its id and its temperature; four at most.

The radio's page comes with the north bridge (T26).

## The buttons

- **Up** and **down** go to the page before and after, round the end. Held, they keep going
  every 0.2 s.
- **Enter** opens the menu.
- When the backlight has dimmed, the first press only lights it.

In the menu, *up* and *down* move, *enter* chooses, and *Back* leaves.

## The menu

- **About:** the versions: the `fiber` program, the south bridge's firmware, the pinned TOWER
  revision. Any button goes back.
- **Restart** asks first ("No" is the first choice), then restarts the FIBER. The agent has no
  right to do that itself: it writes `/run/fiber-agent/restart`, and `fiber-restart.path` has
  the system restart (D23).
- **Set up from the phone** comes with the Bluetooth profile (T29, [`ble.md`](ble.md)).

## Over the pages

- **An apply that runs** (`fiber platform apply`) shows its progress: the component and the
  step, until it ends.
- **A client's message** (`display.message`, `fiber display message`) shows until it is cleared
  or its time is up. The buttons go on working below it.
