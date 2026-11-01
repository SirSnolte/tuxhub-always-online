# SPEC: tuxhub-always-online 0.1.0

Stand: 01.11.2026 · neues Repo `SirSnolte/tuxhub-always-online` (erst lokal) ·
Struktur verbindlich nach `../tuxhub-intern/TOOL-STRUCTURE.md`, Vorbilder
`tuxhub-push-to-mute` (Dienst, Live-Schnittstelle, Fenster) und `tuxhub-autoclicker`
(uinput, `setup`).

Leitlinie des Nutzers: **läuft auf allen gängigen Linux-Systemen ohne Probleme – weniger ist mehr.**
Deshalb nur Kernel-Mittel (uinput) und ein Timer: kein KWin, kein Wayland-Protokoll, kein D-Bus-Dienst
eines Desktops, keine neuen Abhängigkeiten gegenüber den anderen Tools.

## 1. Ziel

Discord, Steam, Teams & Co. setzen dich nach einiger Zeit ohne Eingabe auf „Abwesend“. Das Tool sendet
im Hintergrund in einem festen Abstand (Standard 4 Min.) einen winzigen, folgenlosen Eingabeimpuls über ein
virtuelles Gerät, damit die Leerlaufzeit des Systems nie die Schwelle der Programme erreicht – für alle
Programme gleichzeitig. Ein Wochenplan und eine Höchstdauer begrenzen, wann das gilt. Eigenständig
(Fenster + CLI), gleiche Oberfläche und gleicher Zustand in tuxhub.

## 2. Nicht-Ziele

- **Keine Programm-Bedingungen** („nur wenn Discord läuft“): Der Impuls setzt die Leerlaufzeit des ganzen
  Systems zurück, wirkt also immer auf alle Programme. Pro Programm geht das so nicht (Entscheidung 01.11.).
- **Kein Umdrehen-Modus** („trotz Anwesenheit als abwesend gelten“): später, erst nach Prüfung, was Steam
  (`steam://friends/status/away`) wirklich kann; Discord lässt sich lokal nicht auf Abwesend setzen. Config und
  Zeitplan sind so gebaut, dass ein zweiter Modus ohne Umbau dazukommen kann (Feld `mode` kommt erst dann).
- Keine Leerlauferkennung (`ext_idle_notifier_v1`), kein Lesen echter Eingaben: es wird auch gesendet, wenn
  du gerade aktiv bist. Deshalb ist der Impuls so gewählt, dass er nichts auslöst (siehe 5.2).
- Keine Pause bei gesperrtem Bildschirm (nicht gewählt). **Folge:** Solange Impulse laufen, greifen
  Bildschirmsperre und Standby nicht. Die Oberfläche weist darauf hin; Zeitplan und Höchstdauer sind der Ausweg.
- Kein Tray-Icon, keine Benachrichtigungen, kein Zufall im Abstand.

## 3. Dateien

Aufbau wie TOOL-STRUCTURE.md; Dateien aus Push-to-Mute kopiert (`service.rs`, `client.rs`, `paths.rs`,
`window.rs`, `bridge.js`, `integration.rs` angepasst), Neues:

| Datei | Inhalt |
|---|---|
| `src/config.rs` | `Config` (siehe 4), `load` (repariert Handarbeit still), `merged(patch)` (streng, lehnt unbekannte Schlüssel ab), atomares Schreiben |
| `src/schedule.rs` | reine Funktionen ohne Uhr-Zugriff: Wochenplan prüfen, „aktiv um T?“, „nächste Änderung ab T“ |
| `src/impulse.rs` | uinput-Gerät (`Impulse::new`, `send(kind)`), `setup_access()` (udev-Regel per pkexec, aus `tuxhub-autoclicker/src/device.rs`) |
| `src/engine.rs` | reine Zustandsmaschine: `decide(state, cfg, now) -> Decision {state, next_wake, send}`; kennt weder Uhr noch Gerät |
| `src/daemon.rs` | Dienst (dauerhaft, siehe 5.1), Live-Socket, eine Schleife über `mpsc` mit `recv_timeout`; ruft `engine` mit echter Uhr (`libc::localtime_r`) |
| `src/live.rs` | Schema (de/en), `values`, `set` für die Grundeinstellungen (siehe 6) |
| `src/main.rs` | clap: ohne Unterbefehl Fenster; `daemon`, `on`, `off`, `status`, `test`, `setup`, `install [--force]`, `uninstall [--purge]` |
| `ui/` | Web Awesome 3 + `theme.css` (vendored), Seite siehe 7 |
| `tuxhub.toml` | `id = "always-online"`, `[live]`, `[run]`, `[uninstall]`, `[[actions]] id = "toggle"` („Pause or resume always online“, kein Standard-Hotkey) |

