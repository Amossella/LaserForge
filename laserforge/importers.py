"""Vector importers: DXF (ASCII) and PDF / Illustrator (.ai, PDF-compatible).

Both return a list of (points, closed) polylines in millimetres, with Y pointing down like the canvas.
"""

import math
import re
import zlib


# ====================================================================== DXF
_STEP = [0.3]   # arc sampling step in drawing units (set from the file's units)

DXF_UNITS = {1: 25.4, 2: 304.8, 4: 1.0, 5: 10.0, 6: 1000.0, 8: 0.0000254, 9: 0.0254, 10: 914.4}


def _dxf_pairs(text):
    lines = text.splitlines()
    for i in range(0, len(lines) - 1, 2):
        try:
            yield int(lines[i].strip()), lines[i + 1].strip()
        except ValueError:
            continue


def _bulge_arc(p0, p1, bulge, step=None):
    """Points after p0 up to p1 along a DXF bulge arc."""
    if abs(bulge) < 1e-9:
        return [p1]
    theta = 4 * math.atan(bulge)
    chord = math.hypot(p1[0] - p0[0], p1[1] - p0[1])
    if chord < 1e-12:
        return [p1]
    r = chord / (2 * math.sin(theta / 2))
    mx, my = (p0[0] + p1[0]) / 2, (p0[1] + p1[1]) / 2
    d = r * math.cos(theta / 2)
    nx, ny = -(p1[1] - p0[1]) / chord, (p1[0] - p0[0]) / chord
    cx, cy = mx + nx * d, my + ny * d
    a0 = math.atan2(p0[1] - cy, p0[0] - cx)
    n = max(2, int(abs(theta) / math.tau * 48), int(abs(theta * r) / (step or _STEP[0])))
    return [(cx + abs(r) * math.cos(a0 + theta * k / n), cy + abs(r) * math.sin(a0 + theta * k / n)) for k in range(1, n + 1)]


def _arc(cx, cy, r, a0, a1, step=None):
    if a1 <= a0:
        a1 += 360
    span = math.radians(a1 - a0)
    n = max(4, int(span / math.tau * 64), int(span * r / (step or _STEP[0])))
    return [(cx + r * math.cos(math.radians(a0) + span * k / n), cy + r * math.sin(math.radians(a0) + span * k / n)) for k in range(n + 1)]


def _bspline(ctrl, degree, knots, samples=None):
    n = len(ctrl)
    if n < 2:
        return ctrl
    degree = max(1, min(degree, n - 1))
    if not knots or len(knots) != n + degree + 1:
        knots = [0] * (degree + 1) + list(range(1, n - degree)) + [n - degree] * (degree + 1)
    lo, hi = knots[degree], knots[n]
    samples = samples or max(20, n * 12)

    def deboor(t):
        k = degree
        while k < n - 1 and t >= knots[k + 1]:
            k += 1
        d = [list(ctrl[j + k - degree]) for j in range(degree + 1)]
        for r in range(1, degree + 1):
            for j in range(degree, r - 1, -1):
                i = j + k - degree
                den = knots[i + degree - r + 1] - knots[i]
                a = (t - knots[i]) / den if den else 0
                d[j] = [(1 - a) * d[j - 1][0] + a * d[j][0], (1 - a) * d[j - 1][1] + a * d[j][1]]
        return (d[degree][0], d[degree][1])
    return [deboor(lo + (hi - lo) * s / samples) for s in range(samples + 1)]


def _parse_entities(pairs):
    """Group a flat pair list into entities [(type, [(code, value), ...])]."""
    ents, cur = [], None
    for code, val in pairs:
        if code == 0:
            if cur:
                ents.append(cur)
            cur = (val, [])
        elif cur is not None:
            cur[1].append((code, val))
    if cur:
        ents.append(cur)
    return ents


