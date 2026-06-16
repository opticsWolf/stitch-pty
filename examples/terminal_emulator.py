# -*- coding: utf-8 -*-
"""stitch-pty terminal emulator — PySide6 example.

A minimal but functional terminal emulator demonstrating:
- Async PTY spawn with integrated terminal emulation
- Real-time screen rendering via QPlainTextEdit (monospace)
- Cursor position, title, and scrollback tracking
- Keyboard input forwarding to the child process
- Window resize forwarding (SIGWINCH equivalent)
- asyncio + Qt event loop integration (background thread)

Usage:
    pip install PySide6 stitch-pty
    python terminal_emulator.py              # default shell
    python terminal_emulator.py --cmd "whoami"   # one-shot command
    python terminal_emulator.py --rows 30 --cols 100

Architecture:
    ┌──────────────────────────────────────────────────────┐
    │  MainWindow  (Qt event loop — main thread)            │
    │  ┌──────────────────────────────────────────────────┐ │
    │  │  TerminalWidget (focusable, captures keystrokes)  │ │
    │  │  Renders session.display (visible screen grid)    │ │
    │  └──────────────────────────────────────────────────┘ │
    │  ┌──────────────────────────────────────────────────┐ │
    │  │  StatusBar: cursor │ title │ history │ size       │ │
    │  └──────────────────────────────────────────────────┘ │
    └──────────────────────────────────────────────────────┘
         ▲  (Qt signals — carry plain Python data, thread-safe)
         │
    ┌────┴────────────────────────────────────────────────┐
    │  _AsyncRunner  (background QThread + asyncio loop)   │
    │  ┌────────────────────────────────────────────────┐  │
    │  │  PtySession (Rust, lives ONLY in bg thread)     │  │
    │  │  ├── read_timeout() → feed → TerminalState      │  │
    │  │  ├── write() ← scheduled via call_soon_threadsafe│  │
    │  │  └── terminal.display / cursor / title           │  │
    │  └────────────────────────────────────────────────┘  │
    │  Emits data_ready(lines, cursor_x, cursor_y, ...)    │
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

from PySide6.QtCore import QObject, Qt, QThread, QTimer, Signal
from PySide6.QtGui import QAction, QFont, QKeySequence
from PySide6.QtWidgets import (
    QApplication,
    QMainWindow,
    QTextEdit,
    QStatusBar,
    QVBoxLayout,
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


# Attribute bitmask layout (matches the packing in history.rs::styled_viewport)
_A_BOLD, _A_DIM, _A_ITALIC, _A_UNDERLINE = 1 << 0, 1 << 1, 1 << 2, 1 << 3
_A_BLINK, _A_REVERSE, _A_HIDDEN, _A_STRIKE = 1 << 4, 1 << 5, 1 << 6, 1 << 7


# ─────────────────────────────────────────────────────────────────────
# Data carried across the thread boundary (plain Python, no PyO3 refs)
# ─────────────────────────────────────────────────────────────────────

@dataclass(frozen=True)
class Frame:
    """One snapshot of terminal state, emitted by the bg thread.

    `rows` is the full buffer (scrollback history + visible screen) as styled
    cells: each cell is (text, fg, bg, attrs_bitmask). `cursor_y` is absolute
    (indexed into `rows`, i.e. history_size + on-screen cursor row).
    """
    rows:        list          # list[list[tuple[str, str, str, int]]]
    cursor_x:    int
    cursor_y:    int           # absolute row index into `rows`
    title:       str
    history:     int


# ─────────────────────────────────────────────────────────────────────
# TerminalWidget — focusable display + input capture
# ─────────────────────────────────────────────────────────────────────

class TerminalWidget(QTextEdit):
    """Focusable QTextEdit subclass: renders the screen and captures
    ALL keyboard input for forwarding to the PTY.

    Inherits QTextEdit so it gets focus naturally and receives
    keyPressEvent directly — no proxy tricks needed."""

    key_pressed = Signal(bytes)   # <-- forward keystrokes via signal

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setReadOnly(True)
        self.setLineWrapMode(QTextEdit.LineWrapMode.NoWrap)
        self.setUndoRedoEnabled(False)

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

    # ── rendering ───────────────────────────────────────────────

    def update_frame(self, frame: Frame) -> None:
        """Render one Frame snapshot (called on main thread)."""
        body = "\n".join(
            self._render_row(row, ridx, frame.cursor_x, frame.cursor_y)
            for ridx, row in enumerate(frame.rows)
        )
        html = (
            f'<pre style="margin:0; color:{DEFAULT_FG}; '
            f'background-color:{DEFAULT_BG};">{body}</pre>'
        )

        # Repainting the whole buffer resets the scrollbar, so remember where
        # the user was: stick to the bottom when following live output, but
        # keep their position if they've scrolled up into history.
        sb = self.verticalScrollBar()
        follow = sb.value() >= sb.maximum() - 4
        prev = sb.value()
        self.setHtml(html)
        sb.setValue(sb.maximum() if follow else min(prev, sb.maximum()))

    @staticmethod
    def _render_row(cells: list, ridx: int, cur_x: int, cur_y: int) -> str:
        """Render one row of styled cells to HTML, coalescing equal styles."""
        parts: list[str] = []
        run: list[str] = []
        run_style: str | None = None

        def flush() -> None:
            if run:
                parts.append(f'<span style="{run_style}">{_esc("".join(run))}</span>')
                run.clear()

        for cidx, (text, fg, bg, attrs) in enumerate(cells):
            f = _resolve_color(fg, DEFAULT_FG)
            b = _resolve_color(bg, DEFAULT_BG)
            if attrs & _A_REVERSE:
                f, b = b, f
            is_cursor = (ridx == cur_y and cidx == cur_x)
            if is_cursor:
                f, b = DEFAULT_FG, CURSOR_BG
            style = f"color:{f};background-color:{b};"
            if attrs & _A_BOLD:
                style += "font-weight:bold;"
            if attrs & _A_ITALIC:
                style += "font-style:italic;"
            if attrs & _A_UNDERLINE:
                style += "text-decoration:underline;"

            ch = text if text else " "
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
        return "".join(parts) or " "

    def update_plain(self, lines: list[str]) -> None:
        self.setPlainText("\n".join(lines))

    def show_welcome(self, program: str) -> None:
        """Colored splash shown at startup, before the child's first frame.

        The renderer is monochrome (it draws session.display, which is plain
        grid text), so the color comes from HTML spans here — not from the
        terminal emulation. The first real frame from the shell replaces it.
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
            '<span style="color:#c586c0;">Ctrl+= / Ctrl+-</span>'
            '<span style="color:#888888;"> font size</span>',
        ]
        self.setHtml('<pre style="margin:0;">' + "\n".join(lines) + "</pre>")

    # ── input capture ───────────────────────────────────────────

    def keyPressEvent(self, event) -> None:
        """Intercept ALL key events BEFORE QTextEdit processes them."""
        keystroke = _key_to_bytes(event)
        if keystroke:
            self.key_pressed.emit(keystroke)
        event.accept()  # eat it — don't let QTextEdit blink the cursor

    # ── helpers (kept for resize/font API) ──────────────────────

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

    All PyO3 / Rust state lives HERE.  The main thread only sees
    plain-Python Frame objects delivered via Qt signals.
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
        self.thread = QThread()
        self.moveToThread(self.thread)
        self.thread.started.connect(self._run_loop)
        self.thread.start()

    # ── event loop lifecycle ────────────────────────────────────

    def _run_loop(self) -> None:
        self.loop = asyncio.new_event_loop()
        asyncio.set_event_loop(self.loop)
        self.loop.run_until_complete(self._start())
        # Keep alive so schedule_write() callbacks are processed
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
        """Snapshot terminal state and emit as a Frame (bg thread)."""
        s = self.session
        if not s:
            return
        t = s.terminal
        cur_x, cur_y = t.absolute_cursor()        # (x, history + on-screen y)
        self.data_ready.emit(Frame(
            rows     = t.styled_viewport(),        # history + visible, styled cells
            cursor_x = cur_x,
            cursor_y = cur_y,
            title    = t.title,
            history  = t.history_size,
        ))

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
        self._settle_timer.setInterval(40)  # ms after the last drag tick — quick settle
        self._settle_timer.timeout.connect(self._forward_resize)

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
        self.terminal.key_pressed.connect(self.on_key)   # <-- connect signal
        self.setCentralWidget(self.terminal)
        self.terminal.setFocus()
        self.terminal.show_welcome(self.program)         # colored startup splash

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
        """Render a Frame on the main thread."""
        self.terminal.update_frame(frame)
        if frame.title:
            self.setWindowTitle(f"{frame.title} — stitch-pty")
        self._status(frame)

    def _on_exit(self) -> None:
        self.statusBar().showMessage("Process exited", 3000)

    def _status(self, frame: Frame) -> None:
        self.statusBar().showMessage(
            f"Cursor: {frame.cursor_x},{frame.cursor_y}"
            f"  │  Title: {frame.title}"
            f"  │  History: {frame.history}"
            f"  │  Size: {self.winsize.cols}x{self.winsize.rows}"
        )

    # ── input ───────────────────────────────────────────────────

    def on_key(self, keystroke: bytes) -> None:
        self.runner.schedule_write(keystroke)

    # ── resize ──────────────────────────────────────────────────

    def resizeEvent(self, event) -> None:
        super().resizeEvent(event)
        # Throttle: a live resize at most once per _resize_throttle while dragging.
        if time.monotonic() - self._last_resize_t >= self._resize_throttle:
            self._forward_resize()
        # Trailing: guarantees the final size lands ~40ms after motion stops.
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