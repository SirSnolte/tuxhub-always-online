<p align="center">
  <img src="assets/icon.svg" width="96" height="96" alt="Always Online icon">
</p>

<h1 align="center">Always Online</h1>

<p align="center">
  Stay online in Discord, Steam and other apps while you are at the computer.
</p>

Apps set you to "away" when you have not touched the keyboard or mouse for a
while, even if you are watching a video, reading on a second screen or waiting
for a game to load. Always Online prevents that: at a fixed interval it sends
one harmless input event, so the idle time of your session starts again before
the away time of your apps is reached. It works for every app at once; there is
nothing to set up per app.

Always Online is part of the [tuxhub](https://github.com/SirSnolte/tuxhub)
family of gaming tools for Linux. It works on its own and integrates with
tuxhub, whichever is installed first.

## Features

- One switch and one interval (4 minutes by default) for all apps.
- A weekly schedule: one window per weekday, windows over midnight, with a
  24-hour bar for each day. Outside the windows nothing is sent.
- A longest run in hours, after which it pauses.
- Two impulses: the key F15 (which common keyboards do not have and nothing
  uses), or the pointer moved one unit to the right and back.
- Runs in the background from login, without a window and without tuxhub. It
  stays off until you switch it on.
- No desktop-specific parts: the impulse is a normal input event from the
  kernel, so it works on Wayland and X11 with any desktop.

## Installation

### With tuxhub

Install Always Online from the workshop in tuxhub (version 0.7.0 or newer).
tuxhub takes care of updates, starts the background service when needed and
shows the interface on its own page.

### Without tuxhub

Download the archive from the
[latest release](https://github.com/SirSnolte/tuxhub-always-online/releases/latest),
extract it and run the installer from the extracted folder:

```bash
tar -xf tuxhub-always-online-x86_64-linux.tar.gz
./tuxhub-always-online/tuxhub-always-online install
```

This installs Always Online for your user into
`~/.local/share/tuxhub-always-online`, adds it to the application menu and
starts the service at login. No administrator rights are needed for that. If
you install tuxhub later, it detects the installation and integrates it,
including updates.

To remove it again:

```bash
tuxhub-always-online uninstall           # keeps the settings
tuxhub-always-online uninstall --purge   # also removes the settings
```

### Access to /dev/uinput

Sending the impulse needs write access to `/dev/uinput`. If your user does not
have it, the interface says so and offers **Set up access**, which asks for
your password once and writes a udev rule (`60-tuxhub-uinput.rules`, the same
rule the other tuxhub tools use). On the command line:

```bash
tuxhub-always-online setup
```

Without polkit, create `/etc/udev/rules.d/60-tuxhub-uinput.rules` yourself:

```
KERNEL=="uinput", SUBSYSTEM=="misc", TAG+="uaccess", OPTIONS+="static_node=uinput"
```

and run `sudo modprobe uinput && sudo udevadm control --reload-rules && sudo udevadm trigger --name-match=uinput`.

## Usage

1. Open Always Online from the application menu or in tuxhub.
2. Switch **Keep me online** on.
3. Optionally switch on the **Schedule** and set the windows, or set a longest
   run.

**Send an impulse** tests that access and the impulse work, whatever the
switch and the schedule say.

### Command line

```bash
tuxhub-always-online status    # state, next impulse, schedule
tuxhub-always-online on        # start keeping you online
tuxhub-always-online off
tuxhub-always-online test      # send one impulse now
```

The schedule and the other options are set in the window or in tuxhub; they
are stored in `~/.config/tuxhub-always-online/config.json`.

## Good to know

- **Screen lock and sleep.** An input event also resets the timers of the
  screen lock and of automatic sleep. While impulses are sent, your computer
  does not lock and does not go to sleep. Use the schedule or the longest run
  to limit that, or switch the tool off.
- **Games with anti-cheat.** The impulse comes from a virtual input device.
  If a game with anti-cheat should not see such a device, use the schedule or
  switch the tool off while you play.
- **Whether an app notices the impulse** depends on how it measures idle time.
  Apps that read the idle time of the session see it. How a particular app
  measures it, for example Discord in a Flatpak running through XWayland, can
  differ between systems; the first release has not been confirmed with every
  app yet. Please report what you find.
- It cannot make you appear away: apps that are set to "away" by you or by
  something other than idle time are not affected.

## How it works

The service creates a virtual keyboard or mouse in `/dev/uinput` while it is
sending and removes it when it stops. Every interval it sends one impulse and
waits for the next, using a timer: the service sleeps in between and uses
practically no CPU. The settings live in the service; the window, tuxhub and
the command line talk to it over the socket of the tuxhub live interface.

## License

MIT, see [LICENSE](LICENSE). The interface uses
[Web Awesome](https://webawesome.com) (MIT), bundled in `ui/vendor`.