def _entity_paths(etype, data, ents_iter=None):
    get = lambda c, d=0.0: next((float(v) for k, v in data if k == c), d)
    out = []
    if etype == "LINE":
        out.append(([(get(10), get(20)), (get(11), get(21))], False))
    elif etype == "CIRCLE":
        pts = _arc(get(10), get(20), get(40), 0, 360)
        out.append((pts, True))
    elif etype == "ARC":
        out.append((_arc(get(10), get(20), get(40), get(50), get(51)), False))
    elif etype == "ELLIPSE":
        cx, cy, mx, my, ratio = get(10), get(20), get(11), get(21), get(40, 1.0)
        t0, t1 = get(41, 0.0), get(42, math.tau)
        if t1 <= t0:
            t1 += math.tau
        a = math.hypot(mx, my)
        rot = math.atan2(my, mx)
        n = max(48, int((t1 - t0) * a / _STEP[0]))
        pts = []
        for k in range(n + 1):
            t = t0 + (t1 - t0) * k / n
            x, y = a * math.cos(t), a * ratio * math.sin(t)
            pts.append((cx + x * math.cos(rot) - y * math.sin(rot), cy + x * math.sin(rot) + y * math.cos(rot)))
        out.append((pts, abs(t1 - t0 - math.tau) < 1e-6))
    elif etype == "LWPOLYLINE":
        closed = int(get(70, 0)) & 1
        verts, bulges = [], []
        x = None
        for k, v in data:
            if k == 10:
                x = float(v)
            elif k == 20 and x is not None:
                verts.append((x, float(v)))
                bulges.append(0.0)
            elif k == 42 and bulges:
                bulges[-1] = float(v)
        if len(verts) >= 2:
            pts = [verts[0]]
            n = len(verts)
            for i in range(n - 1 + (1 if closed else 0)):
                pts += _bulge_arc(verts[i], verts[(i + 1) % n], bulges[i])
            out.append((pts, bool(closed)))
    elif etype == "SPLINE":
        degree = int(get(71, 3))
        closed = int(get(70, 0)) & 1
        ctrl, fit, knots = [], [], []
        cx = fx = None
        for k, v in data:
            if k == 40:
                knots.append(float(v))
            elif k == 10:
                cx = float(v)
            elif k == 20 and cx is not None:
                ctrl.append((cx, float(v)))
            elif k == 11:
                fx = float(v)
            elif k == 21 and fx is not None:
                fit.append((fx, float(v)))
        if ctrl:
            out.append((_bspline(ctrl, degree, knots), bool(closed)))
        elif len(fit) >= 2:
            out.append((fit, bool(closed)))
    return out


