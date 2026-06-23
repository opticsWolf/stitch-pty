# -*- coding: utf-8 -*-
"""stitch-pty terminal emulator — a feature-complete PySide6 terminal.

A real, daily-driver-grade terminal emulator built on the stitch-pty module.
It uses a custom QPainter cell-grid renderer (not an HTML text widget), which
makes accurate selection, cursor shapes, true-color rendering and high
throughput possible.

Features
--------
Rendering
  • Custom cell-grid renderer (QPainter) — fast, pixel-accurate
  • 16-color / 256-color / 24-bit true-color, bold / dim / italic /
    underline / strikethrough / reverse / hidden
  • Block / underline / bar cursor shapes (honours DECSCUSR + DECTCEM),
    cursor blink, hollow cursor when unfocused
  • Switchable color themes
  • Visual + audible bell

Input
  • Complete keyboard map: arrows, F1–F12, Home/End/PgUp/PgDn/Ins/Del,
    application-cursor-key mode (DECCKM), xterm modifier encoding,
    Ctrl-letters, Alt-as-Meta (ESC prefix), Shift-Tab
  • Mouse reporting to apps (X10/normal/button/any-event + SGR 1006)
  • Bracketed-paste support (DEC 2004)

Selection & clipboard
  • Click-drag selection, double-click word, triple-click line,
    Alt-drag rectangular/block selection, Select-All
  • Copy (Ctrl+Shift+C / context menu / optional copy-on-select)
  • Paste (Ctrl+Shift+V / middle-click primary selection / context menu)
  • Ctrl/⌘-click to open URLs (auto-detected + underlined on hover)

Scrollback
  • Scrollbar + mouse-wheel, Shift+PgUp / Shift+PgDn
  • Snaps to bottom on keypress; in alt-screen apps the wheel becomes arrows
  • Incremental find with match highlighting (Ctrl+Shift+F, F3 / Shift+F3)

Window & sessions
  • Tabs (Ctrl+Shift+T new, Ctrl+Shift+W close, Ctrl+Tab cycle)
  • Per-tab title from OSC sequences
  • Font zoom (Ctrl+= / Ctrl+- / Ctrl+0, Ctrl+wheel)
  • Reset terminal, restart process, interrupt, kill, save output to file
  • Settings persisted via QSettings across launches
  • Status bar: cursor · title · history · grid size · process state

Usage
    pip install PySide6 stitch-pty
    python terminal_emulator.py                 # default shell
    python terminal_emulator.py --cmd "htop"    # run a command
    python terminal_emulator.py --rows 40 --cols 120

Threading model
    Each tab owns a background QThread running an asyncio loop and a Rust
    PtySession.  All PyO3/Rust state lives in that thread.  The GUI thread
    only ever sees plain-Python Frame snapshots delivered through Qt signals,
    and pushes input back via call_soon_threadsafe.  Nothing Rust-owned ever
    crosses the thread boundary.
"""

from __future__ import annotations

import argparse
import asyncio
import os
import platform
import re
import shutil
import signal
import sys
import time
from dataclasses import dataclass
from pathlib import Path

from PySide6.QtCore import (
    QObject, Qt, QThread, QTimer, Signal, QRect, QPoint, QSize, QEvent,
    QSettings, QUrl,
)
from PySide6.QtGui import (
    QAction, QActionGroup, QFont, QFontMetricsF, QKeySequence, QPainter,
    QColor, QPen, QBrush, QClipboard, QDesktopServices, QShortcut, QKeyEvent,
)
from PySide6.QtWidgets import (
    QApplication, QMainWindow, QWidget, QScrollBar, QHBoxLayout, QVBoxLayout,
    QStatusBar, QTabWidget, QLineEdit, QToolButton, QLabel, QMenu, QFileDialog,
    QSizePolicy,
)

from stitch_pty import PtyError, PtySession, Winsize, spawn


IS_WINDOWS = platform.system() == "Windows"
IS_MAC = platform.system() == "Darwin"

# On macOS the platform "command" modifier maps to Qt's ControlModifier and the
# physical Ctrl key maps to MetaModifier.  We treat ⌘ as the GUI/shortcut
# modifier and the physical Ctrl as the terminal control modifier.


# ─────────────────────────────────────────────────────────────────────
# Color themes
# ─────────────────────────────────────────────────────────────────────

@dataclass(frozen=True)
class Theme:
    name: str
    bg: str
    fg: str
    cursor: str
    sel_bg: str
    sel_fg: str | None       # None → keep cell fg
    match_bg: str
    match_cur_bg: str
    # 16 ANSI colors keyed by the names the Rust cell emits.  Note 33/43 are
    # reported as "brown" (not "yellow") by graphics.rs.
    ansi: dict


def _theme(name, bg, fg, cur, sel, palette) -> Theme:
    keys = ["black", "red", "green", "brown", "blue", "magenta", "cyan",
            "white",
            "brightblack", "brightred", "brightgreen", "brightbrown",
            "brightblue", "brightmagenta", "brightcyan", "brightwhite"]
    ansi = dict(zip(keys, palette))
    # The Rust layer sometimes uses "yellow"/"brightyellow" aliases.
    ansi["yellow"] = ansi["brown"]
    ansi["brightyellow"] = ansi["brightbrown"]
    return Theme(name, bg, fg, cur, sel, None, "#5a4a00", "#a07800", ansi)


THEMES: dict[str, Theme] = {
    "Default Dark": _theme(
        "Default Dark", "#1e1e1e", "#d4d4d4", "#aeafad", "#264f78",
        ["#1e1e1e", "#cd3131", "#0dbc79", "#a87f32", "#2472c8", "#bc3fbc",
         "#11a8cd", "#e5e5e5", "#666666", "#f14c4c", "#23d18b", "#d7a35b",
         "#3b8eea", "#d670d6", "#29b8db", "#ffffff"]),
    "Solarized Dark": _theme(
        "Solarized Dark", "#002b36", "#839496", "#93a1a1", "#073642",
        ["#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682",
         "#2aa198", "#eee8d5", "#586e75", "#cb4b16", "#586e75", "#657b83",
         "#839496", "#6c71c4", "#93a1a1", "#fdf6e3"]),
    "Gruvbox": _theme(
        "Gruvbox", "#282828", "#ebdbb2", "#fe8019", "#504945",
        ["#282828", "#cc241d", "#98971a", "#d79921", "#458588", "#b16286",
         "#689d6a", "#a89984", "#928374", "#fb4934", "#b8bb26", "#fabd2f",
         "#83a598", "#d3869b", "#8ec07c", "#ebdbb2"]),
    "Nord": _theme(
        "Nord", "#2e3440", "#d8dee9", "#88c0d0", "#434c5e",
        ["#3b4252", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead",
         "#88c0d0", "#e5e9f0", "#4c566a", "#bf616a", "#a3be8c", "#ebcb8b",
         "#81a1c1", "#b48ead", "#8fbcbb", "#eceff4"]),
    "Light": _theme(
        "Light", "#fafafa", "#383a42", "#526fff", "#bfceff",
        ["#fafafa", "#e45649", "#50a14f", "#c18401", "#4078f2", "#a626a4",
         "#0184bc", "#383a42", "#a0a1a7", "#e45649", "#50a14f", "#986801",
         "#4078f2", "#a626a4", "#0184bc", "#090a0b"]),
}
DEFAULT_THEME = "Default Dark"


# Attribute bitmask layout (matches history.rs::styled_viewport).
_A_BOLD, _A_DIM, _A_ITALIC, _A_UNDERLINE = 1 << 0, 1 << 1, 1 << 2, 1 << 3
_A_BLINK, _A_REVERSE, _A_HIDDEN, _A_STRIKE = 1 << 4, 1 << 5, 1 << 6, 1 << 7

_HEXDIGITS = set("0123456789abcdefABCDEF")

# URL detection (trailing punctuation trimmed when resolving).
_URL_RE = re.compile(r"(?:https?://|www\.)[^\s<>\"'`|(){}\[\]]+", re.IGNORECASE)
_URL_TRAIL = ".,;:!?)]}>\"'"


# ─────────────────────────────────────────────────────────────────────
# Runtime configuration (shared by reference across all views)
# ─────────────────────────────────────────────────────────────────────

@dataclass
class Config:
    font_family: str = ("Cascadia Code" if not IS_MAC else "Menlo")
    font_size: float = 12.0
    theme_name: str = DEFAULT_THEME
    copy_on_select: bool = False
    audible_bell: bool = False
    visual_bell: bool = True
    cursor_blink: bool = True
    scrollback_lines: int = 5000

    @property
    def theme(self) -> Theme:
        return THEMES.get(self.theme_name, THEMES[DEFAULT_THEME])


# ─────────────────────────────────────────────────────────────────────
# Terminal feature flags scanned out of the raw byte stream
# ─────────────────────────────────────────────────────────────────────

@dataclass
class TermFlags:
    app_cursor: bool = False        # DECCKM (?1)
    mouse_proto: int = 0            # 0 / 1000 / 1002 / 1003
    sgr_mouse: bool = False         # ?1006
    bracketed_paste: bool = False   # ?2004
    alt_screen: bool = False        # ?1049 / ?47 / ?1047
    cursor_visible: bool = True     # DECTCEM (?25)
    cursor_shape: str = "block"     # block / underline / bar
    cursor_blink: bool = True       # from DECSCUSR


class BellCounter:
    """Counts terminal bells (BEL) in the raw output stream.

    Mode flags now come directly from the Rust ``TerminalState`` getters, so the
    GUI no longer scans the stream to recover them.  The bell, however, is not a
    mode — it has to be counted from the bytes.  OSC strings are terminated by
    BEL (e.g. title sequences), so completed OSCs are stripped before counting.
    A trailing, still-open OSC is deferred to the next chunk so its terminating
    BEL is never miscounted as a real bell.
    """

    _OSC_RE = re.compile(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)")

    def __init__(self) -> None:
        self._buf = ""
        self.pending = 0

    def feed(self, data: bytes) -> None:
        text = self._buf + data.decode("latin-1", errors="replace")
        carry = 0
        idx = text.rfind("\x1b]")
        if idx != -1 and "\x07" not in text[idx:] and "\x1b\\" not in text[idx + 2:]:
            carry = len(text) - idx          # open OSC — defer from its introducer
        elif text.endswith("\x1b"):
            carry = 1                        # lone ESC could begin an OSC
        if carry > 2048:                     # runaway/unterminated OSC — give up
            carry = 0
        head = text if carry == 0 else text[:-carry]
        self.pending += self._OSC_RE.sub("", head).count("\x07")
        self._buf = "" if carry == 0 else text[-carry:]

    def reset(self) -> None:
        self._buf = ""
        self.pending = 0


