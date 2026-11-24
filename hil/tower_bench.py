#!/usr/bin/env python3
"""Bench: the radio between a FIBER and TOWER nodes (T27, docs/firmware/radio-compat.md, "The
bench run"). Run it on the computer the Core Modules are plugged into.

    python3 tower_bench.py clicks FIBER DEVICE... [--count N] [--every S]
    python3 tower_bench.py set FIBER DEVICE
    python3 tower_bench.py restarts FIBER DEVICE [--count N]
    python3 tower_bench.py soak FIBER DEVICE... --hours H [--every S]
    python3 tower_bench.py --self-test

FIBER is what `ssh` takes to reach the FIBER (`fiber@fiber.local`); the script runs
`mosquitto_sub`, `mosquitto_pub` and `fiber` there and needs no password. Each DEVICE is a Core
Module's serial port, running TOWER's `radio_button` at the pinned revision and paired with the
FIBER; TOWER's command line (`tower`, built from the same revision) presses its button through
its console (`button simulate 100`) and reads its address (`node status`).

- `clicks`: the nodes in turn, one click every S seconds (5), N clicks in all (20): each click on
  `fiber/radio/nodes/{addr}/event/button`, the counts without a gap, and the share at the first
  try. A second try comes 200 ms or more after the first (the node's wait for the
  acknowledgement), so a click whose event comes more than 0.15 s later than the usual delay
  was a second try or later; one that never comes counts as not at the first try either.
- `set`: `led` set to `false` and back to `true` over MQTT; each must be reported on
  `state/led` after the node's next click.
- `restarts`: the node restarted (`system reboot`), then a click: each must arrive at once.
- `soak`: `clicks` for H hours (the 24-hour run), with a line every 10 minutes
  (`--report-every`) and the north bridge's state (`fiber firmware`) at each.

It ends with 0 when everything passed, 1 when something did not. Nothing here writes a chip.
Status: run on a FIBER (2026-11-20: clicks, set, restarts, a short soak). The first 18-hour soak
(2026-11-23) read the events through one long SSH session, which was reset after 25 minutes, and
its count stopped there; the events are now kept in a file on the FIBER and fetched as it grows.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import time

PRESS = "button simulate 100"
# How much later than the usual delay an event may come and still be a first try: a second try
# comes after the node's 200 ms wait for the acknowledgement and a backoff of up to 100 ms.
SECOND_TRY = 0.15
# How long an event may take at all before its click counts as lost.
LOST_AFTER = 5.0
FIRST_TRY_NEEDED = 0.99


def tower(device, command, timeout=20):
    """One shell command on a node's console; its reply, or None when `tower` hung or failed (a
    soak goes on: the click is then not counted as sent)."""
    try:
        out = subprocess.run(
            ["tower", "-d", device, "exec", command],
            capture_output=True, text=True, timeout=timeout, check=False,
        )
    except subprocess.TimeoutExpired:
        return None
    return out.stdout.strip() if out.returncode == 0 else None


# SSH that notices a dead link within half a minute, instead of hanging on it.
SSH = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-o", "ServerAliveInterval=10",
       "-o", "ServerAliveCountMax=3"]


def ssh_run(fiber, command, timeout=60, tries=3):
    """One command on the FIBER; its output, or None when every try failed. A long run goes on
    through a network blip: the next call simply tries again."""
    for attempt in range(tries):
        try:
            out = subprocess.run(SSH + [fiber, command], capture_output=True, text=True,
                                 timeout=timeout, check=False)
            if out.returncode == 0:
                return out.stdout
        except subprocess.TimeoutExpired:
            pass
        if attempt + 1 < tries:
            time.sleep(5)
    return None


def node_address(device):
    """The node's address, from `node status` (`addr 0xdf438142, kind button ...`)."""
    m = re.search(r"addr (0x[0-9a-f]{8})", tower(device, "node status"))
    if not m or m.group(1) == "0x00000000":
        raise SystemExit(f"{device}: not a paired TOWER node")
    return m.group(1)