def dxf_to_paths(text):
    pairs = list(_dxf_pairs(text))
    # units from the header
    scale = 1.0
    for i, (code, val) in enumerate(pairs):
        if code == 9 and val == "$INSUNITS" and i + 1 < len(pairs):
            scale = DXF_UNITS.get(int(pairs[i + 1][1] or 0), 1.0)
            break
    _STEP[0] = 0.3 / scale
    # split sections
    sections, cur, name = {}, None, None
    for i, (code, val) in enumerate(pairs):
        if code == 0 and val == "SECTION":
            cur, name = [], None
        elif code == 2 and cur is not None and name is None:
            name = val
        elif code == 0 and val == "ENDSEC":
            if name:
                sections[name] = cur
            cur = name = None
        elif cur is not None and name is not None:
            cur.append((code, val))
    blocks = {}
    bents = _parse_entities(sections.get("BLOCKS", []))
    bname, bbase, blist = None, (0.0, 0.0), None
    for etype, data in bents:
        if etype == "BLOCK":
            bname = next((v for k, v in data if k == 2), None)
            bbase = (next((float(v) for k, v in data if k == 10), 0.0), next((float(v) for k, v in data if k == 20), 0.0))
            blist = []
        elif etype == "ENDBLK":
            if bname is not None:
                blocks[bname] = (bbase, blist)
            bname, blist = None, None
        elif blist is not None:
            blist.append((etype, data))

    paths = []

    def emit(ents, tf, depth=0):
        polyline = None
        for etype, data in ents:
            if etype == "POLYLINE":
                polyline = {"closed": int(next((v for k, v in data if k == 70), 0)) & 1, "verts": [], "bulges": []}
                continue
            if etype == "VERTEX" and polyline is not None:
                polyline["verts"].append((next((float(v) for k, v in data if k == 10), 0.0), next((float(v) for k, v in data if k == 20), 0.0)))
                polyline["bulges"].append(next((float(v) for k, v in data if k == 42), 0.0))
                continue
            if etype == "SEQEND" and polyline is not None:
                v, b = polyline["verts"], polyline["bulges"]
                if len(v) >= 2:
                    pts = [v[0]]
                    for i in range(len(v) - 1 + polyline["closed"]):
                        pts += _bulge_arc(v[i], v[(i + 1) % len(v)], b[i])
                    paths.append(([tf(p) for p in pts], bool(polyline["closed"])))
                polyline = None
                continue
            if etype == "INSERT" and depth < 8:
                name = next((v for k, v in data if k == 2), None)
                if name in blocks:
                    (bx, by), bl = blocks[name]
                    ix = next((float(v) for k, v in data if k == 10), 0.0)
                    iy = next((float(v) for k, v in data if k == 20), 0.0)
                    sx = next((float(v) for k, v in data if k == 41), 1.0)
                    sy = next((float(v) for k, v in data if k == 42), sx)
                    rot = math.radians(next((float(v) for k, v in data if k == 50), 0.0))
                    c, s = math.cos(rot), math.sin(rot)

                    def sub(p, ix=ix, iy=iy, sx=sx, sy=sy, c=c, s=s, bx=bx, by=by, outer=tf):
                        x, y = (p[0] - bx) * sx, (p[1] - by) * sy
                        return outer((ix + x * c - y * s, iy + x * s + y * c))
                    emit(bl, sub, depth + 1)
                continue
            for pts, closed in _entity_paths(etype, data):
                paths.append(([tf(p) for p in pts], closed))

    emit(_parse_entities(sections.get("ENTITIES", [])), lambda p: p)
    if not paths:
        return []
    maxy = max(y for pts, _ in paths for _, y in pts)
    minx = min(x for pts, _ in paths for x, _ in pts)
    # DXF is Y-up: flip, convert units and move to the top-left corner
    return [([((x - minx) * scale, (maxy - y) * scale) for x, y in pts], closed) for pts, closed in paths]


# ====================================================================== PDF / AI
_OBJ = re.compile(rb"(\d+)\s+(\d+)\s+obj\b")
_REF = re.compile(rb"(\d+)\s+(\d+)\s+R")


_RAW = {}   # object number -> undecoded stream bytes (needed for JPEG images)


