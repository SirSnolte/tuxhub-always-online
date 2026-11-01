# Changelog

All notable changes are documented in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [0.1.0] - Unreleased

Requires tuxhub 0.7.0 or newer to be used inside tuxhub.

### Added

- Background service that sends one harmless input event from a virtual
  device (`/dev/uinput`) at a fixed interval, so the idle time of the session
  does not reach the away time of Discord, Steam and other apps. Off after
  the installation until it is switched on.
- Two impulses: the key F15 pressed and released, or the pointer moved one unit
  and back.
- Weekly schedule with one window per weekday, windows over midnight, and a
  longest run in hours.
- Window and tuxhub page with the same interface, live interface of tuxhub
  (socket `$XDG_RUNTIME_DIR/tuxhub/always-online.sock`), command line
  (`status`, `on`, `off`, `test`, `setup`), `install` and `uninstall`.
