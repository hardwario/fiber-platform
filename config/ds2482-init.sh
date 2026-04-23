#!/bin/bash
# DS2482 I2C-to-1Wire Bridge Initialization
# Registers the DS2482 chip on I2C bus 10 at address 0x18
# This creates the 1-Wire master that DS18B20 sensors connect through

# Wait for I2C bus to be available (up to 10 seconds)
for i in $(seq 1 10); do
    [ -d /sys/bus/i2c/devices/i2c-10 ] && break
    sleep 1
done

# Check if already initialized
if [ -d /sys/bus/i2c/devices/10-0018 ]; then
    echo "DS2482 already initialized"
    exit 0
fi

# Initialize DS2482 on I2C bus 10, address 0x18
echo ds2482 0x18 > /sys/bus/i2c/devices/i2c-10/new_device
echo "DS2482 initialized"
