// Interface of Always Online. It runs inside tuxhub and in the tool's own
// window; both provide window.tuxhub. All state lives in the background
// service, which notifies every open interface about changes.

"use strict";

const STRINGS = {
  en: {
    enabled: "Keep me online",
    "state.offline": "Connecting",
    "state.offline.detail": "Waiting for the Always Online service.",
    "state.off": "Off",
    "state.off.detail": "Apps set you to away after their own idle time.",
    "state.sending": "Keeping you online",
    "state.sending.next": "Next impulse in {time}.",
    "state.sending.now": "Sending the next impulse.",
    "state.sending.until": "Schedule: until {time}.",
    "state.sending.run": "Running for {time}, pauses after {max} h.",
    "state.outside": "Outside the schedule",
    "state.outside.detail": "Back on {when}.",
    "state.outside.never": "No window is switched on in the schedule.",
    "state.paused": "Paused",
    "state.paused.detail":
      "The longest run of {max} h is over. Switch off and on again, or wait for the next window of the schedule.",
    "state.no_access": "No access to /dev/uinput",
    "state.no_access.detail": "No impulse can be sent until access is set up.",
    "problem.no-access":
      "Sending impulses needs access to /dev/uinput, once. Setting it up asks for your password and adds a rule for your user session.",
    "problem.setup": "Set up access",
    "problem.setupFailed": "Setup failed: {error}",
    "schedule.title": "Schedule",
    "schedule.use": "Use schedule",
    "schedule.help.on": "Impulses are only sent inside the windows. A window can reach past midnight.",
    "schedule.help.off": "Without a schedule, impulses are sent whenever the switch is on.",
    "schedule.copyMonday": "Monday for all days",
    "schedule.weekdays": "Weekdays only",
    "schedule.allDay": "All day, every day",
    "schedule.undo": "Undo",
    "schedule.today": "today",
    "day.on": "{day}: send impulses",
    "day.from": "{day}: from",
    "day.to": "{day}: until",
    "day.bar": "{from} to {to}",
    "day.bar.over": "{from} to {to} the next day",
    "day.bar.off": "No window",
    "day.same": "Start and end must be different times.",
    "day.time": "Enter a time as hours and minutes.",
    "options.title": "Options",
    "interval.label": "Interval",
    "interval.help": "Time between two impulses, now {time}. Keep it below the shortest away time of your apps.",
    "impulse.label": "Impulse",
    "impulse.key": "Key F15",
    "impulse.mouse": "Mouse, one step and back",
    "impulse.help.key": "Presses and releases F15, a key that common keyboards do not have and nothing uses.",
    "impulse.help.mouse": "Moves the pointer one unit to the right and back, which is hardly visible.",
    "max.label": "Longest run",
    "max.help": "Pauses after this many hours. 0 means no limit.",
    "login.label": "Start at login",
    "login.help": "Starts the background service when you log in.",
    "test.title": "Test",
    "test.help": "Sends one impulse now, whatever the switch and the schedule say.",
    "test.button": "Send an impulse",
    "test.ok": "Impulse sent.",
    "hint.lock": "While impulses are sent, the screen does not lock and the computer does not go to sleep.",
    "hint.cheat": "In games with anti-cheat, use the schedule or switch the tool off.",
    footer: "Always Online {version}",
  },
  de: {
    enabled: "Online halten",
    "state.offline": "Verbinde",
    "state.offline.detail": "Warte auf den Always-Online-Dienst.",
    "state.off": "Aus",
    "state.off.detail": "Programme setzen dich nach ihrer eigenen Leerlaufzeit auf Abwesend.",
    "state.sending": "Du bleibst online",
    "state.sending.next": "Nächster Impuls in {time}.",
    "state.sending.now": "Sendet gleich den nächsten Impuls.",
    "state.sending.until": "Zeitplan: bis {time} Uhr.",
    "state.sending.run": "Läuft seit {time}, pausiert nach {max} h.",
    "state.outside": "Außerhalb des Zeitplans",
    "state.outside.detail": "Wieder ab {when}.",
    "state.outside.never": "Im Zeitplan ist kein Fenster eingeschaltet.",
    "state.paused": "Pausiert",
    "state.paused.detail":
      "Die längste Laufzeit von {max} h ist vorbei. Schalte aus und wieder an, oder warte auf das nächste Fenster im Zeitplan.",
    "state.no_access": "Kein Zugriff auf /dev/uinput",
    "state.no_access.detail": "Es kann kein Impuls gesendet werden, bis der Zugriff eingerichtet ist.",
    "problem.no-access":
      "Zum Senden der Impulse braucht das Tool einmalig Zugriff auf /dev/uinput. Das Einrichten fragt nach deinem Passwort und legt eine Regel für deine Sitzung an.",
    "problem.setup": "Zugriff einrichten",
    "problem.setupFailed": "Einrichten fehlgeschlagen: {error}",
    "schedule.title": "Zeitplan",
    "schedule.use": "Zeitplan verwenden",
    "schedule.help.on":
      "Impulse werden nur innerhalb der Fenster gesendet. Ein Fenster kann über Mitternacht hinausgehen.",
    "schedule.help.off": "Ohne Zeitplan werden Impulse gesendet, solange der Schalter an ist.",
    "schedule.copyMonday": "Montag für alle Tage",
    "schedule.weekdays": "Nur Wochentage",
    "schedule.allDay": "Täglich ganztägig",
    "schedule.undo": "Rückgängig",
    "schedule.today": "heute",
    "day.on": "{day}: Impulse senden",
    "day.from": "{day}: von",
    "day.to": "{day}: bis",
    "day.bar": "{from} bis {to}",
    "day.bar.over": "{from} bis {to} am Folgetag",
    "day.bar.off": "Kein Fenster",
    "day.same": "Start und Ende müssen verschiedene Uhrzeiten sein.",
    "day.time": "Gib eine Uhrzeit mit Stunden und Minuten an.",
    "options.title": "Optionen",
    "interval.label": "Abstand",
    "interval.help":
      "Zeit zwischen zwei Impulsen, jetzt {time}. Wähle weniger als die kürzeste Abwesend-Zeit deiner Programme.",
    "impulse.label": "Impuls",
    "impulse.key": "Taste F15",
    "impulse.mouse": "Maus, ein Schritt und zurück",
    "impulse.help.key": "Drückt und löst F15, eine Taste, die übliche Tastaturen nicht haben und die nichts belegt.",
    "impulse.help.mouse": "Bewegt den Zeiger eine Einheit nach rechts und zurück, kaum sichtbar.",
    "max.label": "Längste Laufzeit",
    "max.help": "Pausiert nach so vielen Stunden. 0 heißt ohne Grenze.",
    "login.label": "Bei der Anmeldung starten",
    "login.help": "Startet den Hintergrunddienst, wenn du dich anmeldest.",
    "test.title": "Test",
    "test.help": "Sendet jetzt einen Impuls, egal was Schalter und Zeitplan sagen.",
    "test.button": "Impuls senden",
    "test.ok": "Impuls gesendet.",
    "hint.lock": "Solange Impulse gesendet werden, sperrt sich der Bildschirm nicht und der PC geht nicht in Standby.",
    "hint.cheat": "In Spielen mit Anti-Cheat den Zeitplan nutzen oder das Tool ausschalten.",
    footer: "Always Online {version}",
  },
};

