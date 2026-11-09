# KenShell User Guide

This guide explains how to use KenShell, from the first console cable to analysing a
switch's output. Shortcuts are written for macOS (`⌘`). On Windows and Linux, use `Ctrl`
where the guide says `⌘`.

> **Tip:** you rarely need to remember where something lives. Press **⌘K** to open the
> **command palette** and type what you want: "split", "record", "subnet", "notes" and so on.
> Every action is there, with its shortcut.

## Contents

1. [The window at a glance](#1-the-window-at-a-glance)
2. [Connecting](#2-connecting)
3. [Working in a session](#3-working-in-a-session)
4. [Notes](#4-notes)
5. [Snippets and templates](#5-snippets-and-templates)
6. [Analysing output](#6-analysing-output)
7. [Network scanner and topology](#7-network-scanner-and-topology)
8. [File transfer and file servers](#8-file-transfer-and-file-servers)
9. [Network tools](#9-network-tools)
10. [Logging, recording and export](#10-logging-recording-and-export)
11. [Credentials, keys and backups](#11-credentials-keys-and-backups)
12. [Settings](#12-settings)
13. [Keyboard shortcuts](#13-keyboard-shortcuts)
14. [Troubleshooting](#14-troubleshooting)

---

## 1. The window at a glance

| Area | What it does |
| --- | --- |
| **Top bar** | Sidebar toggle, the KenShell logo, **Quick connect**, **Scan my network**, the command palette and **Settings**. |
| **Sidebar** | **Search sessions**, your saved **Sessions** in folders, **Detected consoles** (USB cables plugged in right now) and **Recent** connections. |
| **Tab bar** | One tab per open session. Drag to reorder, or drag a tab onto a pane to split. |
| **Identity bar** | At the top of every session: CLI mode, environment tag (PROD / LAB / TEST), session name, protocol, target, connection state and uptime. Buttons on the right: notes bar, connection details, side panel. |
| **Mode stripe** | The coloured strip on the left edge of each session: grey = user, blue = privileged, amber = config, pink = boot, teal = shell. |
| **Side panel** | **Notes**, **Analyzer**, **Snippets** and **Tools** (⌘J). It floats over the terminal, or pin it to push the terminal aside. |
| **Status bar** | Logging state, received/sent bytes, charset, terminal size and short feedback messages. |

KenShell never interrupts you with "Are you sure?" dialogs. Feedback appears as a toast
(often with **Undo**) or in the status bar.

---

## 2. Connecting

### USB console cable (plug and connect)

1. Plug in the console cable. It appears under **Detected consoles** with its chip
   (e.g. *FTDI FT232R*), and a "USB console detected" toast appears.
2. Click **Connect**, or press **⌘N** for a new serial session.
3. KenShell **detects the speed automatically** ("Detecting speed…"). If it can't, it
   suggests a best guess and you can change it from the identity bar.

KenShell remembers the last speed and the last device seen on each cable. If the cable's chip
is recognised but its driver is missing (common with Prolific PL2303 on Windows), the
sidebar shows the **Official driver page** link. Install the driver and click
**Rescan after installing**.

### Quick connect (⌘L)

Type into the box in the top bar and press Enter:

| You type | KenShell opens |
| --- | --- |
| `ssh admin@10.0.0.1` | SSH as `admin` |
| `10.0.0.1` or `admin@switch.lab` | SSH (the default for a bare host) |
| `ssh admin@10.0.0.1:2222` | SSH on another port |
| `telnet 10.0.0.5 2001` or `10.0.0.5 2001` | Telnet (e.g. a terminal server port) |
| `raw://10.0.0.5:4001`, `rlogin://host` | Raw TCP, Rlogin |
| `COM4`, `com4 9600` or `/dev/cu.usbserial-A10K3XYZ 115200` | Serial port (optionally with a speed) |
| `core-sw1` | A saved session by name |

### Saved sessions

- **New:** **+** next to *Sessions* or **⌘N**. The editor shows the basics first; advanced
  options (keepalive, legacy SSH algorithms, jump host, proxy, port forwarding, X11, serial
  DTR/RTS, commands to run after login) are tucked away until you need them.
- **Organise:** nested folders, drag and drop, tags, favourites, duplicate, bulk edit
  (select several sessions).
- **Environment tag:** mark a session **PROD**, **LAB** or **TEST**. The tag shows on the
  tab and in the identity bar, so a production device is never mistaken for a lab one.
- **Import:** PuTTY sessions, SecureCRT, MobaXterm, or CSV. Use
  **Settings → Import / export**.

### SSH host keys

The first time you connect, the host key is remembered. If it later changes, the connection
is **blocked**: this is the one case where KenShell stops you. Check with whoever manages the
device; if the change is expected, use **Forget stored key** in the identity bar and connect
again.

### Reconnecting

If a session drops, press **Enter** in the terminal or **⌘⇧R** to reconnect. SSH sessions
can also reconnect automatically (see the session's advanced options).

---

## 3. Working in a session

### Tabs and split view

| Action | Shortcut |
| --- | --- |
| Next / previous tab | `Ctrl+Tab` / `Ctrl+Shift+Tab` |
| Go to tab 1–9 | ⌘1 … ⌘9 |
| Split right / split down | ⌘⇧E / ⌘⇧O |
| Single view | ⌘⇧X |
| Move focus between panes | ⌘⌥ + arrow keys |
| Move tab to a new window | ⌘⇧N |
| Close / reopen closed tab | ⌘W / ⌘⇧T |

### Terminal basics

- **Copy and paste:** selecting text copies it, and a right-click pastes. Both can be
  turned off in Settings.
- **Smart paste:** on serial and Telnet, pasted lines are sent with a short delay so the
  device doesn't drop characters. You can turn on a preview before multi-line pastes.
- **Find in scrollback** (⌘F): plain text or regex.
- **Send BREAK** (⌘⇧B): for serial password recovery or ROMMON.
- **Line timestamps:** add the time to each line (command palette → *timestamps*).

### Device awareness

- **Identification:** after login, KenShell identifies the device: hostname, device type (shown as Type A, Type B …), model
  and OS version, shown in the identity bar. **ⓘ** shows the full connection details: SSH
  algorithms, host key fingerprint and serial port settings.
- **Mode tracking:** the CLI mode is followed from the prompt, and the mode stripe changes
  colour as you move between user, enable and config mode.
- **`--More--` handling:** paging prompts are continued automatically when this is turned
  on.
- **Autocomplete:** commands you typed before on this device are suggested in faint text;
  press Tab or → to accept.
- **Output colouring:** up/down, errors, IP/MAC addresses and interface names are coloured.
  Each category can be switched off in **Settings → Terminal**.
- **Clickable items:** click an IP address or interface name for actions: ping it, show it
  in the network scanner, connect to it, or add a note for that interface.

### Broadcast

**⌘⌥B** adds the current tab to the broadcast group. Typing goes to every tab in the group,
and those tabs are clearly marked so you never type into several devices by accident.

---

## 4. Notes

KenShell has two views of the same notes. Once the device is identified, notes belong to the
**device**, so you see them whether you connect over SSH or console. Before that, they belong
to the saved session.

### Notes bar (under the session)

Click the **notes** icon in the identity bar, or press **⌘⌥N**. A bar opens under the
terminal:

- Type a note and press **Enter**. It is saved with the time, like a work log
  ("01:04 VLAN 30 added to trunk").
- The list under the input shows the session's notes, newest first. Collapse it with the
  arrow, or close the bar with **×**.
- **Esc** returns the keyboard to the terminal.
- Hover a note to delete it.

Notes need a saved session or an identified device. For a quick-connect session, save the
session first.

### Notes panel (side panel → Notes)

The full editor: Markdown, **pin** important notes (pinned notes appear under the session name
in the sidebar), edit, delete with **Undo**, and **export all notes as Markdown**.

**Interface notes:** a note that starts with an interface name, such as
`Gi1/0/24: camera in lobby`, is linked to that interface. Hover the interface name in the
terminal to see the note.

---

## 5. Snippets and templates

Side panel → **Snippets**. A searchable library of commands, filtered by device type.
Starter sets for common CLIs are included.

- **Send a snippet:** click it to send it to the active session.
- **Templates:** use variables such as `{{vlan}}` or `{{interface}}`. Before sending, you
  fill them in on a short form and see a preview.
- **Copy to my snippets:** copy a built-in snippet into your own library to edit it.

---

## 6. Analysing output

Side panel → **Analyzer**.

- **Parsed tables:** run a known `show` command (for example `show interfaces status`,
  `show ip interface brief`, `show vlan`, `show mac address-table`, `show cdp neighbors`) and
  the output turns into a table automatically. You can also select text and choose
  **Analyze selection**.
- **Working with tables:** sort, **Filter rows** and **Export CSV**.
- **Diagnostics:** a set of read-only checks for the supported device types. The findings are ranked
  by severity, and nothing is ever changed on the device.
- **Config audit:** checks a running config against best-practice rules.
- **Config backups:** take compressed backups and compare any two side by side.
- **File analysis:** open a `show tech`, crashinfo or log file. It is analysed as a stream,
  so large files are fine. Link flaps and errors are summarised.
- **Multi-device run:** send the same read-only command to several sessions and compare the
  results.

---

## 7. Network scanner and topology

**Scan my network** (top bar) finds devices on networks this computer is connected to.

- **Discovery only:** ping, the ARP table, and a connection attempt to TCP 22, 23, 80, 443
  and 3389. Hostnames come from reverse DNS, and vendor names from the built-in IEEE OUI
  table.
- **Scope:** only your own directly connected private subnets, at most a /22. You can enter
  a custom range within those limits.
- **Results:** a grid and a list with **Used** and **Probably free** addresses, which is
  handy when looking for a free IP. Right-click an address to connect, ping or copy.
  Export to CSV.

**Topology:** CDP/LLDP neighbours of the current device are drawn as a map. Click a
neighbour to connect to it.

---

## 8. File transfer and file servers

Side panel → **Tools**.

### File transfer (client)

This works over the active SSH tab:
- **SFTP:** browse folders, upload, download, rename, create a folder and delete.
- **SCP:** what most switches and routers offer. Enter the path on the
  device.

### File servers (TFTP, FTP, SCP)

Devices often need to *fetch* firmware or *upload* a config. Use **File servers** for this:

1. Choose a **folder**. Only that folder is served; nothing outside it.
2. Pick the server type, the interface to listen on and the port.
3. FTP and SCP get a **generated password** each time they start. SCP shows its **host key**
   fingerprint.
4. Uploads are off by default. Turn on **Allow uploads** (and optionally overwriting) when a
   device needs to copy files to you.
5. Click **Start**. The panel shows the exact address and an example command for the device
   ("On the device: …").

Transfers are listed with their progress. Stop the server when you're done.

---

## 9. Network tools

Side panel → **Tools → Network tools**.

- **Subnet calculator:** accepts `192.168.1.10/24`, `10.0.0.1 255.255.255.0` or a wildcard
  such as `10.0.0.0 0.0.0.255`. It shows the network, broadcast, mask, wildcard, host range,
  usable hosts and the binary mask.
- **MAC tool:** paste a MAC in any format (`aabb.ccdd.eeff`, `aa:bb:cc:dd:ee:ff`,
  `AA-BB-…`). It shows all formats and the vendor.
- **Ping and traceroute:** run from this computer with live results, packet loss and
  round-trip statistics.

---

## 10. Logging, recording and export

- **Logging:** turn on **Settings → Logging → Log every session**, or start logging for one
  tab from the command palette.
  - Choose the folder and a file-name template (default `{name}_{date}_{time}.log`).
  - Choose the content: **Readable text** or **Raw output (all bytes)**, and optionally a
    timestamp on each line.
  - Passwords you type are never written to the log.
- **Recording:** command palette → *Record*. The status bar shows **● REC** while it runs.
  Recordings are `.cast` files (asciicast v2). Open them with *Play recording*; the replay
  player supports play/pause with Space and seeking with the arrow keys.
- **Export output:** save the visible session output as **TXT** or as **coloured HTML**,
  ready to attach to a ticket.

---

## 11. Credentials, keys and backups

- **Saved passwords:** stored in the operating system's keychain (macOS Keychain, Windows
  Credential Manager, Secret Service on Linux), never in KenShell's database.
- **Credential profiles** (*Settings → Logins*): define a login once and reuse it across many
  sessions.
- **SSH agent:** supported on macOS (ssh-agent), Windows (OpenSSH agent and Pageant) and
  Linux.
- **SSH keys** (*Settings → SSH keys*): generate Ed25519 or RSA keys, and convert between
  PuTTY `.ppk` and OpenSSH formats.
- **Encrypted vault:** *Settings → Import / export* creates a `.ksvault` file with your
  sessions, folders, credential profiles, known SSH hosts and saved passwords, encrypted with
  a master password (Argon2id + ChaCha20-Poly1305). Move it to another computer and restore
  everything there. Key files are not included, and a lost master password cannot be
  recovered.
- **Plain exports** (JSON or CSV) never contain passwords.

---

## 12. Settings

Open with **⌘,**. Changes apply immediately.

| Section | Highlights |
| --- | --- |
| **Appearance** | Theme (dark, light, system), mode stripe width, mode colours per device type. |
| **Language** | English or Turkish. On first launch, the system language is used. |
| **Terminal** | Font, size, line spacing, cursor, output colouring categories, autocomplete, `--More--` handling, scrollback, copy on select, right-click paste, charset, line timestamps, renderer. |
| **Connection** | SSH defaults (keepalive, legacy algorithms), paste delay, identifying devices after login. |
| **Logins** | Credential profiles. |
| **SSH keys** | Key generator and converter. |
| **Logging** | Automatic logging, folder, file name, content. |
| **Console** | USB console and serial defaults. |
| **Shortcuts** | Change any shortcut: click a shortcut and press the new keys. Conflicts are shown, and you can reset to the default. |
| **Import / export** | Import from other apps, export sessions, encrypted backup and restore. |

**Updates:** when a new version is out, a small card appears at the top right of the
window with the new version number and what has changed. Click **Update now** to download
it (a progress bar shows how far along it is); KenShell restarts on the new version. Click
**Later** to hide the card; it asks again the next time you start KenShell, or check any
time from **Settings → Import / export → Updates**.

---

## 13. Keyboard shortcuts

Defaults (all can be changed in **Settings → Shortcuts**):

| Shortcut | Action |
| --- | --- |
| ⌘K | Command palette |
| ⌘L | Quick connect |
| ⌘N | New serial session |
| ⌘W | Close tab |
| ⌘⇧T | Reopen closed tab |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous tab |
| ⌘1 … ⌘9 | Go to tab |
| ⌘⇧PageUp / ⌘⇧PageDown | Move tab left / right |
| ⌘⇧E / ⌘⇧O | Split right / down |
| ⌘⇧X | Single view |
| ⌘⌥ + arrows | Move focus between panes |
| ⌘⇧N | Move tab to a new window |
| ⌘⌥B | Broadcast: add / remove this tab |
| ⌘⇧B | Send BREAK |
| ⌘⇧R | Reconnect |
| ⌘F | Find in scrollback |
| ⌘⇧F | Search sessions |
| ⌘B | Toggle sidebar |
| ⌘J | Toggle side panel |
| ⌘⌥N | Toggle notes bar |
| ⌘, | Settings |

---

## 14. Troubleshooting

| Problem | What to do |
| --- | --- |
| The console cable isn't listed | Check the sidebar for a driver hint and install the driver from the official page, then **Rescan**. On Linux, add your user to the `dialout` (or `uucp`) group and log in again; KenShell shows the exact command. |
| Garbled characters on the console | The speed is wrong. Change it from the identity bar (9600 is typical for most switches and routers), or let autobaud try again. |
| "Host key changed": connection blocked | The device's SSH key is different from last time. Confirm the change is expected, then use **Forget stored key** and reconnect. |
| Old device rejects SSH | Turn on **legacy SSH algorithms** for that session (advanced options). |
| Pasted config loses characters on serial | Increase the **paste line delay** in Settings → Connection. |
| A device can't reach the TFTP/FTP server | Check that the computer's firewall allows the port, and that you chose the interface facing the device under **Listen on**. |
| X11 programs don't open | You need an X server: XQuartz on macOS, or VcXsrv/X410 on Windows. Turn on **X11 forwarding** in the session's advanced options. |