def clock_offset(fiber):
    """The FIBER's clock less this computer's, in seconds: from the reply with the shortest
    round trip of five, its time taken as the middle of the trip."""
    best = None
    for _ in range(5):
        before = time.time()
        out = ssh_run(fiber, "date +%s.%N")
        after = time.time()
        if out is None:
            continue
        trip = after - before
        offset = float(out.strip()) - (before + after) / 2
        if best is None or trip < best[0]:
            best = (trip, offset)
    if best is None:
        raise SystemExit(f"{fiber}: no answer over SSH")
    return best[1]


def take(rest, chunk):
    """The watched messages in `rest` (an unfinished line from before) and `chunk` (what the
    file grew by): (time, topic, payload) for each whole line, and the unfinished rest."""
    *whole, rest = (rest + chunk).split("\n")
    lines = []
    for line in whole:
        parts = line.split(" ", 2)
        if len(parts) == 3:
            try:
                lines.append((float(parts[0]), parts[1], parts[2]))
            except ValueError:
                pass
    return lines, rest


class Watch:
    """`mosquitto_sub` on the FIBER itself, every message of the node topics with the time the
    FIBER took it, into a file there; this computer fetches what the file grew by over SSH. A
    network blip then loses nothing: the next fetch takes up what came meanwhile. (The first
    18-hour soak, 2026-11-23, read one long SSH session; it was reset after 25 minutes and the
    count stopped there.) `now()` is the time on the FIBER's clock, for the clicks."""

    def __init__(self, fiber, topics, keep=None):
        self.fiber = fiber
        self.offset = clock_offset(fiber)
        self.path = f"/tmp/tower-bench-{os.getpid()}-{int(time.time())}.log"
        subs = " ".join(f"-t '{t}'" for t in topics)
        pid = ssh_run(fiber, f"nohup mosquitto_sub -h localhost -F '%U %t %p' {subs} "
                             f"> {self.path} 2> /dev/null < /dev/null & echo $!")
        if pid is None or not pid.strip().isdigit():
            raise SystemExit(f"{fiber}: could not start mosquitto_sub")
        self.pid = pid.strip()
        self.keep = keep
        self.lines = []
        self.size = 0
        self.rest = ""
        self.failed = 0
        time.sleep(2)  # the subscription in place before the first click

    def fetch(self):
        out = ssh_run(self.fiber, f"tail -c +{self.size + 1} {self.path}", tries=1)
        if out is None:
            self.failed += 1
            return
        self.size += len(out.encode())
        lines, self.rest = take(self.rest, out)
        self.lines += lines
        if self.keep and lines:
            with open(self.keep, "a", encoding="utf-8") as f:
                for t, topic, payload in lines:
                    f.write(f"{t:.6f} {topic} {payload}\n")

    def now(self):
        return time.time() + self.offset

    def since(self, index):
        self.fetch()
        return self.lines[index:], len(self.lines)

    def stop(self):
        self.fetch()
        ssh_run(self.fiber, f"kill {self.pid}; rm -f {self.path}", tries=2)


def events_in(lines):
    """The button events among the watched messages: (time, addr, count, counter)."""
    out = []
    for t, topic, payload in lines:
        parts = topic.split("/")
        if len(parts) == 6 and parts[4] == "event" and parts[5] == "button":
            try:
                e = json.loads(payload)
            except ValueError:
                continue
            if e.get("value") == "click":
                out.append((t, parts[3], e.get("count"), e.get("counter")))
    return out


def analyse(presses, events):
    """For each node: its clicks, its events, the gaps in its counts, the delays, and how many
    came at the first try. `presses` are (time, addr), `events` (time, addr, count, counter)."""
    result = {}
    for addr in sorted({a for _, a in presses}):
        mine = sorted(t for t, a in presses if a == addr)
        evs = sorted((t, c) for t, a, c, _ in events if a == addr)
        delays = []
        for t, _ in evs:
            before = [p for p in mine if p <= t]
            if before:
                delays.append(t - before[-1])
        # The shortest delay is a first try's: nothing comes sooner than the radio allows.
        usual = min(delays) if delays else 0.0
        late = [d for d in delays if d > usual + SECOND_TRY]
        counts = [c for _, c in evs]
        # A count that jumps ahead lost events; one that drops means the node restarted.
        gaps = [(a, b) for a, b in zip(counts, counts[1:]) if b > a + 1]
        first = max(0, len(evs) - len(late))
        result[addr] = {
            "clicks": len(mine),
            "events": len(evs),
            "gaps": gaps,
            "usual": usual,
            "late": late,
            "first_try": first,
            "share": first / len(mine) if mine else 0.0,
        }
    return result