const DAYS = 7;
const $ = (id) => document.getElementById(id);

let locale = navigator.language?.startsWith("de") ? "de" : "en";
let config = null;
let svc = { state: "offline" };
/** When `svc` arrived, to count the times in it down. */
let statusAt = Date.now();
let setupError = null;
/** Windows with an invalid entry that is not sent: weekday index to the day as typed. */
const drafts = new Map();
const dayErrors = new Map();
/** The schedule before the latest quick action, for undo. */
let undoSchedule = null;
let testTimer = null;

function t(key, vars = {}) {
  let s = STRINGS[locale][key] ?? STRINGS.en[key] ?? key;
  for (const [k, v] of Object.entries(vars)) s = s.replaceAll(`{${k}}`, String(v));
  return s;
}

function el(tag, props = {}, ...children) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (k === "class") e.className = v;
    else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
    else if (v !== undefined && v !== null) e.setAttribute(k, v);
  }
  for (const c of children) if (c !== null && c !== undefined) e.append(c);
  return e;
}

function showError(e) {
  $("error").textContent = e ? String(e.message ?? e) : "";
}

/** Applies a change. Resolves to true if the service accepted it. */
async function update(patch) {
  showError(null);
  let ok = true;
  try {
    config = await tuxhub.call("config.set", patch);
  } catch (e) {
    showError(e);
    ok = false;
  }
  render();
  return ok;
}

