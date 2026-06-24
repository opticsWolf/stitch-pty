# -*- coding: utf-8 -*-
"""stitch-pty terminal emulator — PySide6 example (light, with scrollback).

A small but capable terminal emulator demonstrating:
- Async PTY spawn with integrated terminal emulation
- Windowed rendering via QTextEdit: only the visible slice of the buffer is
  fetched each frame (terminal.styled_range), so a huge scrollback stays cheap
- Real scrollback with an external scrollbar + wheel + Shift+PageUp/Down
- Full SGR attributes: bold, dim, italic, underline, blink, reverse, hidden,
  strikethrough
- Cursor position, title, and history tracking
- Keyboard input forwarding to the child process
- Window resize forwarding (SIGWINCH equivalent)
- asyncio + Qt event loop integration (background thread)

This is the "light" sibling of the full emulator: it borrows the advanced
viewport model (windowed frames + a follow/scroll handshake) but keeps a single
QTextEdit renderer and skips the heavier machinery (render pump, tabs, find,
selection model).

Usage:
    pip install PySide6 stitch-pty
    python terminal_emulator_simple.py              # default shell
    python terminal_emulator_simple.py --cmd "whoami"   # one-shot command
    python terminal_emulator_simple.py --rows 30 --cols 100

Architecture:
    ┌──────────────────────────────────────────────────────┐
    │  MainWindow  (Qt event loop — main thread)            │
    │  ┌───────────────────────────────────┐ ┌───┐         │
    │  │  TerminalWidget (captures keys)    │ │ s │ ← external
    │  │  Renders one window of styled_range │ │ b │   scrollbar
    │  │  (full color via HTML spans)        │ │ a │   = full
    │  └───────────────────────────────────┘ │ r │   history
    │  ┌──────────────────────────────────────────────────┐ │
    │  │  StatusBar: cursor │ title │ history │ view │ size │ │
    │  └──────────────────────────────────────────────────┘ │
    └──────────────────────────────────────────────────────┘
         ▲  (Qt signals — carry plain Python data, thread-safe)
         │  set_viewport(follow, top)  ▼  (handshake → bg thread)
    ┌────┴────────────────────────────────────────────────┐
    │  _AsyncRunner  (background QThread + asyncio loop)   │
    │  ┌────────────────────────────────────────────────┐  │
    │  │  PtySession (Rust, lives ONLY in bg thread)     │  │
    │  │  ├── read_timeout() → feed → TerminalState      │  │
    │  │  ├── write() ← scheduled via call_soon_threadsafe│  │
    │  │  └── terminal.styled_range(top, n) / cursor / …  │  │
    │  └────────────────────────────────────────────────┘  │
    │  Emits data_ready(Frame: window of styled cells)    │
    └──────────────────────────────────────────────────────┘
"""

from __future__ import annotations

import argparse
import asyncio
import platform
import sys
import time
from pathlib import Path
from dataclasses import dataclass

from PySide6.QtCore import Qt, QObject, QThread, QTimer, Signal
from PySide6.QtGui import QAction, QFont, QKeySequence
from PySide6.QtWidgets import (
    QApplication,
    QHBoxLayout,
    QMainWindow,
    QScrollBar,
    QTextEdit,
    QWidget,
)

from stitch_pty import PtyError, PtySession, Winsize, spawn


# ─────────────────────────────────────────────────────────────────────
# Palette
# ─────────────────────────────────────────────────────────────────────

DEFAULT_FG = "#d0d0d0"
DEFAULT_BG = "#1e1e1e"
CURSOR_BG  = "#555555"
STATUS_FG  = "#aaaaaa"
STATUS_BG  = "#2a2a2a"

# The Rust cell stores fg/bg as strings: "default", an ANSI color name
# (matching graphics.rs — note 33/43 are "brown", not "yellow"), or a raw
# 6-hex-digit string from the 256-color / 24-bit paths. Map them to CSS.
_ANSI_COLORS = {
    "black": "#1e1e1e", "red": "#cd3131", "green": "#0dbc79", "brown": "#a87f32",
    "yellow": "#e5e510", "blue": "#2472c8", "magenta": "#bc3fbc", "cyan": "#11a8cd",
    "white": "#e5e5e5",
    "brightblack": "#666666", "brightred": "#f14c4c", "brightgreen": "#23d18b",
    "brightbrown": "#d7a35b", "brightyellow": "#f5f543", "brightblue": "#3b8eea",
    "brightmagenta": "#d670d6", "brightcyan": "#29b8db", "brightwhite": "#ffffff",
}
_HEXDIGITS = set("0123456789abcdefABCDEF")


