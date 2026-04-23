#!/bin/bash
# Rescan all 1-Wire masters to detect newly connected sensors
for i in $(seq 1 8); do
    master="/sys/bus/w1/devices/w1_bus_master${i}/w1_master_search"
    [ -f "$master" ] && echo 1 > "$master"
done