def report(result):
    ok = True
    for addr, r in result.items():
        print(
            f"{addr}: {r['clicks']} clicks, {r['events']} events, {r['first_try']} at the first try "
            f"({100 * r['share']:.1f} %), usually {r['usual']:.2f} s after the click"
            + (f", later: {', '.join(f'{d:.2f} s' for d in r['late'])}" if r["late"] else "")
            + (f", gaps in the count: {r['gaps']}" if r["gaps"] else "")
        )
        if r["events"] < r["clicks"] or r["gaps"] or r["share"] < FIRST_TRY_NEEDED:
            ok = False
    return ok


def run_clicks(fiber, devices, count, every, watch=None, log_every=None, until=None, keep=None):
    nodes = [(d, node_address(d)) for d in devices]
    own = watch is None
    watch = watch or Watch(fiber, ["fiber/radio/nodes/+/event/#"], keep=keep)
    if log_every:
        print(f"events kept on the FIBER in {watch.path}" + (f" and here in {keep}" if keep else ""),
              flush=True)
    presses = []
    unsent = 0
    last_log = time.time()
    try:
        i = 0
        while (until is None and i < count) or (until is not None and time.time() < until):
            device, addr = nodes[i % len(nodes)]
            i += 1
            # The click's time is when the node answers the command: the time `tower` takes to
            # start and open the port before it varies by a few tenths of a second.
            if tower(device, PRESS) is None:
                unsent += 1
            else:
                presses.append((watch.now(), addr))
            time.sleep(every)
            if log_every and time.time() - last_log >= log_every:
                last_log = time.time()
                lines, _ = watch.since(0)
                r = analyse(presses, events_in(lines))
                north = (ssh_run(fiber, "fiber firmware status | grep North") or "North ?").strip()
                print(time.strftime("%H:%M:%S"), north, "|",
                      "; ".join(f"{a} {v['first_try']}/{v['clicks']}" for a, v in r.items()),
                      f"| fetches failed {watch.failed}, clicks not sent {unsent}", flush=True)
        time.sleep(LOST_AFTER)
        lines, _ = watch.since(0)
    finally:
        if own:
            watch.stop()
    if unsent:
        print(f"{unsent} clicks not sent (`tower` hung or failed): not counted")
    return report(analyse(presses, events_in(lines)))


def wait_for(watch, start, topic, value, seconds):
    end = time.time() + seconds
    while time.time() < end:
        lines, _ = watch.since(start)
        for _, t, payload in lines:
            if t == topic:
                try:
                    if json.loads(payload).get("value") == value:
                        return True
                except ValueError:
                    pass
        time.sleep(0.2)
    return False


def run_set(fiber, device):
    addr = node_address(device)
    base = f"fiber/radio/nodes/{addr}"
    watch = Watch(fiber, [f"{base}/state/led"])
    ok = True
    try:
        for value in (False, True):
            _, start = watch.since(0)
            if ssh_run(fiber, f"mosquitto_pub -h localhost -q 1 -t '{base}/set/led' "
                              f"-m '{json.dumps(value)}'") is None:
                raise SystemExit(f"{fiber}: could not publish {base}/set/led")
            time.sleep(1)
            tower(device, PRESS)
            got = wait_for(watch, start, f"{base}/state/led", value, 10)
            print(f"{addr}: led {json.dumps(value)} {'reported' if got else 'NOT reported'} after its next click")
            ok = ok and got
    finally:
        watch.stop()
    return ok


def run_restarts(fiber, device, count):
    addr = node_address(device)
    watch = Watch(fiber, [f"fiber/radio/nodes/{addr}/event/#"])
    presses = []
    try:
        for _ in range(count):
            tower(device, "system reboot")
            time.sleep(8)
            tower(device, PRESS)
            presses.append((watch.now(), addr))
            time.sleep(4)
        time.sleep(LOST_AFTER)
        lines, _ = watch.since(0)
    finally:
        watch.stop()
    print("a click right after each restart of the node:")
    return report(analyse(presses, events_in(lines)))


