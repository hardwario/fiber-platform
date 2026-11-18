#!/usr/bin/env python3
"""Prints each thermometer's reading on the FIBER as it changes (docs/examples.md).

    python3 thermometers.py

Needs Debian's paho-mqtt: sudo apt install python3-paho-mqtt. FIBER_HOST, FIBER_PORT, FIBER_USER
and FIBER_PASSWORD say where the broker is (on the FIBER: 127.0.0.1:1883, no login).
"""

import json
import os
import sys

import paho.mqtt.client as mqtt
from paho.mqtt.enums import CallbackAPIVersion

# The API this was written for (docs/mqtt-api.md, "The rules").
SCHEMA = 1


def on_connect(client, userdata, flags, reason_code, properties):
    if reason_code.is_failure:
        print(f"The broker refused: {reason_code}", file=sys.stderr)
        return
    # Here, so that the subscriptions come back after a reconnect. The retained topics give the
    # latest state at once.
    client.subscribe([("fiber/status", 1), ("fiber/thermometers/+", 1)])


def on_message(client, userdata, msg):
    if msg.topic == "fiber/status":
        status = json.loads(msg.payload)
        if status.get("schema") != SCHEMA:
            print(f"The agent speaks schema {status.get('schema')}, this example {SCHEMA}.", file=sys.stderr)
            client.disconnect()
        elif status.get("state") != "online":
            print("The agent is offline.")
        return
    probe = msg.topic.rsplit("/", 1)[-1]
    if not msg.payload:
        # An empty message: the probe is gone.
        print(f"{probe}: gone")
        return
    reading = json.loads(msg.payload)
    temperature = reading.get("temperature")
    shown = "no reading" if temperature is None else f"{temperature:.2f} °C"
    print(f"{probe} on port {reading.get('port')}: {shown}", flush=True)


def main():
    client = mqtt.Client(CallbackAPIVersion.VERSION2)
    if os.environ.get("FIBER_USER"):
        client.username_pw_set(os.environ["FIBER_USER"], os.environ.get("FIBER_PASSWORD"))
    client.on_connect = on_connect
    client.on_message = on_message
    client.connect(os.environ.get("FIBER_HOST", "127.0.0.1"), int(os.environ.get("FIBER_PORT", "1883")))
    try:
        client.loop_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