Namen: Binary `tuxhub-always-online`, Tool-ID `always-online`, App-ID `io.github.sirsnolte.TuxhubAlwaysOnline`,
Geräte-Name `tuxhub-always-online`, Socket `$XDG_RUNTIME_DIR/tuxhub/always-online.sock`, Sperre
`$XDG_RUNTIME_DIR/tuxhub-always-online.lock`, Einstellungen `~/.config/tuxhub-always-online/config.json`.

## 4. Einstellungen (`Config`)

| Feld | Typ / Bereich | Standard | Bedeutung |
|---|---|---|---|
| `enabled` | bool | **false** | Hauptschalter |
| `interval_s` | 30 … 1800 | 240 | Abstand der Impulse |
| `impulse` | `"key"` \| `"mouse"` | `"key"` | Art des Impulses (5.2) |
| `max_hours` | 0 … 24 (ganze Zahl) | 0 (aus) | Höchstdauer eines Laufs (5.4) |
| `schedule_enabled` | bool | false | Wochenplan benutzen |
| `schedule` | genau 7 Einträge Mo … So | siehe unten | Zeitfenster je Wochentag |
| `start_at_login` | bool | true | Autostart-Eintrag des Dienstes |

`schedule`-Eintrag: `{"on": bool, "from": "HH:MM", "to": "HH:MM"}`. Standard für alle 7 Tage:
`on = true`, `08:00`–`23:00`. `from`/`to` sind lokale Zeit 00:00–23:59, `from != to`. Ist `from > to`, reicht das
Fenster über Mitternacht in den Folgetag (22:00–01:00). `on = false` heißt: an diesem Tag nie.
Das Ende ist ausgeschlossen (08:00–23:00 gilt bis 22:59), mit einer Ausnahme: `to = 23:59` heißt „bis Tagesende“,
damit 00:00–23:59 ohne Lücke um Mitternacht durchläuft.
Unbekannte Schlüssel, falsche Typen, Werte außerhalb des Bereichs → Fehler mit Feldnamen; eine kaputte
Datei auf der Platte wird beim Laden still auf Standardwerte je Feld repariert (wie Push-to-Mute).

## 5. Verhalten

### 5.1 Dienst

Dauerhaft im Hintergrund (Lebensdauer „dauerhaft“ in `daemon.rs` dokumentieren), Login-Start über
`~/.config/autostart/` solange `start_at_login`. Nach der Installation `enabled = false`: Dienst läuft,
sendet nichts, erzeugt kein Gerät. Nach einem Update startet er sich über den gemerkten Startpfad neu.
Ein Dienst pro Nutzer (flock), Fenster, tuxhub und CLI sind Clients desselben Sockets.

### 5.2 Impuls

- `key`: `KEY_F15` drücken und loslassen (zwei Ereignisse, je mit SYN). F15 hat auf Standard-Tastaturen
  keine Taste und in KDE/GNOME/gängigen Spielen keine Belegung.
- `mouse`: `REL_X +1`, nach 50 ms `REL_X −1` (Netto 0).
- Das virtuelle Gerät hat genau die Fähigkeiten, die der gewählte Impuls braucht (`key`: `KEY_F15`; `mouse`:
  `REL_X`, `REL_Y`), damit libinput es als Tastatur bzw. Zeiger erkennt. Es wird beim Eintritt in den Zustand
  *Sendet* erzeugt und beim Verlassen entfernt; nach dem Erzeugen vor dem ersten Ereignis 500 ms warten, damit der
  Compositor es kennt. Wechselt `impulse` im laufenden Betrieb, wird das Gerät neu erzeugt.