def _resolve_color(color: str, default: str) -> str:
    """Map a Rust cell color string to a CSS color."""
    if not color or color == "default":
        return default
    if color in _ANSI_COLORS:
        return _ANSI_COLORS[color]
    if len(color) == 6 and all(c in _HEXDIGITS for c in color):
        return "#" + color           # 256-color / 24-bit, stored without '#'
    return default


def _dim_color(css: str) -> str:
    """Darken a resolved #rrggbb color toward black (SGR 2 / dim)."""
    if len(css) != 7 or not css.startswith("#"):
        return css
    try:
        r, g, b = int(css[1:3], 16), int(css[3:5], 16), int(css[5:7], 16)
    except ValueError:
        return css
    f = 0.55
    return f"#{int(r * f):02x}{int(g * f):02x}{int(b * f):02x}"


# Attribute bitmask layout (matches the packing in history.rs::styled_viewport)
_A_BOLD, _A_DIM, _A_ITALIC, _A_UNDERLINE = 1 << 0, 1 << 1, 1 << 2, 1 << 3
_A_BLINK, _A_REVERSE, _A_HIDDEN, _A_STRIKE = 1 << 4, 1 << 5, 1 << 6, 1 << 7


# ─────────────────────────────────────────────────────────────────────
# Data carried across the thread boundary (plain Python, no PyO3 refs)
# ─────────────────────────────────────────────────────────────────────

@dataclass(frozen=True)
class Frame:
    """One snapshot of terminal state, emitted by the bg thread.

    `rows` is only the WINDOW currently in view (not the whole buffer): a list
    of styled rows, each a list of (text, fg, bg, attrs_bitmask) cells.
    `window_start` is the absolute index of the first row in `rows`; add the
    local row index to it to get the absolute row. `cursor_y` is absolute
    (history_size + on-screen cursor row), so compare it against
    `window_start + local_index`.
    """
    rows:         list   # list[list[tuple[str, str, str, int]]]
    cursor_x:     int
    cursor_y:     int    # absolute row index into the full buffer
    title:        str
    history:      int
    window_start: int    # absolute index of rows[0]
    total:        int    # total_lines in the full buffer
    view_rows:    int    # number of on-screen rows (== window height)


# ─────────────────────────────────────────────────────────────────────
# TerminalWidget — focusable display + input capture
# ─────────────────────────────────────────────────────────────────────