// ----------------------------------------------------------------- Helpers

/** `2:41` or `1:05:09` for a number of seconds. */
function clockText(total) {
  const s = Math.max(0, Math.floor(total));
  const [h, m, sec] = [Math.floor(s / 3600), Math.floor((s % 3600) / 60), s % 60];
  const pad = (n) => String(n).padStart(2, "0");
  return h ? `${h}:${pad(m)}:${pad(sec)}` : `${m}:${pad(sec)}`;
}

/** `4 min`, `45 s` or `1 h 5 min` for a number of seconds. */
function lengthText(total) {
  const s = Math.max(0, Math.round(total));
  const [h, m, sec] = [Math.floor(s / 3600), Math.floor((s % 3600) / 60), s % 60];
  if (h) return m ? `${h} h ${m} min` : `${h} h`;
  if (m) return sec ? `${m} min ${sec} s` : `${m} min`;
  return `${sec} s`;
}

function minutesOf(text) {
  const m = /^(\d\d):(\d\d)$/.exec(text ?? "");
  return m ? Number(m[1]) * 60 + Number(m[2]) : null;
}

/** The time of day `minutes` after the svc arrived. */
function timeAfter(minutes) {
  // The service counts from the start of the current minute.
  const base = Math.floor(statusAt / 60000) * 60000;
  return new Date(base + minutes * 60000);
}

function timeText(date) {
  return new Intl.DateTimeFormat(locale, { hour: "2-digit", minute: "2-digit", hourCycle: "h23" }).format(date);
}

function whenText(date) {
  const sameDay = date.toDateString() === new Date().toDateString();
  if (sameDay) return timeText(date);
  const day = new Intl.DateTimeFormat(locale, { weekday: "short" }).format(date);
  return `${day} ${timeText(date)}`;
}

/** Name of weekday `i`, Monday first. */
function dayName(i) {
  // 1 January 2024 was a Monday.
  return new Intl.DateTimeFormat(locale, { weekday: "long" }).format(new Date(2024, 0, 1 + i));
}

function todayIndex() {
  return (new Date().getDay() + 6) % 7;
}

function dayError(d) {
  if (minutesOf(d.from) === null || minutesOf(d.to) === null) return t("day.time");
  if (d.from === d.to) return t("day.same");
  return null;
}

/** Parts of the 24-hour bar as `[start, end]` in minutes. */
function segments(d) {
  const from = minutesOf(d.from);
  let to = minutesOf(d.to);
  if (!d.on || from === null || to === null || from === to) return [];
  // 23:59 means the end of the day.
  if (to === 23 * 60 + 59) to = 24 * 60;
  return from < to
    ? [[from, to]]
    : [
        [from, 24 * 60],
        [0, to],
      ];
}

// ------------------------------------------------------------------ Render

function applyTexts() {
  document.documentElement.lang = locale;
  for (const n of document.querySelectorAll("[data-i18n]")) n.textContent = t(n.dataset.i18n);
}

function stateDetail(state) {
  const max = config?.max_hours ?? 0;
  if (state === "sending") {
    const parts = [];
    const left = svc.next_in_s - Math.floor((Date.now() - statusAt) / 1000);
    parts.push(left > 0 ? t("state.sending.next", { time: clockText(left) }) : t("state.sending.now"));
    if (svc.schedule_active && svc.schedule_change_in_min != null) {
      parts.push(t("state.sending.until", { time: timeText(timeAfter(svc.schedule_change_in_min)) }));
    }
    if (max > 0 && svc.run_elapsed_s != null) {
      const elapsed = svc.run_elapsed_s + (Date.now() - statusAt) / 1000;
      parts.push(t("state.sending.run", { time: lengthText(elapsed), max }));
    }
    return parts.join(" ");
  }
  if (state === "outside") {
    return svc.schedule_change_in_min == null
      ? t("state.outside.never")
      : t("state.outside.detail", { when: whenText(timeAfter(svc.schedule_change_in_min)) });
  }
  if (state === "paused") return t("state.paused.detail", { max });
  return STRINGS.en[`state.${state}.detail`] ? t(`state.${state}.detail`) : "";
}

