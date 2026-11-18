#!/usr/bin/env python3
"""Sends one request to the FIBER's agent and prints its answer (docs/examples.md).

    python3 request.py <op> [params as JSON]
    python3 request.py buzzer.beep '{"pattern":"double"}'

Exit status: 0 the answer says "ok": true; 1 it does not; 2 no answer came.
Needs Debian's paho-mqtt: sudo apt install python3-paho-mqtt. FIBER_HOST, FIBER_PORT, FIBER_USER
and FIBER_PASSWORD say where the broker is (on the FIBER: 127.0.0.1:1883, no login).
"""

import json
import os
import sys
import threading
import uuid

import paho.mqtt.client as mqtt
from paho.mqtt.enums import CallbackAPIVersion

# Every request gets an answer within 10 s; firmware.flash within 2 minutes.
TIMEOUT = 130


def main():
    if len(sys.argv) not in (2, 3):
        sys.exit("usage: request.py <op> [params as JSON]")
    request = {
        # New for every request: the agent drops an id it has seen.
        "id": uuid.uuid4().hex,
        "op": sys.argv[1],
        "params": json.loads(sys.argv[2]) if len(sys.argv) == 3 else None,
    }
    answer = {}
    answered = threading.Event()

    def on_connect(client, userdata, flags, reason_code, properties):
        if reason_code.is_failure:
            print(f"The broker refused: {reason_code}", file=sys.stderr)
            answered.set()
            return
        # The answer's topic first; the request goes once the broker has the subscription.
        client.subscribe(f"fiber/rsp/{request['id']}", qos=1)

    def on_subscribe(client, userdata, mid, reason_codes, properties):
        # Never retained: a retained request is cleared and not run.
        client.publish("fiber/cmd", json.dumps(request), qos=1, retain=False)

    def on_message(client, userdata, msg):
        answer.update(json.loads(msg.payload))
        answered.set()

    client = mqtt.Client(CallbackAPIVersion.VERSION2)
    if os.environ.get("FIBER_USER"):
        client.username_pw_set(os.environ["FIBER_USER"], os.environ.get("FIBER_PASSWORD"))
    client.on_connect = on_connect
    client.on_subscribe = on_subscribe
    client.on_message = on_message
    client.connect(os.environ.get("FIBER_HOST", "127.0.0.1"), int(os.environ.get("FIBER_PORT", "1883")))
    client.loop_start()
    answered.wait(TIMEOUT)
    client.disconnect()
    client.loop_stop()
    if not answer:
        print("no answer from the agent", file=sys.stderr)
        sys.exit(2)
    print(json.dumps(answer))
    sys.exit(0 if answer.get("ok") else 1)


if __name__ == "__main__":
    main()
