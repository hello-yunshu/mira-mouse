<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Linux Device Permissions

Never run Mira as root, use `chmod 666 /dev/hidraw*`, or install a wildcard rule for all HID devices. Mira may propose a rule only for an exact, schema-validated VID, PID, usage page, and usage declared by a trusted plugin. The exact destination and contents must be previewed before an optional `pkexec` action.

AppImage users explicitly install or remove the reviewed rule, then replug the device and rerun access diagnostics. DEB/RPM maintainer scripts must be idempotent and remove only their owned rule. Imported plugins get separate proposals and are never granted permission automatically.

