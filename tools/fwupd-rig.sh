#!/bin/bash
# fwupd test rig, sourced inside a fedora:44 container (never on the host):
#   . /src/tools/fwupd-rig.sh
# Starts a private system bus and fwupd with only its `test` plugin (a fake
# "Integrated Webcam" at 1.2.2 with an update to 1.2.4 from the local
# fwupd-tests remote), no LVFS remotes and no other plugins, so nothing
# reaches real hardware or the network. Exports DBUS_SYSTEM_BUS_ADDRESS.
# Needs fwupd, fwupd-tests, dbus-daemon and glib2 (gdbus) installed.
# fwupd's log: /tmp/fwupd.log.

rig_call() {
	gdbus call --system -d org.freedesktop.fwupd -o / -m "org.freedesktop.fwupd.$1" "${@:2}"
}

rig_start() {
	"${FWUPD_DAEMON:-/usr/libexec/fwupd/fwupd}" >>/tmp/fwupd.log 2>&1 &
	FWUPD_PID=$!
	for _ in $(seq 120); do
		rig_call GetRemotes >/dev/null 2>&1 && return 0
		sleep 0.25
	done
	echo "fwupd-rig: fwupd did not come up (see /tmp/fwupd.log)" >&2
	return 1
}

mkdir -p /run/dbus
[ -S /run/dbus/system_bus_socket ] || dbus-daemon --system --fork --nopidfile
export DBUS_SYSTEM_BUS_ADDRESS=unix:path=/run/dbus/system_bus_socket

for r in /etc/fwupd/remotes.d/*.conf; do
	sed -i 's/^Enabled=.*/Enabled=false/' "$r"
done
printf '[fwupd]\nTestDevices=true\n' >/etc/fwupd/fwupd.conf
rig_start || return 1 2>/dev/null || exit 1

# Every plugin but `test` off, then start again with that.
disabled=$(rig_call GetPlugins | grep -o "'Name': <'[^']*'>" | sed "s/'Name': <'\([^']*\)'>/\1/" |
	grep -vx test | paste -sd ';')
kill "$FWUPD_PID"
wait "$FWUPD_PID" 2>/dev/null
printf '[fwupd]\nTestDevices=true\nDisabledPlugins=%s\n' "$disabled" >/etc/fwupd/fwupd.conf
rig_start || return 1 2>/dev/null || exit 1
fwupdmgr refresh --force -y >>/tmp/fwupd.log 2>&1 || true
