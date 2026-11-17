"""3D model import (STL, OBJ, PLY, 3MF, AMF) rendered as a top-view depth map for 3D sliced engraving.

Highest surface = white (no burn), deepest = black; empty background = white.
"""

import re
import struct
import zipfile
import xml.etree.ElementTree as ET

import numpy as np
from PIL import Image


def _stl(data):
    if data[:5].lower() == b"solid" and b"facet" in data[:1024]:
        nums = re.findall(rb"vertex\s+([-+0-9.eE]+)\s+([-+0-9.eE]+)\s+([-+0-9.eE]+)", data)
        v = np.array(nums, dtype=np.float64)
        return v.reshape(-1, 3, 3)
    n = struct.unpack_from("<I", data, 80)[0]
    arr = np.frombuffer(data, dtype=np.dtype([("n", "<f4", 3), ("v", "<f4", (3, 3)), ("a", "<u2")]), count=n, offset=84)
    return arr["v"].astype(np.float64)


def _obj(text):
    verts, tris = [], []
    for line in text.splitlines():
        parts = line.split()
        if not parts:
            continue
        if parts[0] == "v" and len(parts) >= 4:
            verts.append([float(parts[1]), float(parts[2]), float(parts[3])])
        elif parts[0] == "f" and len(parts) >= 4:
            idx = [int(p.split("/")[0]) for p in parts[1:]]
            idx = [i - 1 if i > 0 else len(verts) + i for i in idx]
            for k in range(1, len(idx) - 1):          # fan triangulation
                tris.append([idx[0], idx[k], idx[k + 1]])
    v = np.array(verts, dtype=np.float64)
    return v[np.array(tris, dtype=np.int64)]


def _ply(data):
    head_end = data.index(b"end_header") + len(b"end_header")
    head_end += 2 if data[head_end:head_end + 2] == b"\r\n" else 1
    header = data[:head_end].decode("latin-1").splitlines()
    fmt = next(l.split()[1] for l in header if l.startswith("format"))
    elements, cur = [], None
    for l in header:
        p = l.split()
        if p and p[0] == "element":
            cur = {"name": p[1], "count": int(p[2]), "props": []}
            elements.append(cur)
        elif p and p[0] == "property" and cur is not None:
            cur["props"].append(p[1:])
    body = data[head_end:]
    verts, faces = None, []
    types = {"char": "b", "uchar": "B", "short": "h", "ushort": "H", "int": "i", "uint": "I", "float": "f", "double": "d",
             "int8": "b", "uint8": "B", "int16": "h", "uint16": "H", "int32": "i", "uint32": "I", "float32": "f", "float64": "d"}
    if fmt == "ascii":
        lines = body.decode("latin-1").split("\n")
        pos = 0
        for el in elements:
            rows = lines[pos:pos + el["count"]]
            pos += el["count"]
            if el["name"] == "vertex":
                names = [pr[-1] for pr in el["props"]]
                ix, iy, iz = names.index("x"), names.index("y"), names.index("z")
                verts = np.array([[float(r.split()[ix]), float(r.split()[iy]), float(r.split()[iz])] for r in rows if r.strip()])
            elif el["name"] == "face":
                for r in rows:
                    p = r.split()
                    if p:
                        n = int(p[0])
                        idx = [int(x) for x in p[1:1 + n]]
                        faces += [[idx[0], idx[k], idx[k + 1]] for k in range(1, n - 1)]
    else:
        end = "<" if "little" in fmt else ">"
        off = 0
        for el in elements:
            if el["name"] == "vertex":
                names = [pr[-1] for pr in el["props"]]
                f = end + "".join(types[pr[0]] for pr in el["props"])
                size = struct.calcsize(f)
                rows = [struct.unpack_from(f, body, off + i * size) for i in range(el["count"])]
                off += size * el["count"]
                verts = np.array([[r[names.index("x")], r[names.index("y")], r[names.index("z")]] for r in rows], dtype=np.float64)
            elif el["name"] == "face":
                lt, it = el["props"][0][1], el["props"][0][2]
                for _ in range(el["count"]):
                    n = struct.unpack_from(end + types[lt], body, off)[0]
                    off += struct.calcsize(types[lt])
                    idx = struct.unpack_from(end + types[it] * n, body, off)
                    off += struct.calcsize(types[it]) * n
                    faces += [[idx[0], idx[k], idx[k + 1]] for k in range(1, n - 1)]
            else:
                f = end + "".join(types[pr[0]] for pr in el["props"] if pr[0] != "list")
                off += struct.calcsize(f) * el["count"]
    return verts[np.array(faces, dtype=np.int64)]


