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
MQTT_PASS="fiber_dev"

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

    # Enable I2C and SPI via raspi-config
    raspi-config nonint do_i2c 0 2>/dev/null || true
    raspi-config nonint do_spi 0 2>/dev/null || true

    # Add [all] section with device tree overlays if not present
    # These must be in the [all] section to apply to all Pi models
    local OVERLAYS=(
        "dtoverlay=w1-gpio"
        "dtoverlay=uart3"
        "dtoverlay=uart4"
        "dtoverlay=pwm,pin=13,func=4"
        "dtparam=spi=on"
        "dtoverlay=spi6-1cs"
        "dtoverlay=i2c-rtc,pcf85063a,i2c_csi_dsi"
    )

    # Ensure [all] section exists
    if ! grep -q "^\[all\]$" "$CONFIG" 2>/dev/null; then
        echo "" >> "$CONFIG"
        echo "[all]" >> "$CONFIG"
        info "  Added [all] section"
    fi

    for overlay in "${OVERLAYS[@]}"; do
        if ! grep -q "^${overlay}$" "$CONFIG" 2>/dev/null; then
            echo "$overlay" >> "$CONFIG"
            info "  Added: ${overlay}"
        else
            info "  Already present: ${overlay}"
        fi
    done

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

    mkdir -p /opt/fiber
    mkdir -p /data/fiber/config
    mkdir -p /data/fiber/backups
    mkdir -p /data/ble

    # Dev mode marker — required by the dev-platform binary
    touch /data/fiber/config/DEV_MODE_ENABLED

    ok "Directory structure created"
}

# =============================================================================
# Section 5: Mosquitto MQTT Broker
# =============================================================================

configure_mosquitto() {
    info "Configuring Mosquitto MQTT broker..."

    # Install our config file
    cp "${SCRIPT_DIR}/config/mosquitto.conf" /etc/mosquitto/conf.d/fiber.conf

    # Create MQTT user with password and set correct permissions
    touch /etc/mosquitto/passwd
    mosquitto_passwd -b /etc/mosquitto/passwd "$MQTT_USER" "$MQTT_PASS"
    chown mosquitto:mosquitto /etc/mosquitto/passwd
    chmod 0600 /etc/mosquitto/passwd

    # Restart Mosquitto to apply configuration
    systemctl restart mosquitto
    systemctl enable mosquitto

    ok "Mosquitto configured (user: $MQTT_USER, port: 1883)"
}

verify_mosquitto() {
    info "Verifying Mosquitto..."

    mosquitto_sub -h localhost -u "$MQTT_USER" -P "$MQTT_PASS" -t "test/install" -W 3 &
    local SUB_PID=$!
    sleep 1
    mosquitto_pub -h localhost -u "$MQTT_USER" -P "$MQTT_PASS" -t "test/install" -m "ok"
    wait $SUB_PID 2>/dev/null && ok "Mosquitto pub/sub test passed" || warn "Mosquitto test inconclusive (may still work)"
}

# =============================================================================
# Section 6: FIBER Application
# =============================================================================

install_fiber_app() {
    info "Installing FIBER application..."

    # Copy configuration files
    cp "${SCRIPT_DIR}/config/fiber.config.yaml" /data/fiber/config/
    cp "${SCRIPT_DIR}/config/fiber.sensors.config.yaml" /data/fiber/config/

    # Install systemd service
    cp "${SCRIPT_DIR}/config/fiber.service" /etc/systemd/system/fiber.service
    systemctl daemon-reload

    # Copy binary if present in repo
    if [[ -f "${SCRIPT_DIR}/fiber_app" ]]; then
        cp "${SCRIPT_DIR}/fiber_app" /opt/fiber/fiber_app
        chmod +x /opt/fiber/fiber_app
        systemctl enable fiber.service
        ok "FIBER binary and service installed"
    elif [[ -f /opt/fiber/fiber_app ]]; then
        chmod +x /opt/fiber/fiber_app
        systemctl enable fiber.service
        ok "FIBER service enabled (binary already present)"
    else
        warn "Binary not found — place fiber_app in /opt/fiber/ then run:"
        warn "  sudo chmod +x /opt/fiber/fiber_app"
        warn "  sudo systemctl enable --now fiber.service"
    fi
}

# =============================================================================
# Section 7: 1-Wire Sensor Services
# =============================================================================