- Kein Zugriff auf `/dev/uinput` → Zustand *Kein Zugriff* mit Text; `tuxhub-always-online setup` (bzw. Knopf)
  richtet die udev-Regel per pkexec ein (Regel über stdin, nicht in der Shell zusammengesetzt, wie im Autoclicker).
  Ohne polkit steht die Regel zum Selbsteintragen in der README.
- `test` (CLI, Knopf, Live-Methode `impulse.test`): erzeugt bei Bedarf das Gerät und sendet genau einen Impuls,
  auch bei `enabled = false` und außerhalb des Plans. Antwort: Erfolg oder Fehlertext.

### 5.3 Zustände (`engine.rs`)

| Zustand | wann |
|---|---|
| `off` | `enabled = false` |
| `outside` | `enabled`, `schedule_enabled`, aktuell außerhalb eines Fensters (oder Tag aus) |
| `paused` | `enabled`, im Fenster, aber Höchstdauer des Laufs erreicht |
| `sending` | `enabled`, im Fenster (oder kein Plan), Höchstdauer nicht erreicht |
| `no_access` | `sending` wäre fällig, aber das Gerät lässt sich nicht erzeugen oder ein Impuls schlägt fehl |

Im Zustand `sending` kommt der erste Impuls sofort beim Eintritt, danach alle `interval_s` (Monotonuhr).
`next_wake` = früheste von: nächster Impuls, nächste Planänderung, Höchstdauer; die Schleife schläft aber
höchstens 30 s am Stück und prüft dann die Uhrzeit neu (Zeitumstellung, NTP, Standby überstehen). Ein
überfälliger Impuls (z. B. nach Standby) wird sofort gesendet, nie nachgeholt.

### 5.4 Lauf und Höchstdauer

Ein *Lauf* beginnt, wenn `sending` betreten wird, und endet beim Verlassen in `off` oder `outside`.
`max_hours > 0`: Ist der Lauf `max_hours` Stunden (Wanduhr-Zeit) im Zustand `sending`, wechselt der Dienst nach
`paused`. `paused` endet nur durch einen neuen Lauf: Schalter aus und wieder an, oder das Fenster des Plans
endet und das nächste beginnt. Änderung von `max_hours` im Betrieb gilt sofort für den laufenden Lauf.

### 5.5 Anderes

- `on` / `off` (CLI, Aktion `toggle`) setzen `enabled`; `status` gibt Zustand, Countdown, Zähler aus.
- Jede Änderung (auch von einem anderen Client) geht per `changed` bzw. `config` und `status` an alle Clients.
- Beenden (`quit`, SIGTERM/SIGINT): Gerät entfernen, Socket löschen, Lock freigeben.

## 6. Live-Schnittstelle (Protokoll 1)

Pflichtmethoden `hello`, `schema {locale}`, `get`, `set {key,value}`, `quit` wie in TOOL-STRUCTURE.md.
Das Schema enthält nur: `enabled`, `interval_s` (number, Einheit s, Schritt 10), `impulse` (select),
`max_hours` (number, 0 = aus), `start_at_login`; Aktionen `toggle` und `test`. Wochenplan und Zugriffs-Setup
bleiben in der eigenen UI. Eigene Methoden: `config.get`, `config.set {patch}` (strenge `merged`-Prüfung, Fehler
mit `field`), `status`, `impulse.test`, `access.setup`. `status` liefert
`{state, next_in_s | null, run_elapsed_s | null, sent: <Zähler seit Dienststart>, last_error | null}`. Der Dienst
sendet `status` bei jedem Zustandswechsel und nach jedem Impuls, nicht jede Sekunde; die Oberfläche zählt den
Countdown selbst herunter.

## 7. Oberfläche (`ui/`, eine für Fenster und Hub)

Von oben nach unten:

1. **Statuskarte:** großer Schalter „Online halten“; darunter Zustand in einem Satz („Sendet – nächster Impuls in
   2:41“, „Aus“, „Außerhalb des Zeitplans – wieder ab Mo 08:00“, „Pausiert – Höchstdauer von 4 h erreicht“,
   „Kein Zugriff auf /dev/uinput“ mit Knopf „Zugriff einrichten“).
