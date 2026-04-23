# FIBER Dev Platform -- Student Installation Guide

Complete step-by-step guide to set up the FIBER IoT temperature monitoring system on a Raspberry Pi 4 Compute Module. By the end of this guide you will have sensors publishing data to an MQTT broker and a live dashboard in your browser.

---

## Table of Contents

1. [What You Need](#1-what-you-need)
2. [Flash Raspberry Pi OS](#2-flash-raspberry-pi-os)
3. [First Boot and Connection](#3-first-boot-and-connection)
4. [Clone the Dev Platform Repository](#4-clone-the-dev-platform-repository)
5. [Run the Install Script](#5-run-the-install-script)
6. [Install the FIBER Binary](#6-install-the-fiber-binary)
7. [Reboot](#7-reboot)
8. [Start the FIBER Application](#8-start-the-fiber-application)
9. [Open the Dashboard](#9-open-the-dashboard)
10. [Verify Everything Works](#10-verify-everything-works)
11. [Running Node-RED on Your PC (Optional)](#11-running-node-red-on-your-pc-optional)
12. [Useful Commands Reference](#12-useful-commands-reference)
13. [Troubleshooting](#13-troubleshooting)

---

## 1. What You Need

### Hardware

| Item | Description |
|------|-------------|
| Raspberry Pi 4 Compute Module + IO Board | The main computer that runs everything |
| MicroSD card (32 GB minimum) | Storage for the operating system and data |
| DS18B20 temperature sensors | 1-Wire digital sensors (up to 8 lines) |
| 4.7k ohm resistor | Pull-up resistor between DS18B20 data pin and VCC |
| 5V / 3A USB-C power supply | Powers the Raspberry Pi |
| Ethernet cable or WiFi network | Network connection for SSH and the dashboard |

### Software (on your laptop/PC)

| Software | Where to get it |
|----------|-----------------|
| Raspberry Pi Imager | https://www.raspberrypi.com/software/ |
| SSH client | Built-in on macOS/Linux. On Windows use PuTTY or Windows Terminal |

### Wiring the DS18B20 Sensors

```
     DS18B20 (flat side facing you)
     ┌──────────┐
     │ 1  2  3  │
     └──────────┘
       │  │  │
       │  │  └── Pin 3: VCC  ────── 3.3V (Pin 1 on Pi header)
       │  │
       │  └───── Pin 2: DATA ────── GPIO4 (Pin 7 on Pi header)
       │                    │
       │              ┌─────┘
       │              │
       │           [4.7k ohm]  ← pull-up resistor between DATA and VCC
       │              │
       │              └──── 3.3V
       │
       └──────── Pin 1: GND  ────── GND (Pin 9 on Pi header)
```

Multiple DS18B20 sensors can share the same data line (they each have a unique address).

---

## 2. Flash Raspberry Pi OS

1. Install **Raspberry Pi Imager** on your laptop/PC and open it
2. Click **Choose Device** and select **Raspberry Pi 4**
3. Click **Choose OS** and select:
   - **Raspberry Pi OS (64-bit)** -- either Lite (no desktop) or Desktop version
   - Make sure it says **Bookworm** (Debian 12)
4. Click **Choose Storage** and select your MicroSD card
5. Click **Next** -- a settings dialog will appear. Click **Edit Settings** and configure:

   **General tab:**
   - Set hostname: e.g. `FIBER-001`
   - Set username and password: e.g. `fiber` / choose a password
   - Configure WiFi: enter your network name and password (skip if using Ethernet)
   - Set timezone to your local timezone

   **Services tab:**
   - Enable SSH: select **Use password authentication**

6. Click **Save**, then **Yes** to apply the settings
7. Wait for the flashing to complete (this takes a few minutes)
8. Remove the MicroSD card from your PC

---

## 3. First Boot and Connection

1. Insert the MicroSD card into the Raspberry Pi
2. Connect Ethernet cable (if not using WiFi)
3. Connect the power supply -- the Pi will boot automatically
4. Wait about 60 seconds for the first boot to complete

### Find the Pi's IP Address

You need the Pi's IP address to connect via SSH. Use one of these methods:

**Method A -- From your router:**
Log into your router's admin page and look for a device named `FIBER-001` (or whatever hostname you set).

**Method B -- Using ping (if you set a hostname):**
```bash
ping FIBER-001.local
```

**Method C -- Using nmap (scan your network):**
```bash
# Replace 192.168.1.0 with your network range
nmap -sn 192.168.1.0/24
```

### Connect via SSH

```bash
ssh fiber@<pi-ip-address>
```

Replace `<pi-ip-address>` with the actual IP (e.g. `192.168.1.50`). Type `yes` when asked about the fingerprint, then enter your password.

### Update the System

Once connected, update all packages to the latest versions:

```bash
sudo apt update && sudo apt upgrade -y
```

This may take a few minutes. Wait for it to finish.

---

## 4. Clone the Dev Platform Repository

Download the dev-plat repository to the Pi:

```bash
cd ~
git clone <repo-url> fiber-dev-plat
cd fiber-dev-plat
```

> Replace `<repo-url>` with the actual repository URL provided by your instructor.

Verify the files are there:

```bash
ls -la
```

You should see:

```
install.sh
config/
docs/
node-red/
README.md
```

---

## 5. Run the Install Script

The install script automatically sets up all the components. It is fully commented so you can read it to understand what each step does.

```bash
sudo bash install.sh
```

The script will take several minutes to complete. It installs and configures:

| Step | What it does |
|------|-------------|
| System dependencies | Installs i2c-tools, mosquitto, sqlite3, curl, git |
| Hardware interfaces | Enables I2C, SPI, UART, and 1-Wire in `/boot/firmware/config.txt` |
| User permissions | Adds your user to `dialout`, `i2c`, `gpio`, `spi` groups |
| Directory structure | Creates `/opt/fiber/` (binary) and `/data/fiber/` (data and config) |
| Mosquitto MQTT broker | Configures the message broker on port 1883 |
| FIBER config files | Copies `fiber.config.yaml` and `fiber.sensors.config.yaml` |
| Node-RED | Installs Node-RED with Dashboard 2.0 on port 1880 |
| Dashboard flows | Imports the FIBER dashboard into Node-RED |

When it finishes, you will see a summary showing all installed components and the MQTT credentials:

```
  MQTT credentials:
    User: fiber
    Pass: 123456789
```

> **Write these down.** You will need them later.

---

## 6. Install the FIBER Binary

The FIBER application is a pre-built Rust binary. Download it from the location provided by your instructor.

```bash
# Copy the binary to the correct location
sudo cp fiber_app /opt/fiber/fiber_app
sudo chmod +x /opt/fiber/fiber_app
```

Verify it is in place:

```bash
ls -la /opt/fiber/fiber_app
```

You should see something like:

```
-rwxr-xr-x 1 root root  <size>  <date> /opt/fiber/fiber_app
```

> If you don't have the binary yet, you can still continue with the next steps. The MQTT broker and Node-RED will work without it -- you just won't have sensor data until the binary is installed.

---

## 7. Reboot

A reboot is required for the hardware interface changes (device tree overlays) to take effect.

```bash
sudo reboot
```

Wait about 60 seconds, then reconnect via SSH:

```bash
ssh fiber@<pi-ip-address>
```

---

## 8. Start the FIBER Application

Enable and start the FIBER service:

```bash
sudo systemctl enable --now fiber.service
```

This tells the system to:
- **enable**: start the service automatically on every boot
- **--now**: also start it immediately right now

Check that it is running:

```bash
sudo systemctl status fiber.service
```

You should see `active (running)` in green. If there are errors, check the logs:

```bash
journalctl -u fiber.service -f
```

Press `Ctrl+C` to stop following the logs.

---

## 9. Open the Dashboard

On your laptop/PC, open a web browser and navigate to:

```
http://<pi-ip-address>:1880/dashboard
```

Replace `<pi-ip-address>` with the Pi's IP address (e.g. `http://192.168.1.50:1880/dashboard`).

You should see the FIBER Dashboard with:
- **Overview** -- temperature gauges for each sensor line, color-coded by alarm state
- **Charts** -- real-time temperature line charts
- **Alarms** -- alarm event log
- **System** -- device power, network, and storage status

> If the dashboard is blank or shows no data, check section [10. Verify Everything Works](#10-verify-everything-works).

### Node-RED Editor

You can also open the Node-RED flow editor to see how the dashboard is built:

```
http://<pi-ip-address>:1880
```

This is where you can modify flows, add new widgets, or create your own data processing logic.

---

## 10. Verify Everything Works

Run these commands on the Pi to check each component:

### Check all services are running

```bash
sudo systemctl status mosquitto --no-pager
sudo systemctl status nodered --no-pager
sudo systemctl status fiber.service --no-pager
```

All three should show `active (running)`.

### Check sensors are detected

```bash
ls /sys/bus/w1/devices/
```

You should see directories starting with `28-` (one per DS18B20 sensor). If you see only `w1_bus_master1`, no sensors are detected -- check your wiring.

### Read a sensor value directly

```bash
cat /sys/bus/w1/devices/28-*/w1_slave
```

You should see output ending with a line like `t=36500` which means 36.5 degrees Celsius.

### Check I2C devices

```bash
i2cdetect -y 10
```

### Monitor MQTT messages

This is the most useful verification. It shows all messages flowing through the system in real-time:

```bash
mosquitto_sub -h localhost -u fiber -P 123456789 -t 'fiber/#' -v
```

You should see messages appearing every few seconds with sensor data, system info, and alarms. Press `Ctrl+C` to stop.

### Test MQTT manually

Send a test message and verify you receive it:

```bash
# In one terminal, subscribe:
mosquitto_sub -h localhost -u fiber -P 123456789 -t 'test/hello' -v

# In another terminal (open a second SSH session), publish:
mosquitto_pub -h localhost -u fiber -P 123456789 -t 'test/hello' -m 'it works!'
```

You should see `test/hello it works!` appear in the first terminal.

### Request system info via MQTT

```bash
HOSTNAME=$(hostname)
mosquitto_pub -h localhost -u fiber -P 123456789 \
  -t "fiber/$HOSTNAME/commands/system/get_info" \
  -m '{"command":"get_info"}'
```

The response will appear on the `fiber/$HOSTNAME/system/info` topic (visible if you have the `mosquitto_sub` command running from above).

---

## 11. Running Node-RED on Your PC (Optional)

You can run Node-RED on your own laptop instead of on the Pi. This is useful for developing flows locally while connecting to the Pi's MQTT broker remotely.

### Install Node-RED on your PC

```bash
npm install -g node-red
```

Then start it:

```bash
node-red
```

Open `http://localhost:1880` in your browser.

### Install required packages

In the Node-RED editor:

1. Click the **menu** (top-right hamburger icon) -> **Manage palette**
2. Go to the **Install** tab
3. Search and install:
   - `@flowfuse/node-red-dashboard`
   - `node-red-node-sqlite`

### Import the FIBER flows

1. Click **menu** -> **Import**
2. Select the file `node-red/flows.json` from the dev-plat repository
3. Click **Import**

### Configure the MQTT connection

The flows are configured to connect to `localhost`. Since the MQTT broker is on the Pi, you need to change it:

1. Double-click any **purple MQTT node** in the flow
2. Click the **pencil icon** next to the Server field
3. Change **Server** from `localhost` to the Pi's IP address (e.g. `192.168.1.50`)
4. Confirm **Port** is `1883`
5. Go to the **Security** tab and verify:
   - Username: `fiber`
   - Password: `123456789`
6. Click **Update**, then **Done**
7. Click **Deploy** (red button, top-right)

The MQTT nodes should now show a green **connected** dot.

Open `http://localhost:1880/dashboard` to see the dashboard.

---

## 12. Useful Commands Reference

### Service Management

```bash
# Start / stop / restart services
sudo systemctl start fiber.service
sudo systemctl stop fiber.service
sudo systemctl restart fiber.service

sudo systemctl restart mosquitto
sudo systemctl restart nodered

# Check service status
sudo systemctl status fiber.service
sudo systemctl status mosquitto
sudo systemctl status nodered

# View live logs
journalctl -u fiber.service -f
journalctl -u mosquitto -f
journalctl -u nodered -f
```

### MQTT Commands

```bash
# Set variables for convenience
HOSTNAME=$(hostname)
MQTT="-h localhost -u fiber -P 123456789"

# Monitor all FIBER messages
mosquitto_sub $MQTT -t 'fiber/#' -v

# Monitor only sensor data
mosquitto_sub $MQTT -t 'fiber/+/sensors/aggregated' -v

# Monitor only alarms
mosquitto_sub $MQTT -t 'fiber/+/alarms/events' -v

# Monitor power topics
mosquitto_sub $MQTT -t 'fiber/+/power/#' -v

# Request system info
mosquitto_pub $MQTT -t "fiber/$HOSTNAME/commands/system/get_info" \
  -m '{"command":"get_info"}'

# Get current sensor config
mosquitto_pub $MQTT -t "fiber/$HOSTNAME/commands/sensor/get_config" \
  -m '{"command":"get_sensor_config"}'

# Get current intervals
mosquitto_pub $MQTT -t "fiber/$HOSTNAME/commands/system/get_interval" \
  -m '{"command":"get_interval"}'

# Set alarm threshold on sensor line 0 (4-level)
mosquitto_pub $MQTT -t "fiber/$HOSTNAME/commands/sensor/set_threshold" \
  -m '{"command":"set_threshold","line":0,"thresholds":{"critical_low":18,"warning_low":27,"warning_high":38.5,"critical_high":41}}'

# Set alarm threshold on sensor line 0 (6-level with alarm_low/alarm_high)
mosquitto_pub $MQTT -t "fiber/$HOSTNAME/commands/sensor/set_threshold" \
  -m '{"command":"set_threshold","line":0,"thresholds":{"critical_low":18,"alarm_low":20,"warning_low":27,"warning_high":38.5,"alarm_high":40,"critical_high":41}}'

# Set intervals
mosquitto_pub $MQTT -t "fiber/$HOSTNAME/commands/system/set_interval" \
  -m '{"command":"set_interval","sample_interval_ms":2000,"aggregation_interval_ms":15000,"report_interval_ms":60000}'

# Silence the buzzer
mosquitto_pub $MQTT -t "fiber/$HOSTNAME/commands/sensor/silence_buzzer" \
  -m '{"command":"silence_buzzer"}'

# Restart application
mosquitto_pub $MQTT -t "fiber/$HOSTNAME/commands/system/restart" \
  -m '{"command":"restart"}'
```

### Network and Hardware

```bash
# Find your Pi's IP address
hostname -I

# Check connected 1-Wire sensors
ls /sys/bus/w1/devices/

# Read a sensor directly
cat /sys/bus/w1/devices/28-*/w1_slave

# Check I2C devices
i2cdetect -y 10

# Check which groups your user belongs to
groups
```

---

## 13. Troubleshooting

### Sensors not detected (`ls /sys/bus/w1/devices/` shows nothing)

1. Verify the 1-Wire overlay is enabled:
   ```bash
   grep w1-gpio /boot/firmware/config.txt
   ```
   You should see `dtoverlay=w1-gpio`. If not, add it and reboot.

2. Check your wiring:
   - Data pin connected to **GPIO4** (physical pin 7)
   - 4.7k ohm pull-up resistor between data and 3.3V
   - GND connected to Pi GND

3. Reboot if you just added the overlay:
   ```bash
   sudo reboot
   ```

### FIBER app won't start

Check the logs for the specific error:
```bash
journalctl -u fiber.service --no-pager -n 50
```

Common causes:
- **Binary not found:** verify `ls -la /opt/fiber/fiber_app` shows the file with execute permission
- **Config missing:** verify `ls /data/fiber/config/` shows `fiber.config.yaml` and `fiber.sensors.config.yaml`
- **Permission denied on serial port:** run `sudo usermod -aG dialout $USER` and reboot
- **MQTT connection refused:** check Mosquitto is running: `sudo systemctl status mosquitto`

### Node-RED dashboard is blank

1. Open the Node-RED editor at `http://<pi-ip>:1880`
2. Check if MQTT nodes show a green **connected** dot or a red **disconnected** indicator
3. If disconnected: double-click an MQTT node and verify:
   - Server: `localhost`
   - Port: `1883`
   - Username: `fiber`
   - Password: `123456789`
4. Click **Deploy** after making changes
5. Verify MQTT is working: run `mosquitto_sub -h localhost -u fiber -P 123456789 -t 'fiber/#' -v` on the Pi

### MQTT authentication errors

```bash
# Test the connection manually
mosquitto_pub -h localhost -u fiber -P 123456789 -t test -m hello
```

If you get `Connection Refused: not authorised`:
```bash
# Recreate the password
sudo mosquitto_passwd -b /etc/mosquitto/passwd fiber 123456789
sudo systemctl restart mosquitto
```

### Cannot SSH into the Pi

- Make sure the Pi has finished booting (wait 60 seconds after power on)
- Check if the Pi is on the network: `ping <pi-ip>` or `ping FIBER-001.local`
- If using WiFi, verify the credentials were entered correctly during flashing
- Try connecting via Ethernet instead

### Serial port permission denied

```bash
sudo usermod -aG dialout $USER
```

Log out and back in (or reboot) for the group change to take effect.

---

## Summary: What's Running After Installation

```
┌─────────────────────────────────────────────────────────┐
│                  Raspberry Pi 4 CM                       │
│                                                         │
│   DS18B20 Sensors                                       │
│        │                                                │
│        ▼                                                │
│   FIBER App (/opt/fiber/fiber_app)                      │
│        │         reads sensors, publishes data           │
│        ▼                                                │
│   Mosquitto MQTT Broker (port 1883)                     │
│        │         routes messages between components      │
│        ▼                                                │
│   Node-RED + Dashboard 2.0 (port 1880)                  │
│        │         visualizes data in the browser          │
│        ▼                                                │
└────────┼────────────────────────────────────────────────┘
         │
    ┌────┴────┐
    │ Browser │  http://<pi-ip>:1880/dashboard
    └─────────┘
```

| Service | Port | URL / Command |
|---------|------|---------------|
| MQTT Broker | 1883 | `mosquitto_sub -h localhost -u fiber -P 123456789 -t 'fiber/#' -v` |
| Node-RED Editor | 1880 | `http://<pi-ip>:1880` |
| Node-RED Dashboard | 1880 | `http://<pi-ip>:1880/dashboard` |

**MQTT Credentials:** user `fiber`, password `123456789`