def flags_from_terminal(t) -> TermFlags:
    """Build TermFlags from the live mode getters on stitch-pty's TerminalState.

    Uses getattr with defaults so the emulator still runs against an older
    module build that doesn't expose one of these getters yet.
    """
    def g(name, default):
        return getattr(t, name, default)
    return TermFlags(
        app_cursor=bool(g("app_cursor", False)),
        mouse_proto=int(g("mouse_proto", 0)),
        sgr_mouse=bool(g("sgr_mouse", False)),
        bracketed_paste=bool(g("bracketed_paste", False)),
        alt_screen=bool(g("alt_screen", False)),
        cursor_visible=bool(g("cursor_visible", True)),
        cursor_shape=str(g("cursor_shape", "block")),
        cursor_blink=bool(g("cursor_blink", True)),
    )


# ─────────────────────────────────────────────────────────────────────
# Frame — one snapshot shipped across the thread boundary
# ─────────────────────────────────────────────────────────────────────

@dataclass(frozen=True)
class Frame:
    rows: list                 # list[list[(text, fg, bg, attrs)]] — full buffer
    cursor_x: int
    cursor_y: int              # absolute row index into `rows`
    title: str
    history: int               # scrollback line count
    total: int                 # total rows = history + visible
    visible: int               # number of on-screen rows
    flags: TermFlags
    alive: bool


# ─────────────────────────────────────────────────────────────────────
# Key → PTY byte translation
# ─────────────────────────────────────────────────────────────────────

def _xterm_mod(shift: bool, alt: bool, ctrl: bool, meta: bool) -> int:
    return 1 + (1 if shift else 0) + (2 if alt else 0) \
             + (4 if ctrl else 0) + (8 if meta else 0)


_NAV = {  # key: (final-letter for SS3/CSI form)
    Qt.Key.Key_Up: "A", Qt.Key.Key_Down: "B",
    Qt.Key.Key_Right: "C", Qt.Key.Key_Left: "D",
    Qt.Key.Key_Home: "H", Qt.Key.Key_End: "F",
}
_TILDE = {  # key: numeric code for CSI <n> ~
    Qt.Key.Key_Insert: 2, Qt.Key.Key_Delete: 3,
    Qt.Key.Key_PageUp: 5, Qt.Key.Key_PageDown: 6,
}
_FN_SS3 = {Qt.Key.Key_F1: "P", Qt.Key.Key_F2: "Q",
           Qt.Key.Key_F3: "R", Qt.Key.Key_F4: "S"}
_FN_TILDE = {Qt.Key.Key_F5: 15, Qt.Key.Key_F6: 17, Qt.Key.Key_F7: 18,
             Qt.Key.Key_F8: 19, Qt.Key.Key_F9: 20, Qt.Key.Key_F10: 21,
             Qt.Key.Key_F11: 23, Qt.Key.Key_F12: 24}


def key_event_to_bytes(event: QKeyEvent, app_cursor: bool) -> bytes | None:
    """Translate a Qt key event to the bytes a real terminal would send."""
    key = event.key()
    text = event.text()
    mods = event.modifiers()
    ctrl = bool(mods & Qt.KeyboardModifier.ControlModifier)
    shift = bool(mods & Qt.KeyboardModifier.ShiftModifier)
    alt = bool(mods & Qt.KeyboardModifier.AltModifier)
    meta = bool(mods & Qt.KeyboardModifier.MetaModifier)
    mod_any = ctrl or shift or alt or meta
    code = _xterm_mod(shift, alt, ctrl, meta)

    def esc_prefix(b: bytes) -> bytes:
        return (b"\x1b" + b) if alt else b

    # Navigation / arrows / Home / End ──────────────────────────────
    if key in _NAV:
        letter = _NAV[key]
        if mod_any:
            return f"\x1b[1;{code}{letter}".encode()
        if app_cursor and key in (Qt.Key.Key_Up, Qt.Key.Key_Down,
                                  Qt.Key.Key_Right, Qt.Key.Key_Left,
                                  Qt.Key.Key_Home, Qt.Key.Key_End):
            return f"\x1bO{letter}".encode()
        return f"\x1b[{letter}".encode()

    # PgUp / PgDn / Insert / Delete ──────────────────────────────────
    if key in _TILDE:
        n = _TILDE[key]
        if mod_any:
            return f"\x1b[{n};{code}~".encode()
        return f"\x1b[{n}~".encode()

    # Function keys ──────────────────────────────────────────────────
    if key in _FN_SS3:
        letter = _FN_SS3[key]
        if mod_any:
            return f"\x1b[1;{code}{letter}".encode()
        return f"\x1bO{letter}".encode()
    if key in _FN_TILDE:
        n = _FN_TILDE[key]
        if mod_any:
            return f"\x1b[{n};{code}~".encode()
        return f"\x1b[{n}~".encode()

    # Editing / control keys ─────────────────────────────────────────
    if key == Qt.Key.Key_Backspace:
        return esc_prefix(b"\x08" if ctrl else b"\x7f")
    if key == Qt.Key.Key_Tab:
        return b"\t"
    if key == Qt.Key.Key_Backtab or (key == Qt.Key.Key_Tab and shift):
        return b"\x1b[Z"
    if key in (Qt.Key.Key_Return, Qt.Key.Key_Enter):
        return esc_prefix(b"\r")
    if key == Qt.Key.Key_Escape:
        return b"\x1b"

    # Ctrl + key combinations ────────────────────────────────────────
    if ctrl and not alt:
        if key == Qt.Key.Key_Space or key == Qt.Key.Key_2:
            return b"\x00"
        if Qt.Key.Key_A <= key <= Qt.Key.Key_Z:
            return bytes([(key - Qt.Key.Key_A) + 1])
        ctrl_punct = {
            Qt.Key.Key_BracketLeft: 0x1b, Qt.Key.Key_Backslash: 0x1c,
            Qt.Key.Key_BracketRight: 0x1d, Qt.Key.Key_AsciiCircum: 0x1e,
            Qt.Key.Key_6: 0x1e, Qt.Key.Key_Underscore: 0x1f,
            Qt.Key.Key_Minus: 0x1f, Qt.Key.Key_Slash: 0x1f,
        }
        if key in ctrl_punct:
            return bytes([ctrl_punct[key]])

    # Printable text (optionally Alt = Meta → ESC prefix) ────────────
    if text and text.isprintable():
        data = text.encode("utf-8", errors="replace")
        return esc_prefix(data)
    if text:
        return esc_prefix(text.encode("utf-8", errors="replace"))
    return None


# ─────────────────────────────────────────────────────────────────────
# Small color helpers
# ─────────────────────────────────────────────────────────────────────

def resolve_color(color: str, theme: Theme, is_bg: bool) -> QColor:
    if not color or color == "default":
        return QColor(theme.bg if is_bg else theme.fg)
    if color in theme.ansi:
        return QColor(theme.ansi[color])
    if len(color) == 6 and all(c in _HEXDIGITS for c in color):
        return QColor("#" + color)
    return QColor(theme.bg if is_bg else theme.fg)


def dim_color(c: QColor) -> QColor:
    return QColor(int(c.red() * 0.6), int(c.green() * 0.6),
                  int(c.blue() * 0.6))


# ─────────────────────────────────────────────────────────────────────
# TerminalView — the custom-painted cell grid + all input handling
# ─────────────────────────────────────────────────────────────────────