2. **Wochenplan:** Schalter „Zeitplan verwenden“; je Wochentag eine Zeile mit Schalter, Von/Bis (Zeitfelder) und einer
   24-h-Leiste, die das Fenster zeigt; Schnellknöpfe „Alle gleich wie …“ (Mo übernehmen), „Nur Wochentage“,
   „Täglich 24 h“ (setzt `00:00`–`23:59`). Heute ist hervorgehoben. Ohne Zeitplan ausgegraut.
3. **Weitere Einstellungen:** Abstand, Impulsart (mit Satz zur Wirkung), Höchstdauer, „Bei Anmeldung starten“.
4. **Test:** Knopf „Impuls jetzt senden“ mit Ergebnis.
5. **Hinweise (immer sichtbar, kurz):** Bildschirmsperre und Standby greifen nicht, solange Impulse laufen;
   bei Spielen mit Anti-Cheat Zeitplan nutzen oder abschalten.

Nur Web-Awesome-Bauteile, nur `--wa-*`-Tokens, de/en, Mindestbreite 480 px, hell/dunkel. `app-design-defaults`
vor dem Bau laden und die Checkliste abhaken.

## 8. Randfälle (je ein Test)

`schedule.rs` / `engine.rs` (reine Tests mit festen Zeitpunkten):

1. Fenster 08:00–23:00: 07:59 außen, 08:00 innen, 22:59 innen, 23:00 außen.
2. Fenster über Mitternacht 22:00–01:00 am Mo: Mo 23:30 innen, Di 00:30 innen, Di 01:00 außen; Di-Eintrag aus
   beeinflusst Di 00:30 nicht (gehört zum Mo-Fenster).
3. Tag mit `on = false` → nie aktiv, auch wenn der Vortag nach Mitternacht hineinreicht, nur das Vortags-Fenster zählt.
4. `schedule_enabled = false` → immer innen, `schedule` ignoriert (auch ein ungültiger Inhalt auf der Platte).
5. `from == to`, `from` oder `to` ungültig („24:00“, „8:00“, „ab:cd“), nicht genau 7 Tage → Fehler mit Feld.
6. `enabled = false` → `off`, kein Impuls, `next_wake` leer.
7. Eintritt in `sending` → sofortiger Impuls, dann alle `interval_s`; `interval_s` ändern im Lauf plant neu ab dem
   letzten Impuls.
8. Überfälliger Impuls (Uhr springt 10 Min. vor) → genau ein Impuls, kein Nachholen.
9. `max_hours = 1`: nach 1 h `paused`, kein Impuls; Schalter aus/an → neuer Lauf, sendet wieder.
10. `max_hours` im Lauf auf 0 gesetzt → `paused` endet, `sending` weiter; auf kleineren Wert gesetzt, den der Lauf
    schon überschritten hat → sofort `paused`.
11. Planfenster endet während `paused` und beginnt neu → neuer Lauf, `sending`.
12. Zeitumstellung (lokale Zeit springt eine Stunde): Fensterprüfung nutzt immer die aktuelle lokale Zeit,
    kein Absturz, höchstens 30 s Verzögerung.
13. Schlafen nie länger als 30 s, auch wenn `next_wake` weiter weg liegt.

`config.rs` / `live.rs` / Dienst (Prozess-Tests wie `tests/service.rs`):

14. Alte oder handgeschriebene `config.json` mit Unsinn → Standardwerte je Feld, Dienst startet.
15. `set` mit falschem Typ, Bereich, unbekanntem Schlüssel → Fehler mit `field`, Zustand unverändert.
16. Schema vollständig in de/en, Standardwerte = `Config::default()`.
17. Zweiter Dienststart, toter Socket, fremde Datei am Socket-Pfad, überlange Zeile, `quit` (wie TOOL-STRUCTURE.md).
18. `changed` geht an die anderen Verbindungen, nicht an den Absender; `status` an alle.
19. Gerät: Impulsart `key` erzeugt nur `KEY_F15` (Down/Up + SYN), `mouse` nur `REL_X +1/−1`; Wechsel der Art im Lauf
    erzeugt das Gerät neu; nach `off` existiert kein Gerät mehr (`/dev/input` ohne `tuxhub-always-online`).