def _pdf_objects(data):
    """{num: (dict_bytes, stream_bytes_or_None)} including objects packed in /ObjStm streams."""
    objs = {}
    _RAW.clear()
    for m in _OBJ.finditer(data):
        num = int(m.group(1))
        start = m.end()
        end = data.find(b"endobj", start)
        if end < 0:
            continue
        body = data[start:end]
        stream = None
        si = body.find(b"stream")
        if si >= 0:
            raw = body[si + 6:]
            raw = raw[2:] if raw.startswith(b"\r\n") else raw[1:] if raw[:1] in (b"\n", b"\r") else raw
            ei = raw.rfind(b"endstream")
            raw = raw[:ei] if ei >= 0 else raw
            head = body[:si]
            _RAW[num] = raw
            stream = _decode_stream(head, raw)
            body = head
        objs[num] = (body, stream)
    # object streams
    for num, (head, stream) in list(objs.items()):
        if stream is None or b"/ObjStm" not in head:
            continue
        n = int(re.search(rb"/N\s+(\d+)", head).group(1))
        first = int(re.search(rb"/First\s+(\d+)", head).group(1))
        nums = stream[:first].split()
        pairs = [(int(nums[2 * i]), int(nums[2 * i + 1])) for i in range(min(n, len(nums) // 2))]
        for i, (onum, off) in enumerate(pairs):
            end = pairs[i + 1][1] if i + 1 < len(pairs) else len(stream) - first
            if onum not in objs:
                objs[onum] = (stream[first + off:first + end], None)
    return objs


def _decode_stream(head, raw):
    if b"/FlateDecode" in head or b"/Fl " in head or head.rstrip().endswith(b"/Fl"):
        try:
            return zlib.decompress(raw)
        except zlib.error:
            try:
                return zlib.decompressobj().decompress(raw)
            except zlib.error:
                return b""
    if b"/Filter" in head:
        return b""      # unsupported filter (DCT images, LZW, ...)
    return raw


def _refs(body, key):
    m = re.search(rb"/" + key + rb"\s*(\[[^\]]*\]|\d+\s+\d+\s+R)", body)
    if not m:
        return []
    return [int(r.group(1)) for r in _REF.finditer(m.group(1))]


def _pdf_image(num, objs):
    """Decode an image XObject to a PIL image (JPEG, or Flate-compressed RGB / grey / CMYK 8-bit)."""
    import io
    from PIL import Image
    head, stream = objs[num]
    raw = _RAW.get(num, b"")
    try:
        if b"/DCTDecode" in head:
            return Image.open(io.BytesIO(raw)).convert("RGB")
        if b"/JPXDecode" in head:
            return Image.open(io.BytesIO(raw)).convert("RGB")
        w = int(re.search(rb"/Width\s+(\d+)", head).group(1))
        h = int(re.search(rb"/Height\s+(\d+)", head).group(1))
        bpc = int((re.search(rb"/BitsPerComponent\s+(\d+)", head) or re.search(rb"(8)", b"8")).group(1))
        if not stream or bpc != 8:
            return None
        if b"/DeviceRGB" in head and len(stream) >= w * h * 3:
            return Image.frombytes("RGB", (w, h), stream[:w * h * 3])
        if b"/DeviceCMYK" in head and len(stream) >= w * h * 4:
            return Image.frombytes("CMYK", (w, h), stream[:w * h * 4]).convert("RGB")
        if len(stream) >= w * h:
            return Image.frombytes("L", (w, h), stream[:w * h]).convert("RGB")
    except Exception:
        return None
    return None


def _content_paths(content, ctm, objs, resources, depth=0, images=None):
    """Run the path operators of a content stream. Returns [(points, closed)] in PDF user space.
    Placed images are appended to `images` as (PIL image, ctm)."""
    tokens = re.findall(rb"/[^\s/\[\]()<>]+|\((?:\\.|[^\\)])*\)|<[0-9A-Fa-f\s]*>|\[|\]|[^\s\[\]()<>/]+", content)
    stack, out = [], []
    sub, path = [], []
    gstack = []
    cur = (0.0, 0.0)
    start = (0.0, 0.0)

    def T(x, y, m=None):
        a, b, c, d, e, f = m or ctm
        return (a * x + c * y + e, b * x + d * y + f)

    def flush_sub(closed=False):
        nonlocal sub
        if len(sub) >= 2:
            path.append((sub, closed))
        sub = []

    def cubic(p1, p2, p3):
        nonlocal cur
        p0 = cur
        for k in range(1, 13):
            t = k / 12
            u = 1 - t
            x = u ** 3 * p0[0] + 3 * u * u * t * p1[0] + 3 * u * t * t * p2[0] + t ** 3 * p3[0]
            y = u ** 3 * p0[1] + 3 * u * u * t * p1[1] + 3 * u * t * t * p2[1] + t ** 3 * p3[1]
            sub.append(T(x, y))
        cur = p3

    for tok in tokens:
        try:
            stack.append(float(tok))
            continue
        except ValueError:
            pass
        op = tok.decode("latin-1")
        if op in ("[", "]") or op.startswith(("(", "<", "/")):
            stack.append(op)
            continue
        nums = [v for v in stack if isinstance(v, float)]
        try:
            if op == "q":
                gstack.append(ctm)
            elif op == "Q":
                ctm = gstack.pop() if gstack else ctm
            elif op == "cm" and len(nums) >= 6:
                a, b, c, d, e, f = nums[-6:]
                A, B, C, D, E, F = ctm
                ctm = (a * A + b * C, a * B + b * D, c * A + d * C, c * B + d * D, e * A + f * C + E, e * B + f * D + F)
            elif op == "m" and len(nums) >= 2:
                flush_sub()
                cur = start = (nums[-2], nums[-1])
                sub = [T(*cur)]
            elif op == "l" and len(nums) >= 2:
                cur = (nums[-2], nums[-1])
                if not sub:
                    sub = [T(*start)]
                sub.append(T(*cur))
            elif op == "c" and len(nums) >= 6:
                cubic((nums[-6], nums[-5]), (nums[-4], nums[-3]), (nums[-2], nums[-1]))
            elif op == "v" and len(nums) >= 4:
                cubic(cur, (nums[-4], nums[-3]), (nums[-2], nums[-1]))
            elif op == "y" and len(nums) >= 4:
                cubic((nums[-4], nums[-3]), (nums[-2], nums[-1]), (nums[-2], nums[-1]))
            elif op == "h":
                if sub:
                    sub.append(T(*start))
                flush_sub(True)
                cur = start
            elif op == "re" and len(nums) >= 4:
                flush_sub()
                x, y, w, h = nums[-4:]
                path.append(([T(x, y), T(x + w, y), T(x + w, y + h), T(x, y + h), T(x, y)], True))
                cur = start = (x, y)
            elif op in ("S", "s", "f", "F", "f*", "B", "B*", "b", "b*"):
                flush_sub(op in ("s", "b", "b*"))
                out += path
                path = []
            elif op == "n":
                sub, path = [], []
            elif op == "Do" and depth < 6:
                name = next((v for v in reversed(stack) if isinstance(v, str) and v.startswith("/")), None)
                ref = resources.get(name[1:].encode()) if name else None
                if ref and ref in objs:
                    head, stream = objs[ref]
                    if b"/Image" in head and images is not None:
                        im = _pdf_image(ref, objs)
                        if im is not None:
                            images.append((im, ctm))
                    elif stream and b"/Form" in head:
                        m = re.search(rb"/Matrix\s*\[([^\]]*)\]", head)
                        fm = tuple(float(v) for v in m.group(1).split()) if m else (1, 0, 0, 1, 0, 0)
                        a, b, c, d, e, f = fm
                        A, B, C, D, E, F = ctm
                        fctm = (a * A + b * C, a * B + b * D, c * A + d * C, c * B + d * D, e * A + f * C + E, e * B + f * D + F)
                        out += _content_paths(stream, fctm, objs, {**resources, **_xobjects(head, objs)}, depth + 1, images)
        except (IndexError, ValueError):
            pass
        stack = []
    return out


def _xobjects(body, objs):
    m = re.search(rb"/XObject\s*<<(.*?)>>", body, re.S)
    if not m:
        r = re.search(rb"/Resources\s+(\d+)\s+\d+\s+R", body)
        if r and int(r.group(1)) in objs:
            return _xobjects(objs[int(r.group(1))][0], objs)
        x = re.search(rb"/XObject\s+(\d+)\s+\d+\s+R", body)
        if x and int(x.group(1)) in objs:
            return _xobjects(b"/XObject <<" + objs[int(x.group(1))][0] + b">>", objs)
        return {}
    return {k: int(v) for k, v in re.findall(rb"/([^\s/<>]+)\s+(\d+)\s+\d+\s+R", m.group(1))}


def _page_list(objs):
    """Pages in document order, following the /Kids of the page tree."""
    root = next((n for n, (b, _) in objs.items() if re.search(rb"/Type\s*/Pages\b", b) and b"/Parent" not in b), None)
    out = []

    def walk(n, depth=0):
        if n not in objs or depth > 32:
            return
        b = objs[n][0]
        if re.search(rb"/Type\s*/Pages\b", b):
            for k in _refs(b, b"Kids"):
                walk(k, depth + 1)
        elif re.search(rb"/Type\s*/Page\b", b):
            out.append(n)
    if root is not None:
        walk(root)
    if not out:
        out = sorted(n for n, (b, _) in objs.items() if re.search(rb"/Type\s*/Page\b(?!s)", b))
    return out


def pdf_page_count(data):
    if b"%PDF" not in data[:1024]:
        return 0
    return len(_page_list(_pdf_objects(data)))


def pdf_to_paths(data, page_index=0, images=None):
    """Vector paths of one page in mm (Y down). Embedded images go to `images` as (PIL image, x, y, w, h) in mm."""
    if not data.startswith(b"%PDF") and b"%PDF" not in data[:1024]:
        raise ValueError("not a PDF (older PostScript .ai files are not supported)")
    objs = _pdf_objects(data)
    pages = _page_list(objs)
    if not pages:
        return []
    body = objs[pages[min(page_index, len(pages) - 1)]][0]
    mb = re.search(rb"/(?:CropBox|MediaBox)\s*\[([^\]]*)\]", body)
    box = [float(v) for v in mb.group(1).split()] if mb else [0, 0, 612, 792]
    content = b""
    for ref in _refs(body, b"Contents"):
        if ref in objs and objs[ref][1] is not None:
            content += objs[ref][1] + b"\n"
        elif ref in objs:
            for r2 in _REF.finditer(objs[ref][0]):
                if int(r2.group(1)) in objs and objs[int(r2.group(1))][1] is not None:
                    content += objs[int(r2.group(1))][1] + b"\n"
    placed = []
    paths = _content_paths(content, (1, 0, 0, 1, 0, 0), objs, _xobjects(body, objs), 0, placed)
    k = 25.4 / 72
    top = box[3]
    if images is not None:
        from PIL import ImageOps
        for im, (a, b_, c_, d_, e, f) in placed:
            # unit square mapped by the CTM (axis-aligned placements; flips handled)
            xs = [e, e + a, e + c_, e + a + c_]
            ys = [f, f + b_, f + d_, f + b_ + d_]
            if a < 0:
                im = ImageOps.mirror(im)
            if d_ < 0:          # image row 0 maps to the unit square's top edge; a negative scale flips it
                im = ImageOps.flip(im)
            x0, x1, y0, y1 = min(xs), max(xs), min(ys), max(ys)
            images.append((im, (x0 - box[0]) * k, (top - y1) * k, (x1 - x0) * k, (y1 - y0) * k))
    from shapely.geometry import LineString
    out, kept = [], []
    for pts, closed in paths:
        if len(pts) < 2:
            continue
        mm = [((x - box[0]) * k, (top - y) * k) for x, y in pts]
        xs, ys = [p[0] for p in mm], [p[1] for p in mm]
        bb = (min(xs), min(ys), max(xs), max(ys))
        line = LineString(mm)
        # the same outline is often painted twice (fill + stroke), or a hairline stroke is saved as a thin
        # outlined area (two rings a hair apart): keep a single ring
        dup = False
        for kb, kl in kept:
            if all(abs(a - b) < 0.15 for a, b in zip(bb, kb)) and line.hausdorff_distance(kl) < 0.15:
                dup = True
                break
        if dup:
            continue
        kept.append((bb, line))
        out.append((mm, closed))
    return out
