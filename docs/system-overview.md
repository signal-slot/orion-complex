
# Ephemeral VM Lab — Development Documentation

## Overview
Disposable VM environments for development and testing.

Supports:
- Linux / Windows / macOS guests
- SSH + optional VNC
- suspend / resume / migrate
- cluster scheduling
- Google / Microsoft login
- node resource limits

---

## Node
Physical machine running environments.

Node tracks:
- providers
- resources
- load
- resource limits

Scheduler places environments based on limits.

---

## Image
Immutable VM base image.

Examples:
- base OS
- devtools image
- Qt build image

Images are composed of layers and chunks.

---

## Environment
VM instance created from an image.

Capabilities:
- SSH access
- VNC
- suspend
- resume
- reboot
- force reboot
- migrate

States:
creating
running
suspending
suspended
resuming
rebooting
migrating
destroying
failed

---

## Authentication

Login providers:
- Google OIDC
- Microsoft OIDC

User creation occurs automatically on first login.

Access allowed if:

email_verified == true
AND
email domain OR tenant matches allowed configuration

---

## Resource Limits

Nodes enforce usage limits instead of priority control.

Examples:

CPU <= 70%
Memory <= 60%
Disk <= 80%
Max environments <= 8

Scheduler refuses placements exceeding limits.

---

## Lifecycle

create -> running
running -> suspend
suspended -> resume
running -> reboot
running -> force reboot
suspended -> migrate
destroy -> removed

---

## Identity

System users map to guest OS accounts.

Each OS has account templates.

SSH keys synchronize automatically.

---

## Future Work

distributed chunk storage
live migration
auto rebalancing
node auto discovery
