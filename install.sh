#!/bin/bash
# =============================================================================
# FIBER Dev Platform — Install Script
# For Raspberry Pi 4 Compute Module running Raspberry Pi OS (64-bit, Bookworm)
#
# This script installs and configures all components needed for the FIBER
# student development platform. Each section is documented so students
# can understand what is being installed and why.
#
# Usage: sudo bash install.sh
# =============================================================================

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# MQTT credentials (must match fiber.config.yaml)
MQTT_USER="fiber"
MQTT_PASS="123456789"

# =============================================================================
# Helper functions
# =============================================================================

info()  { echo -e "${BLUE}[INFO]${NC} $1"; }
ok()    { echo -e "${GREEN}[OK]${NC} $1"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $1"; }
error() { echo -e "${RED}[ERROR]${NC} $1"; }

check_root() {
    if [[ $EUID -ne 0 ]]; then
        error "This script must be run as root (sudo bash install.sh)"
        exit 1
    fi
}

# =============================================================================
# Section 1: System Prerequisites
# =============================================================================

install_base_deps() {
    info "Installing base system dependencies..."

    apt-get update
    apt-get install -y \
        i2c-tools \
        mosquitto \
        mosquitto-clients \
        sqlite3 \
        jq \
        curl \
        git

    ok "Base dependencies installed"
}

# =============================================================================
# Section 2: Hardware Interface Configuration
# =============================================================================

configure_interfaces() {
    info "Configuring hardware interfaces..."

    local CONFIG="/boot/firmware/config.txt"

    # Device tree overlays needed by FIBER hardware
    declare -A OVERLAYS=(
        ["dtoverlay=w1-gpio"]="1-Wire bus for DS18B20 temperature sensors"
        ["dtoverlay=uart3"]="Serial port (UART3)"
        ["dtoverlay=uart4"]="Serial port (UART4)"
        ["dtoverlay=pwm,pin=13,func=4"]="PWM for buzzer"
        ["dtparam=spi=on"]="Enable SPI"
        ["dtoverlay=spi6-1cs"]="SPI6 for ST7920 display"
        ["dtoverlay=i2c-rtc,pcf85063a,i2c_csi_dsi"]="RTC on I2C (CSI/DSI bus)"
    )

    for overlay in "${!OVERLAYS[@]}"; do
        if ! grep -q "^${overlay}$" "$CONFIG" 2>/dev/null; then
            echo "$overlay" >> "$CONFIG"
            info "  Added: ${overlay} — ${OVERLAYS[$overlay]}"
        else
            info "  Already present: ${overlay}"
        fi
    done

    # Enable I2C and SPI if not already enabled
    raspi-config nonint do_i2c 0 2>/dev/null || true
    raspi-config nonint do_spi 0 2>/dev/null || true

    ok "Hardware interfaces configured (reboot required to take effect)"
}

# =============================================================================
# Section 3: User Permissions
# =============================================================================

setup_permissions() {
    info "Setting up user permissions..."

    local REAL_USER="${SUDO_USER:-pi}"

    # Add the user to hardware access groups
    usermod -aG dialout "$REAL_USER"  # Serial port access (/dev/ttyAMA4)
    usermod -aG i2c "$REAL_USER"      # I2C access (/dev/i2c-10)
    usermod -aG gpio "$REAL_USER"     # GPIO access
    usermod -aG spi "$REAL_USER"      # SPI access

    ok "User '$REAL_USER' added to dialout, i2c, gpio, spi groups"
}

# =============================================================================
# Section 4: Directory Structure
# =============================================================================

setup_directories() {
    info "Creating FIBER directory structure..."

    # /opt/fiber — application binary and local config
    mkdir -p /opt/fiber

    # /data/fiber — persistent data (sensor logs, database, backups)
    mkdir -p /data/fiber/config
    mkdir -p /data/fiber/backups

    ok "Directory structure created"
    info "  /opt/fiber/        — application binary"
    info "  /data/fiber/config — configuration files"
    info "  /data/fiber/       — database, logs, backups"
}

# =============================================================================
# Section 5: Mosquitto MQTT Broker
# =============================================================================

configure_mosquitto() {
    info "Configuring Mosquitto MQTT broker..."

    # Install our config file
    cp "${SCRIPT_DIR}/config/mosquitto.conf" /etc/mosquitto/conf.d/fiber.conf

    # Create MQTT user with password
    # mosquitto_passwd creates/updates the password file
    touch /etc/mosquitto/passwd
    mosquitto_passwd -b /etc/mosquitto/passwd "$MQTT_USER" "$MQTT_PASS"

    # Restart Mosquitto to apply configuration
    systemctl restart mosquitto
    systemctl enable mosquitto

    ok "Mosquitto configured (user: $MQTT_USER, port: 1883)"
}

verify_mosquitto() {
    info "Verifying Mosquitto..."

    # Quick pub/sub test
    mosquitto_sub -h localhost -u "$MQTT_USER" -P "$MQTT_PASS" -t "test/install" -W 3 &
    local SUB_PID=$!
    sleep 1
    mosquitto_pub -h localhost -u "$MQTT_USER" -P "$MQTT_PASS" -t "test/install" -m "ok"
    wait $SUB_PID 2>/dev/null && ok "Mosquitto pub/sub test passed" || warn "Mosquitto test inconclusive (may still work)"
}

# =============================================================================
# Section 6: FIBER Application Binary
# =============================================================================

install_fiber_app() {
    info "Installing FIBER application..."

    # Copy configuration files
    cp "${SCRIPT_DIR}/config/fiber.config.yaml" /data/fiber/config/
    cp "${SCRIPT_DIR}/config/fiber.sensors.config.yaml" /data/fiber/config/

    # Install systemd service
    cp "${SCRIPT_DIR}/config/fiber.service" /etc/systemd/system/fiber.service
    systemctl daemon-reload

    # NOTE: The binary (fiber_app) must be placed in /opt/fiber/ manually.
    # It will be available from the FIBER releases repository.
    if [[ -f /opt/fiber/fiber_app ]]; then
        chmod +x /opt/fiber/fiber_app
        systemctl enable fiber.service
        ok "FIBER application installed and service enabled"
    else
        warn "Binary not found at /opt/fiber/fiber_app"
        warn "Download the binary and place it there, then run:"
        warn "  sudo chmod +x /opt/fiber/fiber_app"
        warn "  sudo systemctl enable --now fiber.service"
    fi
}

# =============================================================================
# Section 7: Node-RED + Dashboard 2.0
# =============================================================================

install_nodered() {
    info "Installing Node-RED..."

    # Use the official Raspberry Pi install script
    # This installs Node.js (LTS) and Node-RED with proper systemd integration
    bash <(curl -sL https://raw.githubusercontent.com/node-red/linux-installers/master/deb/update-nodejs-and-nodered) \
        --confirm-install --confirm-pi --no-init 2>&1 | tail -5

    ok "Node-RED installed"
}

configure_nodered() {
    info "Configuring Node-RED with Dashboard 2.0..."

    local REAL_USER="${SUDO_USER:-pi}"
    local NR_DIR="/home/${REAL_USER}/.node-red"

    # Install Dashboard 2.0 and SQLite nodes
    cd "$NR_DIR"
    sudo -u "$REAL_USER" npm install --save \
        @flowfuse/node-red-dashboard \
        node-red-node-sqlite

    # Import FIBER dashboard flows
    if [[ -f "${SCRIPT_DIR}/node-red/flows.json" ]]; then
        # Backup existing flows if any
        [[ -f "${NR_DIR}/flows.json" ]] && cp "${NR_DIR}/flows.json" "${NR_DIR}/flows.json.bak"
        cp "${SCRIPT_DIR}/node-red/flows.json" "${NR_DIR}/flows.json"
        chown "$REAL_USER":"$REAL_USER" "${NR_DIR}/flows.json"
        ok "FIBER dashboard flows imported"
    else
        warn "No flows.json found — you can import it later from the Node-RED editor"
    fi

    # Enable and start Node-RED as a service for this user
    systemctl enable "nodered.service"
    systemctl start "nodered.service"

    ok "Node-RED running on port 1880"
    info "  Dashboard: http://<pi-ip>:1880/dashboard"
    info "  Editor:    http://<pi-ip>:1880"
}

# =============================================================================
# Section 8: Final Summary
# =============================================================================

print_summary() {
    echo ""
    echo "==========================================================================="
    echo -e "${GREEN}  FIBER Dev Platform — Installation Complete${NC}"
    echo "==========================================================================="
    echo ""
    echo "  Components installed:"
    echo "    - Mosquitto MQTT broker (port 1883)"
    echo "    - Node-RED + Dashboard 2.0 (port 1880)"
    echo "    - FIBER configuration files (/data/fiber/config/)"
    echo ""
    echo "  MQTT credentials:"
    echo "    User: $MQTT_USER"
    echo "    Pass: $MQTT_PASS"
    echo ""
    echo "  Next steps:"
    echo "    1. Place the FIBER binary at /opt/fiber/fiber_app"
    echo "    2. Reboot to activate hardware interfaces"
    echo "    3. Start the FIBER service: sudo systemctl start fiber.service"
    echo "    4. Open Node-RED dashboard: http://<pi-ip>:1880/dashboard"
    echo ""
    echo "  Useful commands:"
    echo "    sudo systemctl status fiber.service    # Check FIBER app"
    echo "    sudo systemctl status mosquitto        # Check MQTT broker"
    echo "    sudo systemctl status nodered          # Check Node-RED"
    echo "    journalctl -u fiber.service -f         # FIBER app logs"
    echo "    mosquitto_sub -h localhost -u fiber -P 123456789 -t 'fiber/#' -v"
    echo "                                           # Monitor all MQTT messages"
    echo ""
    echo "==========================================================================="

    if [[ ! -f /opt/fiber/fiber_app ]]; then
        echo ""
        warn "REMINDER: The FIBER binary is not yet installed."
        warn "Download it and place at /opt/fiber/fiber_app"
    fi

    echo ""
    warn "A REBOOT is required for hardware interface changes to take effect."
    echo ""
}

# =============================================================================
# Main
# =============================================================================

main() {
    echo ""
    echo "==========================================================================="
    echo "  FIBER Dev Platform — Installer"
    echo "  For Raspberry Pi 4 Compute Module (Bookworm 64-bit)"
    echo "==========================================================================="
    echo ""

    check_root

    install_base_deps
    configure_interfaces
    setup_permissions
    setup_directories
    configure_mosquitto
    verify_mosquitto
    install_fiber_app
    install_nodered
    configure_nodered

    print_summary
}

main "$@"