install_1wire_services() {
    info "Installing 1-Wire sensor services..."

    # DS2482 I2C-to-1Wire bridge initialization
    cp "${SCRIPT_DIR}/config/ds2482-init.sh" /usr/bin/
    chmod +x /usr/bin/ds2482-init.sh
    cp "${SCRIPT_DIR}/config/ds2482-init.service" /etc/systemd/system/

    # 1-Wire bus periodic rescan
    cp "${SCRIPT_DIR}/config/w1-rescan.sh" /usr/bin/
    chmod +x /usr/bin/w1-rescan.sh
    cp "${SCRIPT_DIR}/config/w1-rescan.service" /etc/systemd/system/
    cp "${SCRIPT_DIR}/config/w1-rescan.timer" /etc/systemd/system/

    systemctl daemon-reload
    systemctl enable ds2482-init.service
    systemctl enable w1-rescan.timer

    ok "1-Wire services installed (will start after reboot)"
}

# =============================================================================
# Section 8: Node-RED + Dashboard 2.0
# =============================================================================

install_nodered() {
    info "Installing Node-RED..."

    bash <(curl -sL https://raw.githubusercontent.com/node-red/linux-installers/master/deb/update-nodejs-and-nodered) \
        --confirm-install --confirm-pi --no-init 2>&1 | tail -5

    ok "Node-RED installed"
}

configure_nodered() {
    info "Configuring Node-RED with Dashboard 2.0..."

    local REAL_USER="${SUDO_USER:-pi}"
    local NR_DIR="/home/${REAL_USER}/.node-red"

    # Install Dashboard 2.0
    cd "$NR_DIR"
    sudo -u "$REAL_USER" npm install --save @flowfuse/node-red-dashboard

    # Import FIBER dashboard flows, substituting MQTT credentials inline.
    # Node-RED stores credentials in flows_cred.json (encrypted) and does not
    # expand ${ENV_VAR} inside string credential fields, so env-var injection
    # via systemd does not work — we have to bake the values into flows.json.
    if [[ -f "${SCRIPT_DIR}/node-red/flows.json" ]]; then
        [[ -f "${NR_DIR}/flows.json" ]] && cp "${NR_DIR}/flows.json" "${NR_DIR}/flows.json.bak"
        sed -e "s|\${MQTT_USER}|${MQTT_USER}|g" \
            -e "s|\${MQTT_PASS}|${MQTT_PASS}|g" \
            "${SCRIPT_DIR}/node-red/flows.json" > "${NR_DIR}/flows.json"
        chown "$REAL_USER":"$REAL_USER" "${NR_DIR}/flows.json"
        # Force Node-RED to re-encrypt credentials from the embedded block on next start
        rm -f "${NR_DIR}/flows_cred.json"
        ok "FIBER dashboard flows imported with MQTT credentials"
    else
        warn "No flows.json found — import it later from the Node-RED editor"
    fi

    systemctl enable "nodered.service"
    systemctl start "nodered.service"

    ok "Node-RED running on port 1880"
}

# =============================================================================
# Section 9: Final Summary
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
    echo "    - 1-Wire sensor services (ds2482-init, w1-rescan)"
    echo ""
    echo "  MQTT credentials:"
    echo "    User: $MQTT_USER"
    echo "    Pass: $MQTT_PASS"
    echo ""
    echo "  Next steps:"
    echo "    1. Reboot to activate hardware interfaces"
    echo "    2. After reboot: sudo systemctl start fiber.service"
    echo "    3. Open dashboard: http://<pi-ip>:1880/dashboard"
    echo ""
    echo "  Useful commands:"
    echo "    sudo systemctl status fiber.service    # Check FIBER app"
    echo "    sudo systemctl status mosquitto        # Check MQTT broker"
    echo "    sudo systemctl status nodered          # Check Node-RED"
    echo "    journalctl -u fiber.service -f         # FIBER app logs"
    echo "    ls /sys/bus/w1/devices/                # Check 1-Wire sensors"
    echo "    mosquitto_sub -h localhost -u fiber -P fiber_dev -t 'fiber/#' -v"
    echo ""
    echo "==========================================================================="

    if [[ ! -f /opt/fiber/fiber_app ]]; then
        echo ""
        warn "REMINDER: The FIBER binary is not yet installed."
        warn "Place fiber_app in /opt/fiber/ and run:"
        warn "  sudo chmod +x /opt/fiber/fiber_app"
        warn "  sudo systemctl enable --now fiber.service"
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
    install_1wire_services
    install_nodered
    configure_nodered

    print_summary
}

main "$@"