function renderStatus() {
  const state = svc.state ?? "offline";
  $("hero").dataset.state = state;
  $("state-title").textContent = t(`state.${state}`);
  $("state-detail").textContent = stateDetail(state);
  $("enabled").checked = config?.enabled ?? false;
  $("enabled").disabled = state === "offline" || !config;

  let problem = null;
  if (setupError) problem = t("problem.setupFailed", { error: setupError });
  else if (state === "no_access") problem = t("problem.no-access");
  $("problem").classList.toggle("hidden", !problem);
  $("problem-text").textContent = problem ?? "";
  $("setup").classList.toggle("hidden", state !== "no_access" && !setupError);
  $("footer").textContent = svc.version ? t("footer", { version: svc.version }) : "";
}

function buildWeek() {
  const week = $("week");
  for (let i = 0; i < DAYS; i++) {
    const onChange = (patch) => () => editDay(i, patch());
    week.append(
      el(
        "div",
        { class: "day", "data-day": i },
        el("wa-switch", { class: "name", onchange: onChange(() => ({ on: week.children[i].querySelector(".name").checked })) }),
        el(
          "div",
          { class: "times" },
          el("wa-input", { class: "from", type: "time", onchange: onChange(() => ({ from: week.children[i].querySelector(".from").value })) }),
          el("span", { "aria-hidden": "true" }, "–"),
          el("wa-input", { class: "to", type: "time", onchange: onChange(() => ({ to: week.children[i].querySelector(".to").value })) }),
        ),
        el("div", { class: "bar", role: "img" }),
        el("p", { class: "day-error", role: "alert" }),
      ),
    );
  }
}

function shownDay(i) {
  return drafts.get(i) ?? config.schedule[i];
}

function renderWeek() {
  if (!config) return;
  const scheduled = config.schedule_enabled;
  $("schedule-enabled").checked = scheduled;
  $("schedule-help").textContent = t(scheduled ? "schedule.help.on" : "schedule.help.off");
  const week = $("week");
  if (scheduled) delete week.dataset.off;
  else week.dataset.off = "";
  for (const row of week.children) {
    const i = Number(row.dataset.day);
    const d = shownDay(i);
    const name = dayName(i);
    const label = row.querySelector(".name");
    label.textContent = i === todayIndex() ? `${name} (${t("schedule.today")})` : name;
    label.setAttribute("aria-label", t("day.on", { day: name }));
    label.checked = d.on;
    label.disabled = !scheduled;
    const [from, to] = [row.querySelector(".from"), row.querySelector(".to")];
    from.setAttribute("aria-label", t("day.from", { day: name }));
    to.setAttribute("aria-label", t("day.to", { day: name }));
    if (document.activeElement !== from) from.value = d.from;
    if (document.activeElement !== to) to.value = d.to;
    from.disabled = to.disabled = !scheduled || !d.on;
    if (i === todayIndex()) row.dataset.today = "";
    else delete row.dataset.today;

    const bar = row.querySelector(".bar");
    const parts = segments(d);
    bar.replaceChildren(
      ...parts.map(([a, b]) =>
        el("span", { class: "seg", style: `inset-inline-start:${(a / 1440) * 100}%;width:${((b - a) / 1440) * 100}%` }),
      ),
    );
    const over = minutesOf(d.from) > minutesOf(d.to) && minutesOf(d.to) !== 23 * 60 + 59;
    bar.setAttribute(
      "aria-label",
      parts.length ? t(over ? "day.bar.over" : "day.bar", { from: d.from, to: d.to }) : t("day.bar.off"),
    );
    row.querySelector(".day-error").textContent = dayErrors.get(i) ?? "";
  }
  for (const id of ["copy-monday", "weekdays", "all-day"]) $(id).disabled = !scheduled;
  $("undo").classList.toggle("hidden", !undoSchedule);
}

function renderOptions() {
  if (!config) return;
  if (document.activeElement !== $("interval")) $("interval").value = config.interval_s;
  $("interval-help").textContent = t("interval.help", { time: lengthText(config.interval_s) });
  $("impulse").value = config.impulse;
  $("impulse-help").textContent = t(`impulse.help.${config.impulse}`);
  if (document.activeElement !== $("max-hours")) $("max-hours").value = config.max_hours;
  $("login").checked = config.start_at_login;
}

function render() {
  applyTexts();
  renderStatus();
  renderWeek();
  renderOptions();
}

// ------------------------------------------------------------------ Events

