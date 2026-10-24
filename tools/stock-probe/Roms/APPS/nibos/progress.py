#!/usr/bin/env python3
"""Shows what an install is doing, on both screens, while the stock menu is out of the way.

  progress.py TITLE LOG [STATUS]

LOG is followed as it grows; its last few lines are shown. STATUS, if given, is a file of one
line, "DONE TOTAL WHAT", rewritten as work goes on, shown as a bar. Runs until it is killed.
Uses the stock system's PySDL2 and SDL2_ttf; if they cannot be had, it just ends, and the
install goes on without it.
"""

import ctypes
import os
import sys
import time

os.environ.setdefault("PYSDL2_DLL_PATH", "/usr/lib")
os.environ.setdefault("SDL_VIDEODRIVER", "wayland")
os.environ.setdefault("XDG_RUNTIME_DIR", "/var/run")
os.environ.setdefault("WAYLAND_DISPLAY", "wayland-0")

import sdl2  # noqa: E402
import sdl2.sdlttf as ttf  # noqa: E402

FONTS = [
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
    "/mnt/vendor/bin/default.ttf",
]
WHITE, GREY, DIM, ACCENT = (255, 255, 255), (170, 170, 170), (90, 90, 90), (232, 53, 46)


def tail(path, lines):
    try:
        with open(path, "rb") as f:
            f.seek(0, 2)
            f.seek(max(0, f.tell() - 4096))
            text = f.read().decode(errors="replace")
    except OSError:
        return []
    return [line for line in text.splitlines() if line.strip()][-lines:]


def status(path):
    try:
        done, total, what = open(path).read().split(" ", 2)
        return int(done), max(1, int(total)), what.strip()
    except (OSError, ValueError):
        return None


class Screen:
    def __init__(self):
        if sdl2.SDL_Init(sdl2.SDL_INIT_VIDEO) or ttf.TTF_Init():
            raise RuntimeError(sdl2.SDL_GetError())
        bounds = []
        for i in range(sdl2.SDL_GetNumVideoDisplays()):
            r = sdl2.SDL_Rect()
            if sdl2.SDL_GetDisplayBounds(i, ctypes.byref(r)) == 0:
                bounds.append((r.x, r.y, r.w, r.h))
        if not bounds:
            raise RuntimeError("no displays")
        left = min(b[0] for b in bounds)
        top = min(b[1] for b in bounds)
        right = max(b[0] + b[2] for b in bounds)
        bottom = max(b[1] + b[3] for b in bounds)
        self.panels = [(b[0] - left, b[1] - top, b[2], b[3]) for b in bounds]
        self.window = sdl2.SDL_CreateWindow(b"dsboot", left, top, right - left, bottom - top,
                                            sdl2.SDL_WINDOW_BORDERLESS | sdl2.SDL_WINDOW_SHOWN)
        self.renderer = sdl2.SDL_CreateRenderer(self.window, -1, sdl2.SDL_RENDERER_ACCELERATED)
        if not self.window or not self.renderer:
            raise RuntimeError(sdl2.SDL_GetError())
        font = next((f for f in FONTS if os.path.exists(f)), None)
        if font is None:
            raise RuntimeError("no font")
        self.fonts = {size: ttf.TTF_OpenFont(font.encode(), size) for size in (22, 30, 56)}

    def text(self, words, size, x, y, colour, centre=False):
        if not words:
            return
        surface = ttf.TTF_RenderUTF8_Blended(self.fonts[size], words.encode(), sdl2.SDL_Color(*colour, 255))
        if not surface:
            return
        texture = sdl2.SDL_CreateTextureFromSurface(self.renderer, surface)
        w, h = surface.contents.w, surface.contents.h
        sdl2.SDL_FreeSurface(surface)
        sdl2.SDL_RenderCopy(self.renderer, texture, None, sdl2.SDL_Rect(x - w // 2 if centre else x, y, w, h))
        sdl2.SDL_DestroyTexture(texture)

    def box(self, x, y, w, h, colour):
        sdl2.SDL_SetRenderDrawColor(self.renderer, *colour, 255)
        sdl2.SDL_RenderFillRect(self.renderer, sdl2.SDL_Rect(x, y, w, h))

    def draw(self, title, lines, bar):
        sdl2.SDL_SetRenderDrawColor(self.renderer, 0, 0, 0, 255)
        sdl2.SDL_RenderClear(self.renderer)
        for px, py, pw, ph in self.panels:
            cx = px + pw // 2
            self.text(title, 56, cx, py + ph // 5, WHITE, centre=True)
            if bar:
                done, total, what = bar
                width = pw * 2 // 3
                x, y = cx - width // 2, py + ph // 5 + 110
                self.box(x, y, width, 14, DIM)
                self.box(x, y, width * min(done, total) // total, 14, ACCENT)
                self.text(what, 22, cx, y + 30, GREY, centre=True)
            y = py + ph // 2 + 40
            for line in lines:
                self.text(line[:70], 22, px + pw // 8, y, GREY)
                y += 32
        sdl2.SDL_RenderPresent(self.renderer)


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    title, log = sys.argv[1], sys.argv[2]
    state = sys.argv[3] if len(sys.argv) > 3 else None
    try:
        screen = Screen()
    except Exception as e:
        print(f"progress: no screen: {e}", file=sys.stderr)
        return
    event = sdl2.SDL_Event()
    while True:
        while sdl2.SDL_PollEvent(ctypes.byref(event)):
            pass
        screen.draw(title, tail(log, 6), status(state) if state else None)
        time.sleep(0.2)


if __name__ == "__main__":
    main()