class TerminalWidget(QTextEdit):
    """Focusable QTextEdit subclass: renders one window of the buffer and
    captures ALL keyboard input for forwarding to the PTY.

    Scrolling is driven externally (MainWindow owns a QScrollBar that spans the
    whole history); this widget only ever holds the visible window, so its own
    vertical scrollbar is disabled.
    """

    key_pressed  = Signal(bytes)   # forward keystrokes via signal
    scroll_lines = Signal(int)     # local scrollback request (+down / -up)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setReadOnly(True)
        self.setLineWrapMode(QTextEdit.LineWrapMode.NoWrap)
        self.setUndoRedoEnabled(False)
        self.setVerticalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)

        font = QFont("Cascadia Code")
        if not font.exactMatch():
            for fb in ("Fira Code", "JetBrains Mono",
                       "Consolas", "Courier New", "monospace"):
                font = QFont(fb)
                if font.exactMatch():
                    break
        font.setPointSize(11)
        self.setFont(font)

        self.setStyleSheet(f"""
            QTextEdit {{
                background-color: {DEFAULT_BG};
                color:            {DEFAULT_FG};
                border: none;
                padding: 2px;
                selection-background-color: #333399;
                selection-color:            {DEFAULT_FG};
            }}
        """)

        # Blink: a single timer toggles a phase; we re-render the cached frame
        # only when it actually contains blinking cells (cheap when it doesn't).
        self._last_frame: Frame | None = None
        self._blink_on = True
        self._has_blink = False
        self._blink_timer = QTimer(self)
        self._blink_timer.setInterval(500)
        self._blink_timer.timeout.connect(self._toggle_blink)
        self._blink_timer.start()

    # ── rendering ───────────────────────────────────────────────

    def update_frame(self, frame: Frame) -> None:
        """Cache and render one Frame snapshot (called on main thread)."""
        self._last_frame = frame
        self._render()

    def _render(self) -> None:
        frame = self._last_frame
        if frame is None:
            return
        has_blink = False
        out: list[str] = []
        for i, row in enumerate(frame.rows):
            line, row_blink = self._render_row(
                row, frame.window_start + i,
                frame.cursor_x, frame.cursor_y, self._blink_on)
            has_blink = has_blink or row_blink
            out.append(line)
        self._has_blink = has_blink
        html = (
            f'<pre style="margin:0; color:{DEFAULT_FG}; '
            f'background-color:{DEFAULT_BG};">{chr(10).join(out)}</pre>'
        )
        self.setHtml(html)
        # The widget holds exactly the window, so keep it pinned to the top;
        # the external scrollbar is what actually moves through history.
        self.verticalScrollBar().setValue(0)

    def _toggle_blink(self) -> None:
        self._blink_on = not self._blink_on
        if self._has_blink and self._last_frame is not None:
            self._render()

    @staticmethod
    def _render_row(cells: list, abs_ridx: int, cur_x: int, cur_y: int,
                    blink_on: bool) -> tuple[str, bool]:
        """Render one row of styled cells to HTML, coalescing equal styles.

        Returns (html, row_has_blink).
        """
        parts: list[str] = []
        run: list[str] = []
        run_style: str | None = None
        row_has_blink = False

        def flush() -> None:
            if run:
                parts.append(f'<span style="{run_style}">{_esc("".join(run))}</span>')
                run.clear()

        for cidx, (text, fg, bg, attrs) in enumerate(cells):
            f = _resolve_color(fg, DEFAULT_FG)
            b = _resolve_color(bg, DEFAULT_BG)
            if attrs & _A_REVERSE:
                f, b = b, f
            if attrs & _A_DIM:
                f = _dim_color(f)

            hidden = bool(attrs & _A_HIDDEN)
            if attrs & _A_BLINK:
                row_has_blink = True
                if not blink_on:        # blink "off" phase → make it vanish
                    f = b

            is_cursor = (abs_ridx == cur_y and cidx == cur_x)
            if is_cursor:
                f, b, hidden = DEFAULT_FG, CURSOR_BG, False

            style = f"color:{f};background-color:{b};"
            if attrs & _A_BOLD:
                style += "font-weight:bold;"
            if attrs & _A_ITALIC:
                style += "font-style:italic;"
            deco = []
            if attrs & _A_UNDERLINE:
                deco.append("underline")
            if attrs & _A_STRIKE:
                deco.append("line-through")
            if deco:
                style += f"text-decoration:{' '.join(deco)};"

            ch = " " if hidden else (text if text else " ")
            if is_cursor:                       # cursor cell is always its own span
                flush()
                parts.append(f'<span style="{style}">{_esc(ch)}</span>')
                run_style = None
            else:
                if style != run_style:
                    flush()
                    run_style = style
                run.append(ch)
        flush()
        return ("".join(parts) or " ", row_has_blink)

    def show_welcome(self, program: str) -> None:
        """Colored splash shown at startup, before the child's first frame.

        Just a placeholder until the shell produces output; the first real
        frame replaces it.
        """
        lines = [
            '<span style="color:#4ec9b0; font-weight:bold;">stitch-pty terminal</span>',
            '<span style="color:#888888;">async PTY with integrated terminal emulation</span>',
            "",
            f'<span style="color:#d0d0d0;">starting </span>'
            f'<span style="color:#6a9955; font-weight:bold;">{_esc(program)}</span>'
            f'<span style="color:#d0d0d0;"> …</span>',
            "",
            '<span style="color:#c586c0;">Ctrl+Q</span>'
            '<span style="color:#888888;"> quit'
            '   ·   </span>'
            '<span style="color:#c586c0;">Ctrl+Shift+K</span>'
            '<span style="color:#888888;"> kill process'
            '   ·   </span>'
            '<span style="color:#c586c0;">Shift+PgUp/PgDn</span>'
            '<span style="color:#888888;"> scrollback'
            '   ·   </span>'
            '<span style="color:#c586c0;">Ctrl+= / Ctrl+-</span>'
            '<span style="color:#888888;"> font size</span>',
        ]
        self.setHtml('<pre style="margin:0;">' + "\n".join(lines) + "</pre>")

    # ── input capture ───────────────────────────────────────────

    def keyPressEvent(self, event) -> None:
        """Intercept ALL key events BEFORE QTextEdit processes them."""
        mods = event.modifiers()
        key = event.key()

        # Shift+PageUp / Shift+PageDown scroll the local scrollback instead of
        # being sent to the child (matches common terminal UX).
        if (mods & Qt.KeyboardModifier.ShiftModifier) and key in (
                Qt.Key.Key_PageUp, Qt.Key.Key_PageDown):
            step = self._page_lines()
            self.scroll_lines.emit(-step if key == Qt.Key.Key_PageUp else step)
            event.accept()
            return

        keystroke = _key_to_bytes(event)
        if keystroke:
            self.key_pressed.emit(keystroke)
        event.accept()  # eat it — don't let QTextEdit blink its own cursor

    def wheelEvent(self, event) -> None:
        dy = event.angleDelta().y()
        if dy:
            self.scroll_lines.emit(-3 if dy > 0 else 3)
        event.accept()

    # ── helpers (kept for resize/font/scroll API) ───────────────

    def _page_lines(self) -> int:
        lh = max(1, self.fontMetrics().lineSpacing())
        return max(1, self.contentsRect().height() // lh - 1)

    def get_char_size(self) -> tuple[int, int]:
        fm = self.fontMetrics()
        return fm.horizontalAdvance("M"), fm.lineSpacing()

    def get_contents_rect(self):
        return self.contentsRect()


def _esc(s: str) -> str:
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


# ─────────────────────────────────────────────────────────────────────
# Key → PTY byte translation
# ─────────────────────────────────────────────────────────────────────

def _key_to_bytes(event) -> bytes | None:
    key     = event.key()
    text    = event.text()
    ctrl    = bool(event.modifiers() & Qt.KeyboardModifier.ControlModifier)

    special = {
        Qt.Key.Key_Up, Qt.Key.Key_Down, Qt.Key.Key_Left, Qt.Key.Key_Right,
        Qt.Key.Key_Home, Qt.Key.Key_End, Qt.Key.Key_PageUp,
        Qt.Key.Key_PageDown, Qt.Key.Key_Backspace, Qt.Key.Key_Delete,
        Qt.Key.Key_Insert, Qt.Key.Key_Tab, Qt.Key.Key_Escape,
        Qt.Key.Key_Return, Qt.Key.Key_Enter,
    }

    # Printable
    if text and key not in special:
        if ctrl:
            ch = text.upper()
            if ch.isalpha() and len(ch) == 1:
                return bytes([ord(ch) - 0x40])
        return text.encode("utf-8", errors="replace")

    # Arrows / navigation
    if key == Qt.Key.Key_Up:    return b"\x1b[A"
    if key == Qt.Key.Key_Down:  return b"\x1b[B"
    if key == Qt.Key.Key_Right: return b"\x1b[C"
    if key == Qt.Key.Key_Left:  return b"\x1b[D"
    if key == Qt.Key.Key_Home:  return b"\x1b[H"
    if key == Qt.Key.Key_End:   return b"\x1b[F"
    if key == Qt.Key.Key_PageUp:    return b"\x1b[5~"
    if key == Qt.Key.Key_PageDown:  return b"\x1b[6~"
    if key == Qt.Key.Key_Backspace: return b"\x7f"
    if key == Qt.Key.Key_Delete:    return b"\x1b[3~"
    if key == Qt.Key.Key_Insert:    return b"\x1b[2~"
    if key == Qt.Key.Key_Tab:       return b"\t"
    if key in (Qt.Key.Key_Return, Qt.Key.Key_Enter): return b"\r"
    if key == Qt.Key.Key_Escape:    return b"\x1b"

    # Ctrl shortcuts
    if ctrl and key == Qt.Key.Key_C: return b"\x03"
    if ctrl and key == Qt.Key.Key_D: return b"\x04"
    if ctrl and key == Qt.Key.Key_L: return b"\x0c"

    return None


# ─────────────────────────────────────────────────────────────────────
# _AsyncRunner — background thread with asyncio loop + PtySession
# ─────────────────────────────────────────────────────────────────────

class _AsyncRunner(QObject):
    """Runs asyncio + PtySession in a background QThread.

    All PyO3 / Rust state lives HERE. The main thread only sees plain-Python
    Frame objects delivered via Qt signals, and pushes viewport changes back
    via set_viewport().
    """

    data_ready     = Signal(object)   # Frame
    process_exited = Signal()

    def __init__(self, program: str, args: list[str],
                 winsize: Winsize) -> None:
        super().__init__()
        self.program = program
        self.args    = args
        self.winsize = winsize
        self.session: PtySession | None = None
        self.loop: asyncio.AbstractEventLoop | None = None

        # Viewport state (mutated only on the loop thread via set_viewport).
        self._follow = True
        self._view_top = 0
        self.view_rows = winsize.rows
        self._styled_range_ok: bool | None = None   # capability probe (once)

        self.thread = QThread()
        self.moveToThread(self.thread)
        self.thread.started.connect(self._run_loop)
        self.thread.start()

    # ── event loop lifecycle ────────────────────────────────────

    def _run_loop(self) -> None:
        self.loop = asyncio.new_event_loop()
        asyncio.set_event_loop(self.loop)
        self.loop.run_until_complete(self._start())
        # Keep alive so schedule_write() / set_viewport() callbacks run.
        self.loop.run_forever()

    async def _start(self) -> None:
        self.session = await spawn(
            self.program, self.args, winsize=self.winsize)
        asyncio.create_task(self._read_loop())

    # ── read loop ───────────────────────────────────────────────

    async def _read_loop(self) -> None:
        assert self.session is not None
        while True:
            try:
                data = await self.session.read_timeout(4096, 0.05)
            except asyncio.CancelledError:
                break
            except PtyError:
                # A 0.05s read timeout is the NORMAL idle case for an
                # interactive shell sitting at a prompt — NOT a failure.
                # (Timeout maps to stitch_pty.IOError, a PtyError subclass.)
                data = None
            except Exception:
                # A genuine I/O error (pipe gone) maps to builtin OSError,
                # which is not a PtyError — that one really is fatal.
                break

            if data:
                try:
                    self._emit_frame()
                except Exception as e:
                    print(f"[render] frame skipped: {e!r}", file=sys.stderr)
            elif not self.session.is_alive:
                # No data and the child has exited: stop polling.
                break
        self.process_exited.emit()

    def _emit_frame(self) -> None:
        """Snapshot the visible WINDOW of terminal state and emit it."""
        s = self.session
        if not s:
            return
        t = s.terminal
        total = t.total_lines()
        view = self.view_rows

        # Follow the live tail unless the user has scrolled up into history.
        if self._follow:
            top = max(0, total - view)
        else:
            top = max(0, min(self._view_top, max(0, total - view)))

        # Probe styled_range once; fall back to slicing the full viewport if a
        # build predates it (keeps the example working on older cores).
        if self._styled_range_ok is None:
            self._styled_range_ok = hasattr(t, "styled_range")
            if not self._styled_range_ok:
                print("[warn] TerminalState.styled_range not found — using full "
                      "styled_viewport() (slower). Rebuild the extension to enable "
                      "windowed rendering.", file=sys.stderr)

        if self._styled_range_ok:
            rows = t.styled_range(top, view)
        else:
            rows = t.styled_viewport()[top:top + view]

        cur_x, cur_y = t.absolute_cursor()        # (x, history + on-screen y)
        self.data_ready.emit(Frame(
            rows         = rows,
            cursor_x     = cur_x,
            cursor_y     = cur_y,
            title        = t.title,
            history      = t.history_size,
            window_start = top,
            total        = total,
            view_rows    = view,
        ))

    # ── viewport handshake (called from main thread) ────────────

    def set_viewport(self, follow: bool, top: int) -> None:
        if self.loop:
            self.loop.call_soon_threadsafe(self._apply_viewport, follow, top)

    def _apply_viewport(self, follow: bool, top: int) -> None:
        self._follow = follow
        self._view_top = top
        self._emit_frame()   # repaint immediately at the new position

    # ── write (scheduled from main thread) ──────────────────────

    async def _do_write(self, data: bytes) -> None:
        if self.session:
            await self.session.write(data)

    def schedule_write(self, data: bytes) -> None:
        if self.loop:
            self.loop.call_soon_threadsafe(
                lambda: asyncio.create_task(self._do_write(data)))

    # ── resize (called from main thread, forwarded to bg) ───────

    def schedule_resize(self, rows: int, cols: int) -> None:
        if self.loop:
            self.loop.call_soon_threadsafe(
                lambda: asyncio.create_task(self._do_resize(rows, cols)))

    async def _do_resize(self, rows: int, cols: int) -> None:
        if self.session:
            self.session.resize(rows, cols)
            self.session.terminal.resize(rows, cols)
            self.view_rows = rows
            self._emit_frame()  # refresh after resize

    # ── kill ────────────────────────────────────────────────────

    def schedule_kill(self) -> None:
        if self.loop:
            self.loop.call_soon_threadsafe(
                lambda: asyncio.create_task(self._do_kill()))

    async def _do_kill(self) -> None:
        if self.session:
            try:
                self.session.kill()
            except PtyError:
                pass  # already dead

    # ── teardown ────────────────────────────────────────────────

    def stop(self) -> None:
        if self.loop:
            self.loop.call_soon_threadsafe(self.loop.stop)
        self.thread.quit()
        self.thread.wait(2000)


# ─────────────────────────────────────────────────────────────────────
# MainWindow
# ─────────────────────────────────────────────────────────────────────

class MainWindow(QMainWindow):

    def __init__(self, program: str, args: list[str],
                 winsize: Winsize) -> None:
        super().__init__()
        self.program = program
        self.args    = args
        self.winsize = winsize

        self.runner = _AsyncRunner(program, args, winsize)

        # Resize handling = throttle (live updates during the drag, rate-limited
        # so conhost isn't flooded) + a short trailing timer (so the FINAL size
        # lands quickly once motion stops). See resizeEvent / _forward_resize.
        self._resize_throttle = 0.10        # s: at most one live resize this often
        self._last_resize_t = 0.0
        self._settle_timer = QTimer(self)
        self._settle_timer.setSingleShot(True)
        self._settle_timer.setInterval(40)  # ms after the last drag tick
        self._settle_timer.timeout.connect(self._forward_resize)

        # Guard so programmatic scrollbar updates don't echo back as user scrolls.
        self._syncing_scrollbar = False

        self._setup_ui()
        self._setup_menu()
        self.runner.data_ready.connect(self._on_frame)
        self.runner.process_exited.connect(self._on_exit)

    # ── UI ──────────────────────────────────────────────────────

    def _setup_ui(self) -> None:
        ws = self.winsize
        self.resize(ws.cols * 8 + 16, ws.rows * 18 + 60)
        self.setWindowTitle(f"stitch-pty: {self.program}")

        self.terminal = TerminalWidget()
        self.terminal.key_pressed.connect(self.on_key)
        self.terminal.scroll_lines.connect(self._scroll_by)

        # External scrollbar spans the whole history; the widget shows a window.
        self.vbar = QScrollBar(Qt.Orientation.Vertical)
        self.vbar.valueChanged.connect(self._on_scroll)

        container = QWidget()
        lay = QHBoxLayout(container)
        lay.setContentsMargins(0, 0, 0, 0)
        lay.setSpacing(0)
        lay.addWidget(self.terminal)
        lay.addWidget(self.vbar)
        self.setCentralWidget(container)

        self.terminal.setFocus()
        self.terminal.show_welcome(self.program)

        self.statusBar().setStyleSheet(f"""
            QStatusBar {{ background-color: {STATUS_BG}; color: {STATUS_FG}; }}
        """)

    def _setup_menu(self) -> None:
        mb = self.menuBar()

        fm = mb.addMenu("&File")
        ka = QAction("Kill &Process", self)
        ka.setShortcut(QKeySequence("Ctrl+Shift+K"))
        ka.triggered.connect(self._on_kill)
        fm.addAction(ka)
        fm.addSeparator()
        qa = QAction("E&xit", self)
        qa.setShortcut(QKeySequence("Ctrl+Q"))
        qa.triggered.connect(self.close)
        fm.addAction(qa)

        vm = mb.addMenu("&View")
        ba = QAction("Text &Bigger", self)
        ba.setShortcut(QKeySequence("Ctrl+="))
        ba.triggered.connect(lambda: self._font_delta(1))
        vm.addAction(ba)
        sa = QAction("Text &Smaller", self)
        sa.setShortcut(QKeySequence("Ctrl+-"))
        sa.triggered.connect(lambda: self._font_delta(-1))
        vm.addAction(sa)

    # ── signal handlers ─────────────────────────────────────────

    def _on_frame(self, frame: Frame) -> None:
        """Render a Frame on the main thread and sync the scrollbar to it."""
        self.terminal.update_frame(frame)

        # Reflect the emitted window in the external scrollbar without echoing.
        self._syncing_scrollbar = True
        maxv = max(0, frame.total - frame.view_rows)
        self.vbar.setRange(0, maxv)
        self.vbar.setPageStep(max(1, frame.view_rows))
        self.vbar.setSingleStep(1)
        self.vbar.setValue(frame.window_start)
        self._syncing_scrollbar = False

        if frame.title:
            self.setWindowTitle(f"{frame.title} — stitch-pty")
        self._status(frame)

    def _on_exit(self) -> None:
        self.statusBar().showMessage("Process exited", 3000)

    def _status(self, frame: Frame) -> None:
        maxv = max(0, frame.total - frame.view_rows)
        where = "live" if frame.window_start >= maxv else f"{frame.window_start}/{maxv}"
        self.statusBar().showMessage(
            f"Cursor: {frame.cursor_x},{frame.cursor_y}"
            f"  │  Title: {frame.title}"
            f"  │  History: {frame.history}"
            f"  │  View: {where}"
            f"  │  Size: {self.winsize.cols}x{self.winsize.rows}"
        )

    # ── scrollback ──────────────────────────────────────────────

    def _on_scroll(self, value: int) -> None:
        """User moved the scrollbar (drag, click, wheel, or PageUp/Down)."""
        if self._syncing_scrollbar:
            return
        follow = value >= self.vbar.maximum()
        self.runner.set_viewport(follow, value)

    def _scroll_by(self, delta_lines: int) -> None:
        """Wheel / Shift+PageUp-Down: nudge the scrollbar (drives _on_scroll)."""
        new = self.vbar.value() + delta_lines
        new = max(self.vbar.minimum(), min(new, self.vbar.maximum()))
        self.vbar.setValue(new)

    # ── input ───────────────────────────────────────────────────

    def on_key(self, keystroke: bytes) -> None:
        self.runner.schedule_write(keystroke)

    # ── resize ──────────────────────────────────────────────────

    def resizeEvent(self, event) -> None:
        super().resizeEvent(event)
        if time.monotonic() - self._last_resize_t >= self._resize_throttle:
            self._forward_resize()
        self._settle_timer.start()

    def _forward_resize(self) -> None:
        self._last_resize_t = time.monotonic()
        cw, ch = self.terminal.get_char_size()
        rect = self.terminal.get_contents_rect()
        cols = max(20, rect.width() // cw)
        rows = max(5,  rect.height() // ch)
        if cols != self.winsize.cols or rows != self.winsize.rows:
            self.winsize = Winsize(rows, cols, 0, 0)
            self.runner.schedule_resize(rows, cols)

    # ── font ────────────────────────────────────────────────────

    def _font_delta(self, d: int) -> None:
        f = self.terminal.font()
        f.setPointSize(max(6, f.pointSize() + d))
        self.terminal.setFont(f)
        self._forward_resize()

    # ── menu ────────────────────────────────────────────────────

    def _on_kill(self) -> None:
        self.runner.schedule_kill()
        self.statusBar().showMessage("Process killed", 3000)

    # ── close ───────────────────────────────────────────────────

    def closeEvent(self, event) -> None:
        self.runner.schedule_kill()
        self.runner.stop()
        event.accept()


# ─────────────────────────────────────────────────────────────────────
# Entry point
# ─────────────────────────────────────────────────────────────────────

def get_default_shell() -> tuple[str, list[str]]:
    if platform.system() == "Windows":
        return "cmd.exe", []
    if Path("/bin/bash").exists():
        return "/bin/bash", ["-l"]
    return "/bin/sh", ["-l"]


def main() -> None:
    p = argparse.ArgumentParser(description="stitch-pty terminal (PySide6)")
    p.add_argument("--cmd", type=str, default=None,
                   help="One-shot command")
    p.add_argument("--rows", type=int, default=24)
    p.add_argument("--cols", type=int, default=80)
    args = p.parse_args()

    app = QApplication(sys.argv)
    app.setApplicationName("stitch-pty")
    app.setStyle("fusion")

    if args.cmd:
        if platform.system() == "Windows":
            prog, a = "cmd.exe", ["/c", args.cmd]
        else:
            prog, a = "bash", ["-c", args.cmd]
    else:
        prog, a = get_default_shell()

    ws = Winsize(args.rows, args.cols, 0, 0)
    window = MainWindow(prog, a, ws)
    window.show()

    sys.exit(app.exec())


if __name__ == "__main__":
    main()