async function editDay(i, patch) {
  if (!config) return;
  const day = { ...shownDay(i), ...patch };
  const error = dayError(day);
  if (error) {
    drafts.set(i, day);
    dayErrors.set(i, error);
    renderWeek();
    return;
  }
  drafts.delete(i);
  dayErrors.delete(i);
  const days = config.schedule.map((d, n) => (n === i ? day : d));
  await update({ schedule: days });
}

/** Replaces the whole week, and keeps the old one for undo. */
async function quickSchedule(make) {
  if (!config) return;
  const before = config.schedule;
  drafts.clear();
  dayErrors.clear();
  if (await update({ schedule: make(before) })) undoSchedule = before;
  renderWeek();
}

$("enabled").addEventListener("change", (e) => update({ enabled: e.target.checked }));
$("schedule-enabled").addEventListener("change", (e) => update({ schedule_enabled: e.target.checked }));
$("copy-monday").addEventListener("click", () =>
  quickSchedule((days) => days.map(() => ({ ...days[0] }))),
);
$("weekdays").addEventListener("click", () =>
  quickSchedule((days) => days.map((d, i) => ({ ...d, on: i < 5 }))),
);
$("all-day").addEventListener("click", () =>
  quickSchedule((days) => days.map(() => ({ on: true, from: "00:00", to: "23:59" }))),
);
$("undo").addEventListener("click", async () => {
  const back = undoSchedule;
  undoSchedule = null;
  if (back) await update({ schedule: back });
});

$("interval").addEventListener("change", (e) => {
  const v = Math.round(Number(e.target.value));
  if (Number.isFinite(v) && e.target.value !== "") update({ interval_s: Math.min(1800, Math.max(30, v)) });
  else render();
});
$("impulse").addEventListener("change", (e) => update({ impulse: e.target.value }));
$("max-hours").addEventListener("change", (e) => {
  const v = Math.round(Number(e.target.value));
  if (Number.isFinite(v) && e.target.value !== "") update({ max_hours: Math.min(24, Math.max(0, v)) });
  else render();
});
$("login").addEventListener("change", (e) => update({ start_at_login: e.target.checked }));

$("test").addEventListener("click", async () => {
  const [button, result] = [$("test"), $("test-result")];
  clearTimeout(testTimer);
  result.textContent = "";
  button.loading = true;
  try {
    await tuxhub.call("impulse.test");
    result.textContent = t("test.ok");
  } catch (e) {
    result.textContent = String(e.message ?? e);
  }
  button.loading = false;
  testTimer = setTimeout(() => (result.textContent = ""), 6000);
});

$("setup").addEventListener("click", async () => {
  setupError = null;
  try {
    await tuxhub.action("setup");
  } catch (e) {
    showError(e);
  }
});

tuxhub.on("status", (s) => {
  svc = s ?? { state: "offline" };
  statusAt = Date.now();
  renderStatus();
  if (svc.state === "offline") return;
  // After a reconnect the configuration may have changed.
  if (!config) load();
});
tuxhub.on("config", (c) => {
  config = c;
  render();
});
tuxhub.on("setup", ({ error }) => {
  setupError = error;
  renderStatus();
});
function applyTheme(dark) {
  document.documentElement.classList.toggle("wa-dark", dark);
  document.documentElement.classList.toggle("wa-light", !dark);
}

// The live connection of tuxhub ended (service quit or restarted after an
// update): show the state and connect again, which starts the service.
tuxhub.on("closed", () => {
  svc = { state: "offline" };
  config = null;
  renderStatus();
  setTimeout(load, 1000);
});
tuxhub.on("theme", ({ dark }) => applyTheme(dark));
tuxhub.on("locale", ({ locale: l }) => {
  locale = l === "de" ? "de" : "en";
  render();
});

async function load() {
  try {
    const [c, s] = await Promise.all([tuxhub.call("config.get"), tuxhub.call("status")]);
    config = c;
    svc = s;
    statusAt = Date.now();
    showError(null);
  } catch (e) {
    showError(e);
  }
  render();
}

// The countdown and the length of the run change every second; the service
// does not send them.
setInterval(() => {
  if (!document.hidden && svc.state === "sending") $("state-detail").textContent = stateDetail("sending");
}, 1000);

if (tuxhub.standalone) document.documentElement.dataset.standalone = "";
applyTheme(matchMedia("(prefers-color-scheme: dark)").matches);
buildWeek();
render();
load();