def _xml_mesh(root):
    tris = []
    for mesh in root.iter():
        if not mesh.tag.endswith("mesh"):
            continue
        verts = []
        for v in mesh.iter():
            tag = v.tag.split("}")[-1]
            if tag == "vertex":
                if "x" in v.attrib:                         # 3MF
                    verts.append([float(v.get("x")), float(v.get("y")), float(v.get("z"))])
                else:                                       # AMF: <vertex><coordinates><x>..
                    c = {e.tag.split("}")[-1]: float(e.text) for e in v.iter() if e.tag.split("}")[-1] in ("x", "y", "z")}
                    if len(c) == 3:
                        verts.append([c["x"], c["y"], c["z"]])
        vv = np.array(verts, dtype=np.float64)
        for t in mesh.iter():
            tag = t.tag.split("}")[-1]
            if tag == "triangle":
                if "v1" in t.attrib:
                    ids = [int(t.get("v1")), int(t.get("v2")), int(t.get("v3"))]
                else:
                    c = {e.tag.split("}")[-1]: int(e.text) for e in t.iter() if e.tag.split("}")[-1] in ("v1", "v2", "v3")}
                    ids = [c["v1"], c["v2"], c["v3"]]
                tris.append(vv[ids])
    return np.array(tris, dtype=np.float64)


def load_mesh(path):
    """Triangles as an (N, 3, 3) array."""
    low = path.lower()
    with open(path, "rb") as f:
        data = f.read()
    if low.endswith(".stl"):
        return _stl(data)
    if low.endswith(".obj"):
        return _obj(data.decode("latin-1"))
    if low.endswith(".ply"):
        return _ply(data)
    if low.endswith(".3mf"):
        with zipfile.ZipFile(path) as z:
            name = next(n for n in z.namelist() if n.lower().endswith(".model"))
            return _xml_mesh(ET.fromstring(z.read(name)))
    if low.endswith(".amf"):
        if data[:2] == b"PK":
            with zipfile.ZipFile(path) as z:
                data = z.read(z.namelist()[0])
        return _xml_mesh(ET.fromstring(data))
    raise ValueError("formato 3D non supportato")


def depth_map(tris, px_mm=0.1, up="z", max_px=1600):
    """Rasterise the highest surface seen from above into an 8-bit image. Returns (PIL image, width_mm, height_mm)."""
    if len(tris) == 0:
        raise ValueError("modello vuoto")
    if up == "y":                       # Y-up models (common for OBJ): look down the Y axis
        tris = tris[:, :, [0, 2, 1]]
        tris[:, :, 1] *= -1
    pts = tris.reshape(-1, 3)
    mn, mx = pts.min(axis=0), pts.max(axis=0)
    size = mx - mn
    res = max(px_mm, max(size[0], size[1]) / max_px)
    W = max(1, int(np.ceil(size[0] / res)) + 1)
    H = max(1, int(np.ceil(size[1] / res)) + 1)
    zbuf = np.full((H, W), -np.inf)
    # grid coords: x to the right, model +Y up on the page (row 0 = max Y)
    gx = (tris[:, :, 0] - mn[0]) / res
    gy = (mx[1] - tris[:, :, 1]) / res
    gz = tris[:, :, 2]
    for t in range(len(tris)):
        x0, x1, x2 = gx[t]
        y0, y1, y2 = gy[t]
        den = (y1 - y2) * (x0 - x2) + (x2 - x1) * (y0 - y2)
        if abs(den) < 1e-12:
            continue
        xa, xb = int(max(0, np.floor(min(x0, x1, x2)))), int(min(W - 1, np.ceil(max(x0, x1, x2))))
        ya, yb = int(max(0, np.floor(min(y0, y1, y2)))), int(min(H - 1, np.ceil(max(y0, y1, y2))))
        if xb < xa or yb < ya:
            continue
        yy, xx = np.mgrid[ya:yb + 1, xa:xb + 1]
        px, py = xx + 0.5, yy + 0.5
        l0 = ((y1 - y2) * (px - x2) + (x2 - x1) * (py - y2)) / den
        l1 = ((y2 - y0) * (px - x2) + (x0 - x2) * (py - y2)) / den
        l2 = 1 - l0 - l1
        inside = (l0 >= -1e-9) & (l1 >= -1e-9) & (l2 >= -1e-9)
        if not inside.any():
            continue
        z = l0 * gz[t, 0] + l1 * gz[t, 1] + l2 * gz[t, 2]
        sub = zbuf[ya:yb + 1, xa:xb + 1]
        upd = inside & (z > sub)
        sub[upd] = z[upd]
    covered = np.isfinite(zbuf)
    if not covered.any():
        raise ValueError("il modello non ha superfici visibili dall'alto")
    zmin, zmax = zbuf[covered].min(), zbuf[covered].max()
    span = (zmax - zmin) or 1.0
    gray = np.full((H, W), 255.0)
    gray[covered] = (zbuf[covered] - zmin) / span * 255.0
    return Image.fromarray(gray.astype(np.uint8), "L").convert("RGB"), W * res, H * res