class TerminalView(QWidget):
    """A focusable, custom-painted terminal grid.

    Renders `Frame` snapshots, handles keyboard/mouse input, selection,
    clipboard, scrollback navigation, URL hovering and search highlighting.
    Emits `key_input` for everything that should reach the child process.
    """

    key_input = Signal(bytes)            # bytes to write to the PTY
    grid_resized = Signal(int, int)      # (rows, cols) when the grid changes
    scroll_to_bottom_requested = Signal()

    def __init__(self, config: Config, scrollbar: QScrollBar,
                 parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.cfg = config
        self.vbar = scrollbar
        self.frame: Frame | None = None
        self._flags = TermFlags()

        self.setFocusPolicy(Qt.FocusPolicy.StrongFocus)
        self.setMouseTracking(True)
        self.setCursor(Qt.CursorShape.IBeamCursor)
        self.setAttribute(Qt.WidgetAttribute.WA_OpaquePaintEvent, True)
        self.setSizePolicy(QSizePolicy.Policy.Expanding,
                           QSizePolicy.Policy.Expanding)

        # Geometry
        self.cell_w = 8.0
        self.cell_h = 16.0
        self.baseline = 12.0
        self.cols = 80
        self.rows = 24

        # Scroll (must exist before _apply_font, which may recompute the grid)
        self._follow = True
        self.vbar.valueChanged.connect(self._on_scrollbar)

        self._apply_font()


        # Selection (absolute buffer coordinates)
        self._sel_anchor: tuple[int, int] | None = None
        self._sel_head: tuple[int, int] | None = None
        self._sel_mode = "char"          # char / word / line / block
        self._selecting = False
        self._click_count = 0
        self._last_click_t = 0.0
        self._last_click_cell: tuple[int, int] | None = None

        # URL hover
        self._hover_url: tuple[int, int, int] | None = None  # (row, c0, c1)

        # Search
        self._matches: list[tuple[int, int, int]] = []       # (row, c0, len)
        self._match_idx = -1

        # Cursor blink + visual bell
        self._blink_on = True
        self._blink_timer = QTimer(self)
        self._blink_timer.timeout.connect(self._blink)
        self._restart_blink()
        self._bell_flash = False
        self._bell_timer = QTimer(self)
        self._bell_timer.setSingleShot(True)
        self._bell_timer.timeout.connect(self._end_bell)

        # Auto-scroll while drag-selecting past the edge
        self._autoscroll = QTimer(self)
        self._autoscroll.setInterval(40)
        self._autoscroll.timeout.connect(self._do_autoscroll)
        self._autoscroll_dir = 0

    # ── font / geometry ─────────────────────────────────────────────

    def _apply_font(self) -> None:
        font = QFont(self.cfg.font_family)
        font.setStyleHint(QFont.StyleHint.Monospace)
        font.setFixedPitch(True)
        font.setPointSizeF(self.cfg.font_size)
        if not font.exactMatch():
            for fb in ("Cascadia Code", "JetBrains Mono", "Fira Code",
                       "Menlo", "DejaVu Sans Mono", "Consolas",
                       "Courier New", "monospace"):
                cand = QFont(fb)
                cand.setStyleHint(QFont.StyleHint.Monospace)
                cand.setFixedPitch(True)
                cand.setPointSizeF(self.cfg.font_size)
                if cand.exactMatch():
                    font = cand
                    break
        self._font = font
        self._font_bold = QFont(font); self._font_bold.setBold(True)
        self._font_italic = QFont(font); self._font_italic.setItalic(True)
        self._font_bi = QFont(font)
        self._font_bi.setBold(True); self._font_bi.setItalic(True)

        fm = QFontMetricsF(font)
        self.cell_w = max(1.0, fm.horizontalAdvance("M"))
        self.cell_h = max(1.0, fm.height())
        self.baseline = fm.ascent()
        self._recompute_grid()
        self.update()

    def set_config_changed(self) -> None:
        """Re-read shared config (font/theme/blink) and repaint."""
        self._apply_font()
        self._restart_blink()
        self.update()

    def _recompute_grid(self) -> None:
        cols = max(2, int(self.width() / self.cell_w))
        rows = max(1, int(self.height() / self.cell_h))
        if cols != self.cols or rows != self.rows:
            self.cols, self.rows = cols, rows
            self._update_scrollbar()
            self.grid_resized.emit(rows, cols)

    def resizeEvent(self, event) -> None:
        super().resizeEvent(event)
        self._recompute_grid()
        self.update()

    def sizeHint(self) -> QSize:
        return QSize(int(self.cell_w * 80) + 4, int(self.cell_h * 24) + 4)

    # ── scrollbar ────────────────────────────────────────────────────

    def _update_scrollbar(self) -> None:
        total = self.frame.total if self.frame else self.rows
        max_top = max(0, total - self.rows)
        self.vbar.setRange(0, max_top)
        self.vbar.setPageStep(self.rows)
        self.vbar.setSingleStep(1)
        if self._follow:
            self.vbar.setValue(max_top)

    def _on_scrollbar(self, value: int) -> None:
        max_top = self.vbar.maximum()
        self._follow = value >= max_top
        self.update()

    @property
    def top_row(self) -> int:
        return self.vbar.value()

    def scroll_lines(self, delta: int) -> None:
        self.vbar.setValue(self.vbar.value() + delta)

    def scroll_to_bottom(self) -> None:
        self._follow = True
        self.vbar.setValue(self.vbar.maximum())

    # ── frame intake ─────────────────────────────────────────────────

    def set_frame(self, frame: Frame) -> None:
        self.frame = frame
        self._flags = frame.flags
        self._update_scrollbar()
        if self._matches:                 # keep highlights fresh after output
            self._refresh_matches()
        self._restart_blink()
        self.update()

    def show_bell(self) -> None:
        if self.cfg.audible_bell:
            QApplication.beep()
        if self.cfg.visual_bell:
            self._bell_flash = True
            self._bell_timer.start(90)
            self.update()

    def _end_bell(self) -> None:
        self._bell_flash = False
        self.update()

    # ── cursor blink ─────────────────────────────────────────────────

    def _restart_blink(self) -> None:
        self._blink_on = True
        blink = self.cfg.cursor_blink and self._flags.cursor_blink
        if blink and self.hasFocus():
            self._blink_timer.start(530)
        else:
            self._blink_timer.stop()
        if self.isVisible():
            self.update()

    def _blink(self) -> None:
        self._blink_on = not self._blink_on
        self.update(self._cursor_rect())

    def focusInEvent(self, event) -> None:
        super().focusInEvent(event)
        self._restart_blink()

    def focusOutEvent(self, event) -> None:
        super().focusOutEvent(event)
        self._blink_timer.stop()
        self.update()

    # ── painting ─────────────────────────────────────────────────────

    def paintEvent(self, event) -> None:
        p = QPainter(self)
        try:
            p.setRenderHint(QPainter.RenderHint.TextAntialiasing, True)
            theme = self.cfg.theme
            bg = QColor(theme.bg)
            if self._bell_flash:
                bg = QColor(theme.fg)
            p.fillRect(self.rect(), bg)

            if not self.frame:
                return

            rows = self.frame.rows
            top = self.top_row
            sel = self._normalized_selection()

            for screen_row in range(self.rows):
                ridx = top + screen_row
                if ridx >= len(rows):
                    break
                self._paint_row(p, screen_row, ridx, rows[ridx], theme, sel)

            self._paint_cursor(p, theme, top)
        finally:
            p.end()

    def _paint_row(self, p: QPainter, screen_row: int, ridx: int,
                   cells: list, theme: Theme, sel) -> None:
        y = screen_row * self.cell_h
        # Build coalesced runs of identical effective style.
        runs: list[tuple[int, list[str], QColor, QColor, int, QFont]] = []
        cur = None
        for cidx in range(self.cols):
            if cidx < len(cells):
                text, fgname, bgname, attrs = cells[cidx]
            else:
                text, fgname, bgname, attrs = " ", "default", "default", 0
            fg = resolve_color(fgname, theme, is_bg=False)
            cbg = resolve_color(bgname, theme, is_bg=True)
            if attrs & _A_DIM:
                fg = dim_color(fg)
            if attrs & _A_REVERSE:
                fg, cbg = cbg, fg
            if attrs & _A_HIDDEN:
                fg = cbg
            # Selection wins over normal styling.
            if self._cell_selected(ridx, cidx, sel):
                cbg = QColor(theme.sel_bg)
                if theme.sel_fg:
                    fg = QColor(theme.sel_fg)
            # Search matches.
            mhit = self._match_at(ridx, cidx)
            if mhit is not None:
                cbg = QColor(theme.match_cur_bg if mhit else theme.match_bg)
                fg = QColor("#000000")

            bold = bool(attrs & _A_BOLD)
            italic = bool(attrs & _A_ITALIC)
            uline = bool(attrs & _A_UNDERLINE)
            strike = bool(attrs & _A_STRIKE)
            # URL hover underline.
            if self._hover_url and self._hover_url[0] == ridx \
                    and self._hover_url[1] <= cidx < self._hover_url[2]:
                uline = True

            font = (self._font_bi if bold and italic else
                    self._font_bold if bold else
                    self._font_italic if italic else self._font)
            line_flags = (1 if uline else 0) | (2 if strike else 0)
            ch = text if text else " "

            key = (fg.rgb(), cbg.rgb(), line_flags, bold, italic)
            if cur and cur[6] == key:
                cur[1].append(ch)
            else:
                cur = [cidx, [ch], fg, cbg, line_flags, font, key]
                runs.append(cur)

        for start_col, chars, fg, cbg, line_flags, font, _ in runs:
            x = start_col * self.cell_w
            w = len(chars) * self.cell_w
            rect = QRect(int(round(x)), int(round(y)),
                         int(round(w)), int(round(self.cell_h)) + 1)
            if cbg != QColor(theme.bg) or self._bell_flash:
                p.fillRect(rect, cbg)
            s = "".join(chars)
            if s.strip():
                p.setFont(font)
                p.setPen(QPen(fg))
                p.drawText(QPoint(int(round(x)),
                                  int(round(y + self.baseline))), s)
            if line_flags & 1:               # underline
                ly = int(round(y + self.cell_h - 1))
                p.setPen(QPen(fg))
                p.drawLine(int(round(x)), ly, int(round(x + w)), ly)
            if line_flags & 2:               # strikethrough
                ly = int(round(y + self.cell_h * 0.55))
                p.setPen(QPen(fg))
                p.drawLine(int(round(x)), ly, int(round(x + w)), ly)

    def _cursor_rect(self) -> QRect:
        if not self.frame:
            return QRect()
        screen_row = self.frame.cursor_y - self.top_row
        x = self.frame.cursor_x * self.cell_w
        y = screen_row * self.cell_h
        return QRect(int(x), int(y), int(self.cell_w) + 1,
                     int(self.cell_h) + 1)

    def _paint_cursor(self, p: QPainter, theme: Theme, top: int) -> None:
        f = self.frame
        if not f or not self._flags.cursor_visible:
            return
        screen_row = f.cursor_y - top
        if not (0 <= screen_row < self.rows):
            return
        x = f.cursor_x * self.cell_w
        y = screen_row * self.cell_h
        col = QColor(theme.cursor)
        focused = self.hasFocus()

        if not focused:
            p.setPen(QPen(col, 1))
            p.setBrush(Qt.BrushStyle.NoBrush)
            p.drawRect(QRect(int(x), int(y),
                             int(self.cell_w), int(self.cell_h) - 1))
            return
        if not self._blink_on:
            return

        shape = self._flags.cursor_shape
        if shape == "bar":
            p.fillRect(QRect(int(x), int(y), 2, int(self.cell_h)), col)
        elif shape == "underline":
            p.fillRect(QRect(int(x), int(y + self.cell_h - 2),
                             int(self.cell_w), 2), col)
        else:  # block — fill and redraw the glyph beneath in bg color
            rect = QRect(int(x), int(y), int(self.cell_w), int(self.cell_h))
            p.fillRect(rect, col)
            ch = self._char_at(f.cursor_y, f.cursor_x)
            if ch and ch.strip():
                p.setFont(self._font)
                p.setPen(QPen(QColor(theme.bg)))
                p.drawText(QPoint(int(x), int(y + self.baseline)), ch)

    # ── data access helpers ──────────────────────────────────────────

    def _char_at(self, row: int, col: int) -> str:
        if not self.frame or row >= len(self.frame.rows):
            return " "
        cells = self.frame.rows[row]
        if col < len(cells):
            t = cells[col][0]
            return t if t else " "
        return " "

    def _row_text(self, row: int) -> str:
        if not self.frame or row >= len(self.frame.rows):
            return ""
        return "".join((c[0] if c[0] else " ") for c in self.frame.rows[row])

    # ── coordinate mapping ────────────────────────────────────────────

    def _cell_at(self, pos: QPoint) -> tuple[int, int]:
        col = int(pos.x() / self.cell_w)
        col = max(0, min(self.cols - 1, col))
        row = self.top_row + int(pos.y() / self.cell_h)
        if self.frame:
            row = max(0, min(len(self.frame.rows) - 1, row))
        return row, col

    # ── selection ─────────────────────────────────────────────────────

    def _normalized_selection(self):
        if self._sel_anchor is None or self._sel_head is None:
            return None
        a, b = self._sel_anchor, self._sel_head
        if (a[0], a[1]) <= (b[0], b[1]):
            return a, b, self._sel_mode
        return b, a, self._sel_mode

    def _cell_selected(self, row: int, col: int, sel) -> bool:
        if not sel:
            return False
        (r0, c0), (r1, c1), mode = sel
        if mode == "block":
            lo_c, hi_c = min(c0, c1), max(c0, c1)
            return r0 <= row <= r1 and lo_c <= col <= hi_c
        if row < r0 or row > r1:
            return False
        if r0 == r1:
            return c0 <= col <= c1
        if row == r0:
            return col >= c0
        if row == r1:
            return col <= c1
        return True

    def has_selection(self) -> bool:
        sel = self._normalized_selection()
        if not sel:
            return False
        (r0, c0), (r1, c1), _ = sel
        return (r0, c0) != (r1, c1)

    def clear_selection(self) -> None:
        self._sel_anchor = self._sel_head = None
        self.update()

    def select_all(self) -> None:
        if not self.frame or not self.frame.rows:
            return
        last = len(self.frame.rows) - 1
        self._sel_mode = "char"
        self._sel_anchor = (0, 0)
        self._sel_head = (last, max(0, self.cols - 1))
        self.update()

    def selected_text(self) -> str:
        sel = self._normalized_selection()
        if not sel or not self.frame:
            return ""
        (r0, c0), (r1, c1), mode = sel
        out: list[str] = []
        if mode == "block":
            lo, hi = min(c0, c1), max(c0, c1)
            for r in range(r0, r1 + 1):
                line = self._row_text(r)
                out.append(line[lo:hi + 1].rstrip())
            return "\n".join(out)
        for r in range(r0, r1 + 1):
            line = self._row_text(r)
            s = c0 if r == r0 else 0
            e = (c1 + 1) if r == r1 else len(line)
            out.append(line[s:e].rstrip())
        return "\n".join(out)

    def _word_bounds(self, row: int, col: int) -> tuple[int, int]:
        line = self._row_text(row)
        if not line or col >= len(line):
            return col, col
        wordish = lambda c: c.isalnum() or c in "_-./~:@"  # noqa: E731
        if not wordish(line[col]):
            return col, col
        s = col
        while s > 0 and wordish(line[s - 1]):
            s -= 1
        e = col
        while e < len(line) - 1 and wordish(line[e + 1]):
            e += 1
        return s, e

    # ── mouse ──────────────────────────────────────────────────────────

    def _mouse_reporting(self) -> bool:
        return self._flags.mouse_proto != 0

    def mousePressEvent(self, event) -> None:
        self.setFocus()
        pos = event.position().toPoint()
        row, col = self._cell_at(pos)
        btn = event.button()
        mods = event.modifiers()
        shift = bool(mods & Qt.KeyboardModifier.ShiftModifier)
        ctrl = bool(mods & Qt.KeyboardModifier.ControlModifier)
        alt = bool(mods & Qt.KeyboardModifier.AltModifier)

        # Ctrl/⌘-click on a URL opens it.
        if btn == Qt.MouseButton.LeftButton and (ctrl or (IS_MAC and ctrl)):
            url = self._url_at(row, col)
            if url:
                self._open_url(url)
                return

        # Middle-click pastes the primary selection (X11 convention).
        if btn == Qt.MouseButton.MiddleButton:
            self._paste(QClipboard.Mode.Selection)
            return

        # Forward to the app when it requested mouse reporting (Shift overrides).
        if self._mouse_reporting() and not shift \
                and btn in (Qt.MouseButton.LeftButton,
                            Qt.MouseButton.MiddleButton,
                            Qt.MouseButton.RightButton):
            self._send_mouse(btn, row, col, mods, press=True)
            return

        # Right-click: with copy-on-select active, paste the clipboard; the
        # context menu then requires Ctrl+right-click. Without copy-on-select,
        # fall through so the normal context menu appears.
        if btn == Qt.MouseButton.RightButton and self.cfg.copy_on_select \
                and not ctrl:
            self._paste(QClipboard.Mode.Clipboard)
            return

        if btn != Qt.MouseButton.LeftButton:
            super().mousePressEvent(event)
            return

        # Multi-click detection (double = word, triple = line).
        now = time.monotonic()
        if (now - self._last_click_t < 0.4
                and self._last_click_cell == (row, col)):
            self._click_count += 1
        else:
            self._click_count = 1
        self._last_click_t = now
        self._last_click_cell = (row, col)

        self._selecting = True
        if alt:
            self._sel_mode = "block"
            self._sel_anchor = (row, col)
            self._sel_head = (row, col)
        elif self._click_count >= 3:
            self._sel_mode = "line"
            self._sel_anchor = (row, 0)
            self._sel_head = (row, self.cols - 1)
        elif self._click_count == 2:
            self._sel_mode = "word"
            s, e = self._word_bounds(row, col)
            self._sel_anchor = (row, s)
            self._sel_head = (row, e)
        else:
            self._sel_mode = "char"
            if shift and self._sel_anchor is not None:
                self._sel_head = (row, col)
            else:
                self._sel_anchor = (row, col)
                self._sel_head = (row, col)
        self.update()

    def mouseMoveEvent(self, event) -> None:
        pos = event.position().toPoint()
        buttons = event.buttons()

        if self._selecting and (buttons & Qt.MouseButton.LeftButton):
            row, col = self._cell_at(pos)
            if self._sel_mode == "word":
                s, e = self._word_bounds(row, col)
                self._sel_head = (row, e if (row, col) >= self._sel_anchor
                                  else s)
            elif self._sel_mode == "line":
                self._sel_head = (row, self.cols - 1
                                  if row >= self._sel_anchor[0] else 0)
            else:
                self._sel_head = (row, col)
            # Auto-scroll if dragging past top/bottom edge.
            if pos.y() < 0:
                self._autoscroll_dir = -1; self._autoscroll.start()
            elif pos.y() > self.height():
                self._autoscroll_dir = 1; self._autoscroll.start()
            else:
                self._autoscroll.stop()
            self.update()
            return

        # Forward motion to mouse-reporting apps that asked for it.
        if self._mouse_reporting() and self._flags.mouse_proto in (1002, 1003) \
                and (buttons or self._flags.mouse_proto == 1003):
            row, col = self._cell_at(pos)
            self._send_mouse(Qt.MouseButton.NoButton, row, col,
                             event.modifiers(), motion=True,
                             held=bool(buttons))
            return

        # Hover: detect URL under the pointer and switch cursor.
        row, col = self._cell_at(pos)
        url_range = self._url_range_at(row, col)
        if url_range != self._hover_url:
            self._hover_url = url_range
            self.update()
        self.setCursor(Qt.CursorShape.PointingHandCursor if url_range
                       else Qt.CursorShape.IBeamCursor)

    def mouseReleaseEvent(self, event) -> None:
        self._autoscroll.stop()
        if self._mouse_reporting() and not (
                event.modifiers() & Qt.KeyboardModifier.ShiftModifier):
            if event.button() in (Qt.MouseButton.LeftButton,
                                   Qt.MouseButton.MiddleButton,
                                   Qt.MouseButton.RightButton):
                row, col = self._cell_at(event.position().toPoint())
                self._send_mouse(event.button(), row, col,
                                 event.modifiers(), press=False)
        if self._selecting:
            self._selecting = False
            if self.cfg.copy_on_select and self.has_selection():
                self.copy(QClipboard.Mode.Clipboard)
                self.copy(QClipboard.Mode.Selection)
            elif self.has_selection():
                self.copy(QClipboard.Mode.Selection)   # primary always tracks

    def _do_autoscroll(self) -> None:
        self.scroll_lines(self._autoscroll_dir)
        if self._sel_head is not None:
            r = max(0, self.top_row if self._autoscroll_dir < 0
                    else self.top_row + self.rows - 1)
            self._sel_head = (r, self._sel_head[1])
        self.update()

    def wheelEvent(self, event) -> None:
        mods = event.modifiers()
        if mods & Qt.KeyboardModifier.ControlModifier:
            steps = event.angleDelta().y()
            self.parent_zoom(1 if steps > 0 else -1)
            return
        notches = event.angleDelta().y() / 120.0
        lines = int(notches * 3) or (1 if notches > 0 else -1)
        # In an alt-screen app the wheel scrolls the application, not history.
        if self._flags.alt_screen and not self._mouse_reporting():
            seq = b"\x1b[A" if lines > 0 else b"\x1b[B"
            if self._flags.app_cursor:
                seq = b"\x1bOA" if lines > 0 else b"\x1bOB"
            for _ in range(abs(lines)):
                self.key_input.emit(seq)
            return
        if self._mouse_reporting():
            row, col = self._cell_at(event.position().toPoint())
            self._send_wheel(lines > 0, row, col, mods)
            return
        self.scroll_lines(-lines)

    def parent_zoom(self, delta: int) -> None:
        # Wired by the owning tab/window to the global zoom handler.
        self.zoom_requested.emit(delta)

    zoom_requested = Signal(int)

    # ── mouse reporting encoders ──────────────────────────────────────

    def _screen_coords(self, row: int, col: int) -> tuple[int, int] | None:
        if not self.frame:
            return None
        sy = row - self.frame.history     # row relative to the visible screen
        if not (0 <= sy < self.frame.visible):
            return None
        sx = max(0, min(self.cols - 1, col))
        return sx, max(0, min(self.frame.visible - 1, sy))

    def _mouse_button_code(self, btn, mods) -> int:
        base = {Qt.MouseButton.LeftButton: 0,
                Qt.MouseButton.MiddleButton: 1,
                Qt.MouseButton.RightButton: 2}.get(btn, 3)
        if mods & Qt.KeyboardModifier.ShiftModifier:
            base += 4
        if mods & Qt.KeyboardModifier.AltModifier:
            base += 8
        if mods & Qt.KeyboardModifier.ControlModifier:
            base += 16
        return base

    def _emit_mouse(self, code: int, sx: int, sy: int, press: bool) -> None:
        if self._flags.sgr_mouse:
            final = "M" if press else "m"
            self.key_input.emit(
                f"\x1b[<{code};{sx + 1};{sy + 1}{final}".encode())
        else:
            cb = 3 if not press and code < 3 else code
            data = bytes([0x1b, ord("["), ord("M"),
                          32 + (cb if press else 3),
                          32 + min(223, sx + 1), 32 + min(223, sy + 1)])
            self.key_input.emit(data)

    def _send_mouse(self, btn, row, col, mods, press=True,
                    motion=False, held=True) -> None:
        sc = self._screen_coords(row, col)
        if sc is None:
            return
        sx, sy = sc
        code = self._mouse_button_code(btn, mods)
        if motion:
            code = (code if held else 3) + 32
            self._emit_mouse(code, sx, sy, True)
        else:
            self._emit_mouse(code, sx, sy, press)

    def _send_wheel(self, up: bool, row, col, mods) -> None:
        sc = self._screen_coords(row, col)
        if sc is None:
            return
        sx, sy = sc
        code = 64 if up else 65
        if mods & Qt.KeyboardModifier.ShiftModifier:
            code += 4
        if mods & Qt.KeyboardModifier.ControlModifier:
            code += 16
        self._emit_mouse(code, sx, sy, True)

    # ── URLs ───────────────────────────────────────────────────────────

    def _url_range_at(self, row: int, col: int):
        line = self._row_text(row)
        for m in _URL_RE.finditer(line):
            s, e = m.start(), m.end()
            while e > s and line[e - 1] in _URL_TRAIL:
                e -= 1
            if s <= col < e:
                return (row, s, e)
        return None

    def _url_at(self, row: int, col: int) -> str | None:
        r = self._url_range_at(row, col)
        if not r:
            return None
        return self._row_text(row)[r[1]:r[2]]

    def _open_url(self, url: str) -> None:
        if url.lower().startswith("www."):
            url = "https://" + url
        QDesktopServices.openUrl(QUrl(url))

    # ── search ──────────────────────────────────────────────────────────

    def set_search(self, term: str) -> None:
        self._search_term = term
        self._refresh_matches()
        if self._matches:
            self._match_idx = 0
            self._scroll_to_match()
        self.update()

    def _refresh_matches(self) -> None:
        term = getattr(self, "_search_term", "")
        self._matches = []
        if not term or not self.frame:
            self._match_idx = -1
            return
        low = term.lower()
        for r in range(len(self.frame.rows)):
            line = self._row_text(r).lower()
            start = 0
            while True:
                i = line.find(low, start)
                if i < 0:
                    break
                self._matches.append((r, i, len(term)))
                start = i + 1
        if self._matches:
            self._match_idx = min(max(self._match_idx, 0),
                                  len(self._matches) - 1)
        else:
            self._match_idx = -1

    def _match_at(self, row: int, col: int):
        """Return True if `col` is in the current match, False if in another
        match, None otherwise."""
        for i, (r, c0, length) in enumerate(self._matches):
            if r == row and c0 <= col < c0 + length:
                return i == self._match_idx
        return None

    def find_next(self, forward: bool = True) -> None:
        if not self._matches:
            return
        self._match_idx = (self._match_idx + (1 if forward else -1)) \
            % len(self._matches)
        self._scroll_to_match()
        self.update()

    def _scroll_to_match(self) -> None:
        if not (0 <= self._match_idx < len(self._matches)):
            return
        r = self._matches[self._match_idx][0]
        if not (self.top_row <= r < self.top_row + self.rows):
            self._follow = False
            self.vbar.setValue(max(0, r - self.rows // 2))

    def clear_search(self) -> None:
        self._search_term = ""
        self._matches = []
        self._match_idx = -1
        self.update()

    @property
    def match_count(self) -> int:
        return len(self._matches)

    @property
    def match_index(self) -> int:
        return self._match_idx

    # ── clipboard ────────────────────────────────────────────────────

    def copy(self, mode=QClipboard.Mode.Clipboard) -> None:
        cb = QApplication.clipboard()
        # The X11 "primary selection" doesn't exist on Windows/macOS; writing to
        # it there triggers a Qt warning, so skip it when unsupported.
        if mode == QClipboard.Mode.Selection and not cb.supportsSelection():
            return
        text = self.selected_text()
        if text:
            cb.setText(text, mode)

    def _paste(self, mode=QClipboard.Mode.Clipboard) -> None:
        cb = QApplication.clipboard()
        if mode == QClipboard.Mode.Selection and not cb.supportsSelection():
            mode = QClipboard.Mode.Clipboard   # fall back to the main clipboard
        text = cb.text(mode)
        if not text:
            return
        data = text.replace("\r\n", "\r").replace("\n", "\r").encode("utf-8")
        if self._flags.bracketed_paste:
            data = b"\x1b[200~" + data + b"\x1b[201~"
        self.scroll_to_bottom()
        self.key_input.emit(data)

    def paste(self) -> None:
        self._paste(QClipboard.Mode.Clipboard)

    # ── keyboard ──────────────────────────────────────────────────────

    def keyPressEvent(self, event: QKeyEvent) -> None:
        # Shift+PgUp / Shift+PgDn scroll local history instead of paging.
        mods = event.modifiers()
        if mods & Qt.KeyboardModifier.ShiftModifier:
            if event.key() == Qt.Key.Key_PageUp:
                self.scroll_lines(-(self.rows - 1)); return
            if event.key() == Qt.Key.Key_PageDown:
                self.scroll_lines(self.rows - 1); return

        data = key_event_to_bytes(event, self._flags.app_cursor)
        if data is not None:
            self.scroll_to_bottom()
            if self.has_selection() and not (
                    mods & Qt.KeyboardModifier.ShiftModifier):
                self.clear_selection()
            self.key_input.emit(data)
            event.accept()
        else:
            event.ignore()

    # ── context menu ──────────────────────────────────────────────────

    def contextMenuEvent(self, event) -> None:
        # With copy-on-select, a plain right-click pastes (see mousePressEvent),
        # so the menu is reserved for Ctrl+right-click. The keyboard Menu key
        # always opens it.
        if self.cfg.copy_on_select \
                and event.reason() == event.Reason.Mouse \
                and not (event.modifiers() & Qt.KeyboardModifier.ControlModifier):
            return
        menu = QMenu(self)
        act_copy = menu.addAction("Copy")
        act_copy.setShortcut(QKeySequence("Ctrl+Shift+C"))
        act_copy.setEnabled(self.has_selection())
        act_copy.triggered.connect(lambda: self.copy())
        act_paste = menu.addAction("Paste")
        act_paste.setShortcut(QKeySequence("Ctrl+Shift+V"))
        act_paste.setEnabled(bool(QApplication.clipboard().text()))
        act_paste.triggered.connect(self.paste)
        act_all = menu.addAction("Select All")
        act_all.triggered.connect(self.select_all)
        menu.addSeparator()

        row, col = self._cell_at(event.pos())
        url = self._url_at(row, col)
        if url:
            a = menu.addAction("Open Link")
            a.triggered.connect(lambda: self._open_url(url))
            a2 = menu.addAction("Copy Link")
            a2.triggered.connect(
                lambda: QApplication.clipboard().setText(url))
            menu.addSeparator()

        act_bottom = menu.addAction("Scroll to Bottom")
        act_bottom.triggered.connect(self.scroll_to_bottom)
        menu.exec(event.globalPos())


# ─────────────────────────────────────────────────────────────────────
# _AsyncRunner — background thread: asyncio loop + PtySession
# ─────────────────────────────────────────────────────────────────────

class _AsyncRunner(QObject):
    """Owns the asyncio loop and the Rust PtySession in a background QThread.

    Reads PTY output, feeds the integrated terminal, scans the stream for mode
    flags / bells, and emits plain-Python Frame snapshots to the GUI thread.
    """

    data_ready = Signal(object)       # Frame
    process_exited = Signal(int)      # exit code (or -1 if unknown)
    bell = Signal()
    title_changed = Signal(str)

    def __init__(self, program: str, args: list[str], winsize: Winsize,
                 scrollback: int) -> None:
        super().__init__()
        self.program = program
        self.args = args
        self.winsize = winsize
        self.scrollback = scrollback
        self.session: PtySession | None = None
        self.loop: asyncio.AbstractEventLoop | None = None
        self.bells = BellCounter()
        self._last_title = ""
        self._last_emit = 0.0
        self._min_frame_dt = 1.0 / 60.0   # cap emit rate to ~60fps
        self._dirty = False               # output arrived but not yet rendered
        self._stopping = False            # shutting down: stop the render pump
        self._pump_handle = None          # asyncio TimerHandle for the pump
        self.thread = QThread()
        self.moveToThread(self.thread)
        self.thread.started.connect(self._run_loop)
        self.thread.start()

    # ── loop lifecycle ────────────────────────────────────────────────

    def _run_loop(self) -> None:
        self.loop = asyncio.new_event_loop()
        asyncio.set_event_loop(self.loop)
        try:
            self.loop.run_until_complete(self._start())
            self.loop.run_forever()
        finally:
            try:
                self.loop.close()
            except Exception:
                pass

    async def _start(self) -> None:
        self.session = await spawn(self.program, self.args,
                                   winsize=self.winsize)
        try:
            self.session.terminal.set_scrollback_lines(self.scrollback)
        except Exception:
            pass
        asyncio.create_task(self._read_loop())
        self._pump()                      # start the independent render pump

    # ── read loop ──────────────────────────────────────────────────────

    async def _read_loop(self) -> None:
        assert self.session is not None
        while True:
            try:
                data = await self.session.read_timeout(8192, 0.05)
            except asyncio.CancelledError:
                return                   # shutdown requested — stop at once
            except PtyError:
                data = None              # idle read timeout — normal at a prompt
            except Exception:
                break                    # real I/O error: pipe gone
            if data:
                self.bells.feed(data)
                if self.bells.pending:
                    self.bells.pending = 0
                    self.bell.emit()
                self._dirty = True       # the render pump will pick this up
            elif not self.session.is_alive:
                break
        self._dirty = True
        self._emit_frame(force=True)     # guarantee the final state is shown
        code = -1
        try:
            res = await self.session.wait()
            if isinstance(res, dict):
                code = int(res.get("exit_code", -1) or -1)
        except Exception:
            pass
        self.process_exited.emit(code)

    def _pump(self) -> None:
        """Render unrendered output on a fixed cadence.

        Runs on the asyncio loop via ``call_later`` and reschedules itself, so
        it fires every frame interval *independently of the read coroutine*.
        This matters on Windows ConPTY, where an idle ``read_timeout`` may park
        waiting for the next byte rather than timing out — if rendering were
        driven from the read loop, the final frame after a command (the new
        prompt) would not appear until the next keystroke produced data.
        """
        if self._stopping:
            return
        if self.session is not None and self._dirty:
            self._emit_frame(force=True)
        loop = self.loop
        if loop is not None and not self._stopping:
            self._pump_handle = loop.call_later(self._min_frame_dt, self._pump)

    def _emit_frame(self, force: bool = False) -> None:
        s = self.session
        if not s:
            return
        try:
            t = s.terminal
            cur_x, cur_y = t.absolute_cursor()
            rows = t.styled_viewport()
            total = t.total_lines()
            visible = total - t.history_size
            title = t.title or ""
            if title != self._last_title:
                self._last_title = title
                self.title_changed.emit(title)
            self.data_ready.emit(Frame(
                rows=rows, cursor_x=cur_x, cursor_y=cur_y, title=title,
                history=t.history_size, total=total, visible=visible,
                flags=flags_from_terminal(t), alive=s.is_alive))
            self._last_emit = time.monotonic()
            self._dirty = False
        except Exception as e:
            print(f"[render] frame skipped: {e!r}", file=sys.stderr)

    # ── GUI → bg thread actions ────────────────────────────────────────

    def _schedule(self, coro_factory) -> None:
        if self.loop:
            self.loop.call_soon_threadsafe(
                lambda: asyncio.create_task(coro_factory()))

    def schedule_write(self, data: bytes) -> None:
        async def _w():
            if self.session:
                try:
                    await self.session.write_all(data)
                except Exception:
                    pass
        self._schedule(_w)

    def schedule_resize(self, rows: int, cols: int) -> None:
        async def _r():
            if self.session:
                try:
                    self.session.resize(rows, cols)
                    self.session.terminal.resize(rows, cols)
                    self._emit_frame(force=True)
                except Exception:
                    pass
        self._schedule(_r)

    def schedule_reset(self) -> None:
        async def _x():
            if self.session:
                try:
                    self.session.terminal.reset()
                    self.bells.reset()
                    self._emit_frame(force=True)
                except Exception:
                    pass
        self._schedule(_x)

    def schedule_interrupt(self) -> None:
        async def _i():
            if self.session:
                try:
                    self.session.interrupt()
                except Exception:
                    pass
        self._schedule(_i)

    def schedule_kill(self) -> None:
        async def _k():
            if self.session:
                try:
                    self.session.kill()
                except Exception:
                    pass
        self._schedule(_k)

    def schedule_restart(self) -> None:
        async def _re():
            try:
                if self.session:
                    try:
                        await self.session.terminate(grace_period=2.0)
                    except Exception:
                        pass
                self.session = await spawn(self.program, self.args,
                                           winsize=self.winsize)
                self.session.terminal.set_scrollback_lines(self.scrollback)
                self.bells.reset()
                asyncio.create_task(self._read_loop())
                self._emit_frame(force=True)
            except Exception:
                pass
        self._schedule(_re)

    def raw_text(self) -> bytes:
        try:
            return self.session.raw_output if self.session else b""
        except Exception:
            return b""

    def stop(self) -> None:
        loop = self.loop
        if loop and loop.is_running():
            def _shutdown():
                self._stopping = True
                if self._pump_handle is not None:
                    try:
                        self._pump_handle.cancel()
                    except Exception:
                        pass
                try:
                    if self.session:
                        self.session.kill()
                except Exception:
                    pass
                for task in asyncio.all_tasks(loop):
                    task.cancel()
                loop.call_soon(loop.stop)   # one iteration delivers the cancels
            loop.call_soon_threadsafe(_shutdown)
        self.thread.quit()
        if not self.thread.wait(3000):
            self.thread.terminate()


# ─────────────────────────────────────────────────────────────────────
# FindBar — incremental search UI docked under the terminal
# ─────────────────────────────────────────────────────────────────────

class FindBar(QWidget):
    search = Signal(str)
    next_match = Signal(bool)
    closed = Signal()

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        lay = QHBoxLayout(self)
        lay.setContentsMargins(6, 3, 6, 3)
        lay.setSpacing(6)
        self.edit = QLineEdit()
        self.edit.setPlaceholderText("Find…")
        self.edit.textChanged.connect(self.search.emit)
        self.edit.returnPressed.connect(lambda: self.next_match.emit(True))
        self.count = QLabel("")
        prev = QToolButton(); prev.setText("▲")
        prev.setToolTip("Previous (Shift+F3)")
        prev.clicked.connect(lambda: self.next_match.emit(False))
        nxt = QToolButton(); nxt.setText("▼")
        nxt.setToolTip("Next (F3)")
        nxt.clicked.connect(lambda: self.next_match.emit(True))
        close = QToolButton(); close.setText("✕")
        close.clicked.connect(self.closed.emit)
        lay.addWidget(self.edit, 1)
        lay.addWidget(self.count)
        lay.addWidget(prev)
        lay.addWidget(nxt)
        lay.addWidget(close)

    def keyPressEvent(self, event) -> None:
        if event.key() == Qt.Key.Key_Escape:
            self.closed.emit()
            return
        super().keyPressEvent(event)

    def set_count(self, idx: int, total: int) -> None:
        self.count.setText(f"{idx + 1}/{total}" if total else "0/0")


# ─────────────────────────────────────────────────────────────────────
# TerminalTab — one session: view + scrollbar + find bar + runner
# ─────────────────────────────────────────────────────────────────────

class TerminalTab(QWidget):
    title_changed = Signal(object, str)      # (self, title)
    status_changed = Signal(object, object)  # (self, Frame|None)
    process_exited = Signal(object, int)     # (self, exit_code)
    bell = Signal(object)                    # (self,)
    zoom_requested = Signal(int)

    def __init__(self, config: Config, program: str, args: list[str],
                 winsize: Winsize, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.cfg = config
        self.program = program
        self.title = program
        self.winsize = winsize

        self.vbar = QScrollBar(Qt.Orientation.Vertical)
        self.view = TerminalView(config, self.vbar)
        self.findbar = FindBar()
        self.findbar.hide()

        grid = QHBoxLayout()
        grid.setContentsMargins(0, 0, 0, 0)
        grid.setSpacing(0)
        grid.addWidget(self.view, 1)
        grid.addWidget(self.vbar)
        top = QVBoxLayout(self)
        top.setContentsMargins(0, 0, 0, 0)
        top.setSpacing(0)
        top.addLayout(grid, 1)
        top.addWidget(self.findbar)

        self.runner = _AsyncRunner(program, args, winsize,
                                   config.scrollback_lines)

        # The runner's thread runs an asyncio loop (run_forever), which blocks
        # that thread's Qt event loop — so a normal (queued) cross-thread signal
        # to schedule_write would never be delivered. A direct connection runs
        # schedule_write on the GUI thread, where it hands off to the asyncio
        # loop via the thread-safe call_soon_threadsafe.
        self.view.key_input.connect(
            self.runner.schedule_write,
            Qt.ConnectionType.DirectConnection)
        self.view.grid_resized.connect(self._on_grid_resized)
        self.view.zoom_requested.connect(self.zoom_requested.emit)
        self.runner.data_ready.connect(self._on_frame)
        self.runner.process_exited.connect(self._on_exit)
        self.runner.bell.connect(self._on_bell)
        self.runner.title_changed.connect(self._on_title)

        self.findbar.search.connect(self.view.set_search)
        self.findbar.next_match.connect(self.view.find_next)
        self.findbar.search.connect(lambda *_: self._update_find_count())
        self.findbar.next_match.connect(lambda *_: self._update_find_count())
        self.findbar.closed.connect(self.hide_find)

        # Some platforms (notably Windows) freeze queued-signal and paint
        # delivery during an interactive resize, which can leave the screen
        # showing a stale frame until the next output. This single-shot timer
        # fires once resizing settles — when the event loop is free again — to
        # request a fresh, correctly-sized frame and force a repaint.
        self._resize_timer = QTimer(self)
        self._resize_timer.setSingleShot(True)
        self._resize_timer.timeout.connect(self._resize_settled)

    # ── frame / signals ────────────────────────────────────────────────

    def _on_frame(self, frame: Frame) -> None:
        self.view.set_frame(frame)
        self.status_changed.emit(self, frame)
        if self.findbar.isVisible():
            self._update_find_count()

    def _on_title(self, title: str) -> None:
        self.title = title or self.program
        self.title_changed.emit(self, self.title)

    def _on_bell(self) -> None:
        self.view.show_bell()
        self.bell.emit(self)

    def _on_exit(self, code: int) -> None:
        self.process_exited.emit(self, code)

    def _on_grid_resized(self, rows: int, cols: int) -> None:
        # Debounce: record the target size and resize only once the drag
        # settles. Resizing on every intermediate size during a fast drag
        # thrashes the scrollback reflow and can corrupt history.
        self.winsize = Winsize(rows, cols, 0, 0)
        self._resize_timer.start(50)

    def _resize_settled(self) -> None:
        # Apply the final size once and force a repaint with the fresh frame.
        self.runner.schedule_resize(self.winsize.rows, self.winsize.cols)
        self.view.update()

    # ── find ────────────────────────────────────────────────────────────

    def show_find(self) -> None:
        self.findbar.show()
        self.findbar.edit.setFocus()
        self.findbar.edit.selectAll()
        if self.view.has_selection():
            sel = self.view.selected_text().split("\n")[0]
            if sel:
                self.findbar.edit.setText(sel)

    def hide_find(self) -> None:
        self.findbar.hide()
        self.view.clear_search()
        self.view.setFocus()

    def _update_find_count(self) -> None:
        self.findbar.set_count(self.view.match_index, self.view.match_count)

    # ── actions exposed to the window ──────────────────────────────────

    def copy(self) -> None: self.view.copy()
    def paste(self) -> None: self.view.paste()
    def select_all(self) -> None: self.view.select_all()
    def reset_terminal(self) -> None: self.runner.schedule_reset()
    def interrupt(self) -> None: self.runner.schedule_interrupt()
    def kill(self) -> None: self.runner.schedule_kill()
    def restart(self) -> None: self.runner.schedule_restart()

    def refresh_config(self) -> None:
        self.runner.scrollback = self.cfg.scrollback_lines
        self.view.set_config_changed()

    def save_output(self, path: str) -> None:
        try:
            with open(path, "w", encoding="utf-8") as fh:
                if self.view.frame:
                    for row in self.view.frame.rows:
                        fh.write("".join(
                            (c[0] if c[0] else " ") for c in row).rstrip())
                        fh.write("\n")
        except OSError as e:
            print(f"save failed: {e}", file=sys.stderr)

    def shutdown(self) -> None:
        self.runner.stop()


# ─────────────────────────────────────────────────────────────────────
# MainWindow — tabs, menus, shortcuts, settings
# ─────────────────────────────────────────────────────────────────────

class MainWindow(QMainWindow):
    ORG = "stitch-pty"
    APP = "terminal"

    def __init__(self, program: str, args: list[str],
                 winsize: Winsize, restore_shell: bool = True) -> None:
        super().__init__()
        self.program = program
        self.args = args
        self.winsize = winsize
        self.cfg = Config()
        self._saved_shell: tuple[str, list[str]] | None = None
        self._load_settings()

        # Restore the user's previously chosen default shell (unless launched
        # with an explicit one-off command).
        if restore_shell and self._saved_shell \
                and shell_available(self._saved_shell[0]):
            self.program, self.args = self._saved_shell[0], \
                list(self._saved_shell[1])

        self.tabs = QTabWidget()
        self.tabs.setTabsClosable(True)
        self.tabs.setMovable(True)
        self.tabs.setDocumentMode(True)
        self.tabs.tabCloseRequested.connect(self._close_tab)
        self.tabs.currentChanged.connect(self._on_tab_changed)
        self.setCentralWidget(self.tabs)

        self.statusBar()
        self._setup_menu()
        self._apply_style()

        self.resize(int(winsize.cols * 9) + 40, int(winsize.rows * 18) + 80)
        self.new_tab()

    # ── tabs ────────────────────────────────────────────────────────────

    def new_tab(self, program: str | None = None,
                args: list[str] | None = None) -> None:
        prog = program if program is not None else self.program
        targs = args if args is not None else self.args
        tab = TerminalTab(self.cfg, prog, list(targs),
                          Winsize(self.winsize.rows, self.winsize.cols, 0, 0))
        tab.title_changed.connect(self._on_tab_title)
        tab.status_changed.connect(self._on_status)
        tab.process_exited.connect(self._on_tab_exit)
        tab.bell.connect(self._on_tab_bell)
        tab.zoom_requested.connect(self._zoom)
        idx = self.tabs.addTab(tab, Path(prog).name or prog)
        self.tabs.setCurrentIndex(idx)
        tab.view.setFocus()

    def current_tab(self) -> TerminalTab | None:
        w = self.tabs.currentWidget()
        return w if isinstance(w, TerminalTab) else None

    def _close_tab(self, index: int) -> None:
        w = self.tabs.widget(index)
        if isinstance(w, TerminalTab):
            w.shutdown()
        self.tabs.removeTab(index)
        if self.tabs.count() == 0:
            self.close()

    def close_current_tab(self) -> None:
        self._close_tab(self.tabs.currentIndex())

    def next_tab(self) -> None:
        n = self.tabs.count()
        if n:
            self.tabs.setCurrentIndex((self.tabs.currentIndex() + 1) % n)

    def prev_tab(self) -> None:
        n = self.tabs.count()
        if n:
            self.tabs.setCurrentIndex((self.tabs.currentIndex() - 1) % n)

    def _on_tab_changed(self, index: int) -> None:
        tab = self.current_tab()
        if tab:
            tab.view.setFocus()
            self.setWindowTitle(f"{tab.title} — stitch-pty")
            self._on_status(tab, tab.view.frame)

    def _on_tab_title(self, tab: TerminalTab, title: str) -> None:
        idx = self.tabs.indexOf(tab)
        if idx >= 0:
            short = title if len(title) <= 24 else title[:23] + "…"
            self.tabs.setTabText(idx, short)
            self.tabs.setTabToolTip(idx, title)
        if tab is self.current_tab():
            self.setWindowTitle(f"{title} — stitch-pty")

    def _on_tab_bell(self, tab: TerminalTab) -> None:
        idx = self.tabs.indexOf(tab)
        if idx >= 0 and tab is not self.current_tab():
            self.tabs.setTabText(idx, "🔔 " + self.tabs.tabText(idx))

    def _on_tab_exit(self, tab: TerminalTab, code: int) -> None:
        idx = self.tabs.indexOf(tab)
        if idx >= 0:
            txt = self.tabs.tabText(idx).removeprefix("🔔 ")
            self.tabs.setTabText(idx, f"[exited] {txt}")
        if tab is self.current_tab():
            self.statusBar().showMessage(
                f"Process exited (code {code})" if code >= 0
                else "Process exited", 5000)

    def _on_status(self, tab, frame: Frame | None) -> None:
        if tab is not self.current_tab():
            return
        if not frame:
            self.statusBar().showMessage("")
            return
        sx = frame.cursor_x
        sy = frame.cursor_y - frame.history
        state = "running" if frame.alive else "exited"
        self.statusBar().showMessage(
            f"  {tab.view.cols}×{tab.view.rows}"
            f"   ·   cursor {sx},{sy}"
            f"   ·   scrollback {frame.history}"
            f"   ·   {state}"
        )

    # ── menus & shortcuts ───────────────────────────────────────────────

    def _act(self, text, shortcut, slot, checkable=False, checked=False):
        a = QAction(text, self)
        if shortcut:
            a.setShortcut(QKeySequence(shortcut))
        a.setCheckable(checkable)
        a.setChecked(checked)
        a.triggered.connect(slot)
        return a

    def _build_shell_menu(self, mb) -> None:
        self._shells = detect_shells()
        sh = mb.addMenu("&Shell")

        hdr = sh.addAction("Default for new tabs:")
        hdr.setEnabled(False)
        self._shell_group = QActionGroup(self)
        self._shell_group.setExclusive(True)
        cur = (self.program or "").lower()
        any_checked = False
        for label, prog, args in self._shells:
            a = QAction(label, self, checkable=True)
            checked = prog.lower() == cur
            any_checked = any_checked or checked
            a.setChecked(checked)
            a.triggered.connect(
                lambda _=False, p=prog, ar=args: self._set_default_shell(p, ar))
            self._shell_group.addAction(a)
            sh.addAction(a)
        # If the active program isn't among the detected shells, none is checked.
        if not any_checked and self._shell_group.actions():
            self._shell_group.actions()[0].setChecked(False)

        if self._shells:
            sh.addSeparator()
            newm = sh.addMenu("Open in New &Tab")
            for label, prog, args in self._shells:
                newm.addAction(self._act(
                    label, "",
                    lambda _=False, p=prog, ar=args: self.new_tab(p, ar)))

    def _set_default_shell(self, prog: str, args: list[str]) -> None:
        self.program = prog
        self.args = list(args)
        self._save_settings()
        self.statusBar().showMessage(
            f"New tabs will use: {prog}", 3000)

    def _setup_menu(self) -> None:
        mb = self.menuBar()

        m = mb.addMenu("&File")
        m.addAction(self._act("New &Tab", "Ctrl+Shift+T",
                              lambda: self.new_tab()))
        m.addAction(self._act("&Close Tab", "Ctrl+Shift+W",
                              self.close_current_tab))
        m.addSeparator()
        m.addAction(self._act("&Save Output…", "Ctrl+Shift+S",
                              self._save_output))
        m.addSeparator()
        m.addAction(self._act("E&xit", "Ctrl+Q", self.close))

        self._build_shell_menu(mb)

        e = mb.addMenu("&Edit")
        e.addAction(self._act("&Copy", "Ctrl+Shift+C",
                              lambda: self._on_tab(lambda t: t.copy())))
        e.addAction(self._act("&Paste", "Ctrl+Shift+V",
                              lambda: self._on_tab(lambda t: t.paste())))
        e.addAction(self._act("Select &All", "Ctrl+Shift+A",
                              lambda: self._on_tab(lambda t: t.select_all())))
        e.addSeparator()
        e.addAction(self._act("&Find…", "Ctrl+Shift+F", self._show_find))

        v = mb.addMenu("&View")
        v.addAction(self._act("Zoom &In", "Ctrl+=", lambda: self._zoom(1)))
        v.addAction(self._act("Zoom &Out", "Ctrl+-", lambda: self._zoom(-1)))
        v.addAction(self._act("&Reset Zoom", "Ctrl+0", self._zoom_reset))
        v.addSeparator()
        tm = v.addMenu("&Theme")
        grp = QActionGroup(self)
        grp.setExclusive(True)
        for name in THEMES:
            a = QAction(name, self, checkable=True)
            a.setChecked(name == self.cfg.theme_name)
            a.triggered.connect(lambda _=False, n=name: self._set_theme(n))
            grp.addAction(a)
            tm.addAction(a)

        p = mb.addMenu("&Terminal")
        p.addAction(self._act("&Interrupt (SIGINT)", "",
                              lambda: self._on_tab(lambda t: t.interrupt())))
        p.addAction(self._act("Re&set Terminal", "",
                              lambda: self._on_tab(lambda t: t.reset_terminal())))
        p.addAction(self._act("&Restart Process", "",
                              lambda: self._on_tab(lambda t: t.restart())))
        p.addAction(self._act("&Kill Process", "Ctrl+Shift+K",
                              lambda: self._on_tab(lambda t: t.kill())))
        p.addSeparator()
        self.a_copysel = self._act(
            "Copy on &Select", "", self._toggle_copy_on_select,
            checkable=True, checked=self.cfg.copy_on_select)
        self.a_abell = self._act(
            "&Audible Bell", "", self._toggle_audible_bell,
            checkable=True, checked=self.cfg.audible_bell)
        self.a_vbell = self._act(
            "&Visual Bell", "", self._toggle_visual_bell,
            checkable=True, checked=self.cfg.visual_bell)
        self.a_blink = self._act(
            "Cursor &Blink", "", self._toggle_blink,
            checkable=True, checked=self.cfg.cursor_blink)
        for a in (self.a_copysel, self.a_abell, self.a_vbell, self.a_blink):
            p.addAction(a)

        # Hidden tab-navigation shortcuts (don't reach the child).
        QShortcut(QKeySequence("Ctrl+Tab"), self, activated=self.next_tab)
        QShortcut(QKeySequence("Ctrl+Shift+Tab"), self,
                  activated=self.prev_tab)
        QShortcut(QKeySequence("Ctrl+PgUp"), self, activated=self.prev_tab)
        QShortcut(QKeySequence("Ctrl+PgDown"), self, activated=self.next_tab)
        QShortcut(QKeySequence("F3"), self,
                  activated=lambda: self._on_tab(
                      lambda t: t.view.find_next(True)))
        QShortcut(QKeySequence("Shift+F3"), self,
                  activated=lambda: self._on_tab(
                      lambda t: t.view.find_next(False)))

    def _on_tab(self, fn) -> None:
        tab = self.current_tab()
        if tab:
            fn(tab)

    def _show_find(self) -> None:
        self._on_tab(lambda t: t.show_find())

    def _save_output(self) -> None:
        tab = self.current_tab()
        if not tab:
            return
        path, _ = QFileDialog.getSaveFileName(
            self, "Save terminal output", "terminal-output.txt",
            "Text files (*.txt);;All files (*)")
        if path:
            tab.save_output(path)
            self.statusBar().showMessage(f"Saved to {path}", 3000)

    # ── config toggles ──────────────────────────────────────────────────

    def _zoom(self, delta: int) -> None:
        self.cfg.font_size = max(6.0, min(48.0, self.cfg.font_size + delta))
        self._refresh_all()

    def _zoom_reset(self) -> None:
        self.cfg.font_size = 12.0
        self._refresh_all()

    def _set_theme(self, name: str) -> None:
        self.cfg.theme_name = name
        self._apply_style()
        self._refresh_all()

    def _toggle_copy_on_select(self, on: bool) -> None:
        self.cfg.copy_on_select = on

    def _toggle_audible_bell(self, on: bool) -> None:
        self.cfg.audible_bell = on

    def _toggle_visual_bell(self, on: bool) -> None:
        self.cfg.visual_bell = on

    def _toggle_blink(self, on: bool) -> None:
        self.cfg.cursor_blink = on
        self._refresh_all()

    def _refresh_all(self) -> None:
        for i in range(self.tabs.count()):
            w = self.tabs.widget(i)
            if isinstance(w, TerminalTab):
                w.refresh_config()

    def _apply_style(self) -> None:
        th = self.cfg.theme
        self.setStyleSheet(f"""
            QMainWindow, QTabWidget::pane {{ background: {th.bg}; }}
            QStatusBar {{ background: {th.bg}; color: {th.fg}; }}
            QScrollBar:vertical {{ background: {th.bg}; width: 12px; }}
            QScrollBar::handle:vertical {{
                background: {th.sel_bg}; border-radius: 5px; min-height: 24px;
            }}
            QScrollBar::add-line, QScrollBar::sub-line {{ height: 0; }}
            QLineEdit {{
                background: {th.bg}; color: {th.fg};
                border: 1px solid {th.sel_bg}; padding: 3px;
            }}
            QLabel {{ color: {th.fg}; }}
        """)

    # ── settings persistence ────────────────────────────────────────────

    def _load_settings(self) -> None:
        s = QSettings(self.ORG, self.APP)
        self.cfg.font_size = float(s.value("font_size", self.cfg.font_size))
        self.cfg.theme_name = s.value("theme", self.cfg.theme_name)
        if self.cfg.theme_name not in THEMES:
            self.cfg.theme_name = DEFAULT_THEME
        self.cfg.copy_on_select = s.value("copy_on_select", False, type=bool)
        self.cfg.audible_bell = s.value("audible_bell", False, type=bool)
        self.cfg.visual_bell = s.value("visual_bell", True, type=bool)
        self.cfg.cursor_blink = s.value("cursor_blink", True, type=bool)
        sp = s.value("shell_program", "", type=str)
        if sp:
            sa = s.value("shell_args", [], type=list)
            self._saved_shell = (sp, [str(x) for x in (sa or [])])

    def _save_settings(self) -> None:
        s = QSettings(self.ORG, self.APP)
        s.setValue("font_size", self.cfg.font_size)
        s.setValue("theme", self.cfg.theme_name)
        s.setValue("copy_on_select", self.cfg.copy_on_select)
        s.setValue("audible_bell", self.cfg.audible_bell)
        s.setValue("visual_bell", self.cfg.visual_bell)
        s.setValue("cursor_blink", self.cfg.cursor_blink)
        s.setValue("shell_program", self.program)
        s.setValue("shell_args", list(self.args))

    # ── close ────────────────────────────────────────────────────────────

    def closeEvent(self, event) -> None:
        self._save_settings()
        for i in range(self.tabs.count()):
            w = self.tabs.widget(i)
            if isinstance(w, TerminalTab):
                w.shutdown()
        event.accept()


# ─────────────────────────────────────────────────────────────────────
# Entry point
# ─────────────────────────────────────────────────────────────────────

def get_default_shell() -> tuple[str, list[str]]:
    if IS_WINDOWS:
        return os.environ.get("COMSPEC", "cmd.exe"), []
    shell = os.environ.get("SHELL")
    if shell and Path(shell).exists():
        return shell, ["-i"]
    if Path("/bin/bash").exists():
        return "/bin/bash", ["-i"]
    return "/bin/sh", ["-i"]


def shell_available(program: str) -> bool:
    """Whether a shell program can be found (full path or on PATH)."""
    if not program:
        return False
    return Path(program).exists() or shutil.which(program) is not None


def detect_shells() -> list[tuple[str, str, list[str]]]:
    """Discover installed shells as (label, program, args) for this OS."""
    found: list[tuple[str, str, list[str]]] = []
    seen: set[str] = set()

    def add(label: str, prog: str, args: list[str]) -> None:
        if not prog:
            return
        resolved = prog if Path(prog).exists() else (shutil.which(prog) or "")
        if not resolved:
            return
        key = os.path.realpath(resolved).lower()   # collapse symlinks
        if key in seen:
            return
        seen.add(key)
        found.append((label, resolved, args))

    if IS_WINDOWS:
        add("Command Prompt",
            os.environ.get("COMSPEC", r"C:\Windows\System32\cmd.exe"), [])
        add("Windows PowerShell", "powershell.exe", ["-NoLogo"])
        add("PowerShell", "pwsh.exe", ["-NoLogo"])
        for cand in (r"C:\Program Files\Git\bin\bash.exe",
                     r"C:\Program Files (x86)\Git\bin\bash.exe"):
            if Path(cand).exists():
                add("Git Bash", cand, ["-i"])
                break
        add("WSL", "wsl.exe", [])
    else:
        candidates: list[str] = []
        env_shell = os.environ.get("SHELL")
        if env_shell:
            candidates.append(env_shell)
        try:
            with open("/etc/shells", encoding="utf-8") as f:
                for line in f:
                    line = line.strip()
                    if line and not line.startswith("#"):
                        candidates.append(line)
        except OSError:
            pass
        candidates += ["/bin/bash", "/usr/bin/zsh", "/bin/zsh",
                       "/usr/bin/fish", "/bin/fish", "/bin/dash",
                       "/usr/bin/tcsh", "/bin/sh"]
        for path in candidates:
            if path and Path(path).exists():
                add(Path(path).name, path, ["-i"])

    if not found:
        prog, args = get_default_shell()
        add(Path(prog).name or prog, prog, args)
    return found


def main() -> None:
    p = argparse.ArgumentParser(description="stitch-pty terminal (PySide6)")
    p.add_argument("--cmd", type=str, default=None, help="Run a command")
    p.add_argument("--rows", type=int, default=24)
    p.add_argument("--cols", type=int, default=80)
    a = p.parse_args()

    app = QApplication(sys.argv)
    app.setApplicationName("stitch-pty")
    app.setOrganizationName("stitch-pty")
    app.setStyle("fusion")

    # A child's interrupt (Ctrl+C / SIGINT) can reach this process through the
    # console process group — notably on Windows, where it would otherwise raise
    # KeyboardInterrupt in the GUI thread mid-paint. Ignore it here so it affects
    # only the child; the user quits the GUI by closing the window.
    try:
        signal.signal(signal.SIGINT, signal.SIG_IGN)
    except (ValueError, OSError):
        pass

    if a.cmd:
        if IS_WINDOWS:
            prog, args = os.environ.get("COMSPEC", "cmd.exe"), ["/c", a.cmd]
        else:
            prog, args = "/bin/sh", ["-c", a.cmd]
    else:
        prog, args = get_default_shell()

    win = MainWindow(prog, args, Winsize(a.rows, a.cols, 0, 0),
                     restore_shell=not bool(a.cmd))
    win.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