def self_test():
    # Three clicks of one node: the second a second try (0.30 s later than usual), the third lost.
    presses = [(100.0, "0xa"), (110.0, "0xa"), (120.0, "0xa"), (101.0, "0xb"), (111.0, "0xb")]
    events = [
        (100.33, "0xa", 5, 50), (110.63, "0xa", 6, 51),
        (101.34, "0xb", 1, 10), (111.33, "0xb", 2, 11),
    ]
    r = analyse(presses, events)
    a, b = r["0xa"], r["0xb"]
    assert (a["clicks"], a["events"], a["first_try"]) == (3, 2, 1), a
    assert abs(a["late"][0] - 0.63) < 1e-9 and a["gaps"] == [], a
    assert (b["clicks"], b["events"], b["first_try"], b["gaps"]) == (2, 2, 2, []), b
    assert not report({"0xa": a}) and report({"0xb": b})
    # A gap in the count: an event lost on its way, though every click got an event.
    r = analyse([(0.0, "0xc"), (5.0, "0xc")], [(0.33, "0xc", 7, 1), (5.33, "0xc", 9, 2)])
    assert r["0xc"]["gaps"] == [(7, 9)] and not report(r)
    # A node restarted between two clicks: its count starts again, which is not a gap.
    r = analyse([(0.0, "0xd"), (20.0, "0xd")], [(0.08, "0xd", 4, 1), (20.08, "0xd", 1, 1025)])
    assert r["0xd"]["gaps"] == [] and report(r)
    lines = [
        (1.0, "fiber/radio/nodes/0xc/event/button", '{"value":"click","count":3,"counter":9}'),
        (1.1, "fiber/radio/nodes/0xc/event/button", '{"value":"hold","count":1,"counter":10}'),
        (1.2, "fiber/radio/nodes/0xc", '{"name":"button:0"}'),
        (1.3, "fiber/radio/nodes/0xc/event/button", "not json"),
    ]
    assert events_in(lines) == [(1.0, "0xc", 3, 9)]
    # The file fetched in pieces: a line cut between two fetches is taken whole by the second.
    got, rest = take("", '1.5 fiber/radio/nodes/0xc/event/button {"value":"click","count":4,"coun')
    assert got == [] and rest.startswith("1.5 ")
    got, rest = take(rest, 'ter":11}\n1.6 fiber/radio/nodes/0xc/x 1\nbad line\n')
    assert [t for t, _, _ in got] == [1.5, 1.6] and rest == ""
    assert events_in(got) == [(1.5, "0xc", 4, 11)]
    print("self-test passed")


def main():
    if "--self-test" in sys.argv:
        self_test()
        return
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="what", required=True)
    for name in ("clicks", "soak"):
        p = sub.add_parser(name)
        p.add_argument("fiber")
        p.add_argument("devices", nargs="+")
        p.add_argument("--every", type=float, default=5.0)
        if name == "clicks":
            p.add_argument("--count", type=int, default=20)
        else:
            p.add_argument("--hours", type=float, required=True)
            p.add_argument("--report-every", type=float, default=10.0, help="minutes between progress lines")
            p.add_argument("--events", help="a file here to keep every event in, as it is fetched")
    p = sub.add_parser("set")
    p.add_argument("fiber")
    p.add_argument("device")
    p = sub.add_parser("restarts")
    p.add_argument("fiber")
    p.add_argument("device")
    p.add_argument("--count", type=int, default=3)
    args = parser.parse_args()
    if args.what == "clicks":
        ok = run_clicks(args.fiber, args.devices, args.count, args.every)
    elif args.what == "soak":
        ok = run_clicks(args.fiber, args.devices, 0, args.every, log_every=args.report_every * 60,
                        until=time.time() + args.hours * 3600, keep=args.events)
    elif args.what == "set":
        ok = run_set(args.fiber, args.device)
    else:
        ok = run_restarts(args.fiber, args.device, args.count)
    print("passed" if ok else "FAILED")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