20. Kein Zugriff auf uinput (Test mit unlesbarem Pfad) → `no_access` mit Text, kein Absturz, Dienst läuft.
21. `install`/`uninstall` mit eigenem HOME/XDG: Starter, Icon, Anmeldung `apps/always-online.toml`, Autostart nur bei
    `start_at_login`; `--purge` entfernt `config.json`; bricht ab, wenn tuxhub das Tool schon verwaltet (außer `--force`).
22. UI 480 px und breit, hell und dunkel, im Fenster und im Hub: alle Zustände aus 7.1, Wochenplan mit Über-
    Mitternacht-Fenster, ausgegrauter Plan, Zugriffs-Banner.

## 9. Sicherheit

- Externe Eingaben: `config.json` (Handarbeit möglich), Live-Socket (0600, Ordner 0700, Zeilen ≤ 1 MiB, nur
  JSON-Objekte), CLI-Argumente. Alles läuft durch `merged`/`set` mit festen Bereichen und Aufzählungen; Zeiten
  werden mit fester Form `HH:MM` geparst (kein `strptime`).
- Das virtuelle Gerät kann nur die beiden festen Impulse erzeugen; es gibt keinen Weg, beliebige Tasten über den
  Socket zu schicken. `impulse.test` sendet nur den gewählten Impuls.
- `setup`: pkexec mit festem Befehl, Regeltext über stdin; keine Eingabe fließt in den Befehl.
- Keine Shell-Befehle aus Eingaben. Keine Secrets. Autostart-/Starter-Dateien nur in eigenen XDG-Pfaden (absolute Pfade).
- **Keine neuen Abhängigkeiten** gegenüber den Schwesterprojekten: `evdev 0.13`, `serde`, `serde_json`, `clap 4`,
  `libc`, `tao 0.37`, `wry 0.57` (und was `window.rs` aus Push-to-Mute schon nutzt). Zeit über `libc::localtime_r`,
  kein `chrono`. Die zuvor besprochenen Pakete `wayland-client`/`wayland-protocols` werden nicht aufgenommen.

## 10. Ende-zu-Ende

Immer isoliert: eigenes `XDG_CONFIG_HOME`, `XDG_RUNTIME_DIR` unter `/run/user/1000`, eigener `HOME`; das virtuelle
Gerät wird beim Prüfen mit `evtest --grab` (EVIOCGRAB) auf seinem Knoten gegriffen, damit **kein Ereignis den
Desktop erreicht**.

1. `cargo test -- --test-threads=1`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, UI-Skripttests falls vorhanden.
2. Gerät: `tuxhub-always-online daemon`, `on` mit `interval_s = 30`; `evtest --grab` auf dem Knoten von
   `tuxhub-always-online` zeigt bei `key` alle 30 s `KEY_F15` 1/0, bei `mouse` `REL_X 1` und `REL_X -1`; nach `off`
   ist der Knoten weg. Ausgabe in den Bericht.
3. Plan: `status` mit einem Fenster, das gerade nicht gilt, zeigt `outside` und die Zeit bis zum Beginn. Die
   Höchstdauer (Stunden) wird nicht live gewartet, sondern über die Testuhr in Randfall 9 bis 11 belegt.
4. Oberfläche: Screenshots (`kwin_wayland --virtual`, Test-Settings-Portal) im eigenen Fenster und im Hub, hell
   und dunkel, bei 480 px: Statuskarte in jedem Zustand, Wochenplan, Zugriffs-Banner.
5. **Die eigentliche Frage (vom Nutzer am echten Desktop):** Kontrolle ohne Tool: 15 Min. nichts tun, Discord/Steam
   gehen auf „Abwesend“ (Status vom zweiten Gerät aus). Dann mit `key`, danach mit `mouse`, je 15–25 Min.: bleibt
   der Status „Online“? Ergebnis in `../tuxhub-intern/PLAN.md` festhalten und den Standard von `impulse`
   entsprechend setzen. Hält keiner der beiden Impulse Discord online, ist die Annahme des Tools widerlegt – dann
   stoppen wir und überlegen neu, statt weiterzubauen.
