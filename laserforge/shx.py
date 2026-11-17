"""AutoCAD SHX single-line fonts ("AutoCAD-86 shapes 1.0/1.1" and "unifont 1.0").

Shapes are byte-coded vector programs (length/direction nibbles plus special codes for pen up/down,
scaling, stack, sub-shapes, displacements and arcs), as described in AutoCAD's shape-definition reference.
"""

import math
import os
import struct

DIRS = [(1, 0), (1, .5), (1, 1), (.5, 1), (0, 1), (-.5, 1), (-1, 1), (-1, .5),
        (-1, 0), (-1, -.5), (-1, -1), (-.5, -1), (0, -1), (.5, -1), (1, -1), (1, -.5)]


def _s8(b):
    return b - 256 if b > 127 else b


class ShxFont:
    def __init__(self, path):
        with open(path, "rb") as f:
            data = f.read()
        head_end = data.index(b"\x1a") + 1
        header = data[:head_end].decode("latin-1")
        self.unifont = "unifont" in header.lower()
        if "bigfont" in header.lower():
            raise ValueError("bigfont SHX (asian fonts) not supported")
        self.shapes = {}
        self.above, self.below = 1, 0
        pos = head_end
        if self.unifont:
            count, info_len = struct.unpack_from("<IH", data, pos)
            pos += 6
            info = data[pos:pos + info_len]
            pos += info_len
            name_end = info.index(b"\0")
            self.name = info[:name_end].decode("latin-1")
            if len(info) > name_end + 2:
                self.above, self.below = info[name_end + 1], info[name_end + 2]
            for _ in range(count - 1):
                if pos + 4 > len(data):
                    break
                num, ln = struct.unpack_from("<HH", data, pos)
                pos += 4
                body = data[pos:pos + ln]
                pos += ln
                nz = body.find(b"\0")
                self.shapes[num] = body[nz + 1:] if nz >= 0 else body
        else:
            first, last, count = struct.unpack_from("<HHH", data, pos)
            pos += 6
            index = [struct.unpack_from("<HH", data, pos + 4 * i) for i in range(count)]
            pos += 4 * count
            for num, ln in index:
                body = data[pos:pos + ln]
                pos += ln
                nz = body.find(b"\0")
                spec = body[nz + 1:] if nz >= 0 else body
                if num == 0:
                    self.name = body[:nz].decode("latin-1") if nz >= 0 else ""
                    if len(spec) >= 2:
                        self.above, self.below = spec[0], spec[1]
                else:
                    self.shapes[num] = spec
        self.above = self.above or 1

    # ------------------------------------------------------------ rendering
    def _cmd_len(self, spec, i):
        """Number of bytes used by the command starting at spec[i] (for the vertical-only code 14)."""
        c = spec[i]
        if c in (3, 4):
            return 2
        if c == 7:
            return 3 if self.unifont else 2
        if c == 8:
            return 3
        if c == 9:
            j = i + 1
            while j + 1 < len(spec) and not (spec[j] == 0 and spec[j + 1] == 0):
                j += 2
            return j + 2 - i
        if c == 10:
            return 3
        if c == 11:
            return 6
        if c == 12:
            return 4
        if c == 13:
            j = i + 1
            while j + 1 < len(spec) and not (spec[j] == 0 and spec[j + 1] == 0):
                j += 3
            return j + 2 - i
        return 1

    def glyph(self, code, depth=0):
        """(strokes, advance_x) in font units for one character."""
        spec = self.shapes.get(code)
        if spec is None or depth > 8:
            return [], self.above * 0.6
        strokes, cur = [], None
        x = y = 0.0
        pen = True
        scale = 1.0
        stack = []

        def move(dx, dy):
            nonlocal x, y, cur
            nx, ny = x + dx, y + dy
            if pen:
                if cur is None:
                    cur = [(x, y)]
                    strokes.append(cur)
                cur.append((nx, ny))
            else:
                cur = None
            x, y = nx, ny

        def arc(cx, cy, r, a0, a1):
            n = max(4, int(abs(a1 - a0) / (math.pi / 16)))
            for k in range(1, n + 1):
                a = a0 + (a1 - a0) * k / n
                move(cx + r * math.cos(a) - x, cy + r * math.sin(a) - y)

        def bulge_arc(dx, dy, b):
            if b == 0:
                move(dx, dy)
                return
            chord = math.hypot(dx, dy)
            theta = 4 * math.atan(b / 127.0)
            r = chord / (2 * math.sin(theta / 2))
            mx, my = x + dx / 2, y + dy / 2
            d = r * math.cos(theta / 2)
            nx, ny = -dy / chord, dx / chord
            cx, cy = mx + nx * d, my + ny * d
            a0 = math.atan2(y - cy, x - cx)
            arc(cx, cy, abs(r), a0, a0 + theta)

        i = 0
        while i < len(spec):
            c = spec[i]
            if c == 0:
                break
            if c == 1:
                pen = True
                i += 1
            elif c == 2:
                pen = False
                cur = None
                i += 1
            elif c == 3:
                scale /= max(1, spec[i + 1])
                i += 2
            elif c == 4:
                scale *= spec[i + 1]
                i += 2
            elif c == 5:
                stack.append((x, y))
                i += 1
            elif c == 6:
                if stack:
                    x, y = stack.pop()
                cur = None
                i += 1
            elif c == 7:
                if self.unifont:
                    sub = (spec[i + 1] << 8) | spec[i + 2]
                    i += 3
                else:
                    sub = spec[i + 1]
                    i += 2
                sstrokes, sadv = self.glyph(sub, depth + 1)
                for st in sstrokes:
                    strokes.append([(x + px * scale, y + py * scale) for px, py in st])
                x += sadv * scale
                cur = None
            elif c == 8:
                move(_s8(spec[i + 1]) * scale, _s8(spec[i + 2]) * scale)
                i += 3
            elif c == 9:
                i += 1
                while i + 1 < len(spec) and not (spec[i] == 0 and spec[i + 1] == 0):
                    move(_s8(spec[i]) * scale, _s8(spec[i + 1]) * scale)
                    i += 2
                i += 2
            elif c == 10:
                r = spec[i + 1] * scale
                sc = _s8(spec[i + 2])
                cw = sc < 0
                sc = abs(sc)
                start, count = (sc >> 4) & 7, sc & 7 or 8
                a0 = start * math.pi / 4
                cx, cy = x - r * math.cos(a0), y - r * math.sin(a0)
                a1 = a0 - count * math.pi / 4 if cw else a0 + count * math.pi / 4
                arc(cx, cy, r, a0, a1)
                i += 3
            elif c == 11:
                so, eo, rh, rl, sc = spec[i + 1:i + 6]
                r = (rh * 256 + rl) * scale
                sc = _s8(sc)
                cw = sc < 0
                sc = abs(sc)
                start, count = (sc >> 4) & 7, sc & 7 or 8
                a0 = start * math.pi / 4 + (so / 256) * math.pi / 4 * (-1 if cw else 1)
                span = (count - 1) * math.pi / 4 + (eo / 256 if eo else 1) * math.pi / 4 - (so / 256) * math.pi / 4
                cx, cy = x - r * math.cos(a0), y - r * math.sin(a0)
                arc(cx, cy, r, a0, a0 - span if cw else a0 + span)
                i += 6
            elif c == 12:
                bulge_arc(_s8(spec[i + 1]) * scale, _s8(spec[i + 2]) * scale, _s8(spec[i + 3]))
                i += 4
            elif c == 13:
                i += 1
                while i + 1 < len(spec) and not (spec[i] == 0 and spec[i + 1] == 0):
                    bulge_arc(_s8(spec[i]) * scale, _s8(spec[i + 1]) * scale, _s8(spec[i + 2]))
                    i += 3
                i += 2
            elif c == 14:
                i += 1
                if i < len(spec):
                    i += self._cmd_len(spec, i)      # horizontal text: skip the vertical-only command
            else:
                length, direction = c >> 4, c & 15
                dx, dy = DIRS[direction]
                move(dx * length * scale, dy * length * scale)
                i += 1
        return [s for s in strokes if len(s) >= 2], x

    def text_strokes(self, text, height):
        """Open strokes in mm for a string; baseline at y=0, Y up. Height = cap height (font 'above')."""
        k = height / self.above
        out, pen_x = [], 0.0
        for ch in text:
            strokes, adv = self.glyph(ord(ch))
            out += [[(pen_x + px * k, py * k) for px, py in st] for st in strokes]
            pen_x += adv * k
        return out, pen_x


_cache = {}


def load(path):
    f = _cache.get(path)
    if f is None:
        f = ShxFont(path)
        _cache[path] = f
    return f


def list_fonts(folder):
    if not folder or not os.path.isdir(folder):
        return []
    return sorted(os.path.join(folder, n) for n in os.listdir(folder) if n.lower().endswith(".shx"))
